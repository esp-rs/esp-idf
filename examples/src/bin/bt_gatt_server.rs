//! Example of a BLE GATT server using the ESP IDF Bluedroid BLE bindings.
//!
//! You can test it with the bt_gatt_client example or any "GATT Browser" app, like e.g.
//! the "GATTBrowser" mobile app available on Android.
//!
//! The example server publishes a single service featuring two characteristics:
//! - A "recv" characteristic that clients can write to
//! - An "indicate" characteristic that clients can subscribe to and receive indications from
//!
//! The example is relatively sophisticated as it demonstrates not only how to receive data from clients
//! but also how to broadcast data to all clients that have subscribed to a characteristic, including
//! handling indication confirmations.
//!
//! Note that the Bluedroid stack consumes a lot of memory, so `sdkconfig.defaults` should be carefully configured
//! to avoid running out of memory.
//!
//! Here's a working configuration, but you might need to adjust further to your concrete use-case:
//!
//! CONFIG_BT_ENABLED=y
//! CONFIG_BT_BLUEDROID_ENABLED=y
//! CONFIG_BT_CLASSIC_ENABLED=n
//! CONFIG_BTDM_CTRL_MODE_BLE_ONLY=y
//! CONFIG_BTDM_CTRL_MODE_BR_EDR_ONLY=n
//! CONFIG_BTDM_CTRL_MODE_BTDM=n
//! CONFIG_BT_BLE_42_FEATURES_SUPPORTED=y
//! CONFIG_BT_BLE_50_FEATURES_SUPPORTED=n
//! CONFIG_BT_BTC_TASK_STACK_SIZE=15000
//! CONFIG_BT_BLE_DYNAMIC_ENV_MEMORY=y

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(not(any(esp32s2, esp32p4)), esp_idf_bt_bluedroid_enabled))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(not(all(not(any(esp32s2, esp32p4)), esp_idf_bt_bluedroid_enabled)))]
fn main() -> anyhow::Result<()> {
    panic!("This example requires a Bluedroid-enabled build (CONFIG_BT_BLUEDROID_ENABLED=y) on a chip with a BLE radio");
}

#[cfg(all(not(any(esp32s2, esp32p4)), esp_idf_bt_bluedroid_enabled))]
mod example {
    use std::sync::{Arc, Condvar, Mutex};

    use enumset::enum_set;

    use esp_idf_svc::bt::ble::gap::{AdvConfiguration, BleGapEvent, EspBleGap};
    use esp_idf_svc::bt::ble::gatt::server::{ConnectionId, EspGatts, GattsEvent, TransferId};
    use esp_idf_svc::bt::ble::gatt::{
        AutoResponse, GattCharacteristic, GattDescriptor, GattId, GattInterface, GattResponse,
        GattServiceId, GattStatus, Handle, Permission, Property,
    };
    use esp_idf_svc::bt::{BdAddr, Ble, BtDriver, BtStatus, BtUuid};
    use esp_idf_svc::hal::delay::FreeRtos;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::nvs::EspDefaultNvsPartition;
    use esp_idf_svc::sys::{EspError, ESP_FAIL};

    use log::{info, warn};

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let bt = Arc::new(BtDriver::new(peripherals.modem, Some(nvs.clone()))?);

        let server = ExampleServer::new(
            Arc::new(EspBleGap::new(bt.clone())?),
            Arc::new(EspGatts::new(bt.clone())?),
        );

        info!("BLE Gap and Gatts initialized");

        let gap_server = server.clone();

        server.gap.subscribe(move |event| {
            gap_server.check_esp_status(gap_server.on_gap_event(event));
        })?;

        let gatts_server = server.clone();

        server.gatts.subscribe(move |(gatt_if, event)| {
            gatts_server.check_esp_status(gatts_server.on_gatts_event(gatt_if, event))
        })?;

        info!("BLE Gap and Gatts subscriptions initialized");

        server.gatts.register_app(APP_ID)?;

        info!("Gatts BTP app registered");

        let mut ind_data = 0_u16;

        loop {
            server.indicate(&ind_data.to_le_bytes())?;
            info!("Broadcasted indication: {ind_data}");

            ind_data = ind_data.wrapping_add(1);

            FreeRtos::delay_ms(10000);
        }
    }

    const APP_ID: u16 = 0;
    const MAX_CONNECTIONS: usize = 2;

    // Our service UUID
    pub const SERVICE_UUID: u128 = 0xad91b201734740479e173bed82d75f9d;

    /// Our "recv" characteristic - i.e. where clients can send data.
    pub const RECV_CHARACTERISTIC_UUID: u128 = 0xb6fccb5087be44f3ae22f85485ea42c4;
    /// Our "indicate" characteristic - i.e. where clients can receive data if they subscribe to it
    pub const IND_CHARACTERISTIC_UUID: u128 = 0x503de214868246c4828fd59144da41be;

    // Name the types as they are used in the example to get shorter type signatures in the various functions below.
    // note that - rather than `Arc`s, you can use regular references as well, but then you have to deal with lifetimes
    // and the signatures below will not be `'static`.
    type ExBtDriver = BtDriver<'static, Ble>;
    type ExEspBleGap = Arc<EspBleGap<'static, Ble, Arc<ExBtDriver>>>;
    type ExEspGatts = Arc<EspGatts<'static, Ble, Arc<ExBtDriver>>>;

    #[derive(Debug, Clone)]
    struct Connection {
        peer: BdAddr,
        conn_id: Handle,
        subscribed: bool,
        mtu: Option<u16>,
    }

    #[derive(Default)]
    struct State {
        gatt_if: Option<GattInterface>,
        service_handle: Option<Handle>,
        recv_handle: Option<Handle>,
        ind_handle: Option<Handle>,
        ind_cccd_handle: Option<Handle>,
        connections: heapless::Vec<Connection, MAX_CONNECTIONS>,
        response: GattResponse,
        ind_confirmed: Option<BdAddr>,
    }

    #[derive(Clone)]
    pub struct ExampleServer {
        gap: ExEspBleGap,
        gatts: ExEspGatts,
        state: Arc<Mutex<State>>,
        condvar: Arc<Condvar>,
    }

    impl ExampleServer {
        pub fn new(gap: ExEspBleGap, gatts: ExEspGatts) -> Self {
            Self {
                gap,
                gatts,
                state: Arc::new(Mutex::new(Default::default())),
                condvar: Arc::new(Condvar::new()),
            }
        }
    }

    impl ExampleServer {
        /// Send (indicate) data to all peers that are currently
        /// subscribed to our indication characteristic
        ///
        /// Uses a Mutex + Condvar to wait until indication confirmation
        /// is received.
        ///
        /// This complexity is necessary only when using indications.
        /// Notifications do not really send confirmation and thus do not
        /// need this synchronization.
        fn indicate(&self, data: &[u8]) -> Result<(), EspError> {
            for peer_index in 0..MAX_CONNECTIONS {
                // Propagate this data to all clients which are connected
                // and have subscribed to our indication characteristic

                let mut state = self.state.lock().unwrap();

                loop {
                    if state.connections.len() <= peer_index {
                        // We've send to everybody who is connected
                        break;
                    }

                    let Some(gatt_if) = state.gatt_if else {
                        // We lost the gatt interface in the meantime
                        break;
                    };

                    let Some(ind_handle) = state.ind_handle else {
                        // We lost the indication handle in the meantime
                        break;
                    };

                    if state.ind_confirmed.is_none() {
                        let conn = &state.connections[peer_index];

                        if conn.subscribed {
                            self.gatts
                                .indicate(gatt_if, conn.conn_id, ind_handle, data)?;

                            state.ind_confirmed = Some(conn.peer);
                            let conn = &state.connections[peer_index];

                            info!("Indicated data to {}", conn.peer);
                        }
                        break;
                    } else {
                        state = self.condvar.wait(state).unwrap();
                    }
                }
            }

            Ok(())
        }

        /// Sample callback where the user code can handle a newly-subscribed client
        ///
        /// If the user code just broadcasts the _same_ indication to all subscribed
        /// clients, this callback might not be necessary.
        fn on_subscribed(&self, addr: BdAddr) {
            // Put your custom code here or leave empty
            // `indicate()` will anyway send to all connected clients
            warn!("Client {addr} subscribed - put your custom logic here");
        }

        /// Sample callback where the user code can handle an unsubscribed client
        ///
        /// If the user code just broadcasts the _same_ indication to all subscribed
        /// clients, this callback might not be necessary.
        fn on_unsubscribed(&self, addr: BdAddr) {
            // Put your custom code here
            // `indicate()` will anyway send to all connected clients
            warn!("Client {addr} unsubscribed - put your custom logic here");
        }

        /// Sample callback where the user code can handle received data
        /// For demo purposes, the data is just logged.
        fn on_recv(&self, addr: BdAddr, data: &[u8], offset: u16, mtu: Option<u16>) {
            // Put your custom code here
            warn!("Received data from {addr}: {data:?}, offset: {offset}, mtu: {mtu:?} - put your custom logic here");
        }

        /// The main event handler for the GAP events
        fn on_gap_event(&self, event: BleGapEvent) -> Result<(), EspError> {
            info!("Got event: {event:?}");

            if let BleGapEvent::AdvertisingConfigured(status) = event {
                self.check_bt_status(status)?;
                self.gap.start_advertising()?;
            }

            Ok(())
        }

        /// The main event handler for the GATTS events
        fn on_gatts_event(
            &self,
            gatt_if: GattInterface,
            event: GattsEvent,
        ) -> Result<(), EspError> {
            info!("Got event: {event:?}");

            match event {
                GattsEvent::ServiceRegistered { status, app_id } => {
                    self.check_gatt_status(status)?;
                    if APP_ID == app_id {
                        self.create_service(gatt_if)?;
                    }
                }
                GattsEvent::ServiceCreated {
                    status,
                    service_handle,
                    ..
                } => {
                    self.check_gatt_status(status)?;
                    self.configure_and_start_service(service_handle)?;
                }
                GattsEvent::CharacteristicAdded {
                    status,
                    attr_handle,
                    service_handle,
                    char_uuid,
                } => {
                    self.check_gatt_status(status)?;
                    self.register_characteristic(service_handle, attr_handle, char_uuid)?;
                }
                GattsEvent::DescriptorAdded {
                    status,
                    attr_handle,
                    service_handle,
                    descr_uuid,
                } => {
                    self.check_gatt_status(status)?;
                    self.register_cccd_descriptor(service_handle, attr_handle, descr_uuid)?;
                }
                GattsEvent::ServiceDeleted {
                    status,
                    service_handle,
                } => {
                    self.check_gatt_status(status)?;
                    self.delete_service(service_handle)?;
                }
                GattsEvent::ServiceUnregistered {
                    status,
                    service_handle,
                    ..
                } => {
                    self.check_gatt_status(status)?;
                    self.unregister_service(service_handle)?;
                }
                GattsEvent::Mtu { conn_id, mtu } => {
                    self.register_conn_mtu(conn_id, mtu)?;
                }
                GattsEvent::PeerConnected { conn_id, addr, .. } => {
                    self.create_conn(conn_id, addr)?;
                }
                GattsEvent::PeerDisconnected { addr, .. } => {
                    self.delete_conn(addr)?;
                }
                GattsEvent::Write {
                    conn_id,
                    trans_id,
                    addr,
                    handle,
                    offset,
                    need_rsp,
                    is_prep,
                    value,
                } => {
                    let handled = self.recv(
                        gatt_if, conn_id, trans_id, addr, handle, offset, need_rsp, is_prep, value,
                    )?;

                    if handled {
                        self.send_write_response(
                            gatt_if, conn_id, trans_id, handle, offset, need_rsp, is_prep, value,
                        )?;
                    }
                }
                GattsEvent::Confirm { status, .. } => {
                    self.check_gatt_status(status)?;
                    self.confirm_indication()?;
                }
                _ => (),
            }

            Ok(())
        }

        /// Set the advertising configuration, effectively starting advertising
        fn set_adv_conf(&self) -> Result<(), EspError> {
            self.gap.set_adv_conf(&AdvConfiguration {
                include_name: true,
                include_txpower: true,
                flag: 2,
                service_uuid: Some(BtUuid::uuid128(SERVICE_UUID)),
                // service_data: todo!(),
                // manufacturer_data: todo!(),
                ..Default::default()
            })
        }

        /// Create the service and start advertising
        /// Called from within the event callback once we are notified that the GATTS app is registered
        fn create_service(&self, gatt_if: GattInterface) -> Result<(), EspError> {
            self.state.lock().unwrap().gatt_if = Some(gatt_if);

            self.gap.set_device_name("ESP32")?;
            self.set_adv_conf()?;
            self.gatts.create_service(
                gatt_if,
                &GattServiceId {
                    id: GattId {
                        uuid: BtUuid::uuid128(SERVICE_UUID),
                        inst_id: 0,
                    },
                    is_primary: true,
                },
                8,
            )?;

            Ok(())
        }

        /// Delete the service
        /// Called from within the event callback once we are notified that the GATTS app is deleted
        fn delete_service(&self, service_handle: Handle) -> Result<(), EspError> {
            let mut state = self.state.lock().unwrap();

            if state.service_handle == Some(service_handle) {
                state.recv_handle = None;
                state.ind_handle = None;
                state.ind_cccd_handle = None;
            }

            Ok(())
        }

        /// Unregister the service
        /// Called from within the event callback once we are notified that the GATTS app is unregistered
        fn unregister_service(&self, service_handle: Handle) -> Result<(), EspError> {
            let mut state = self.state.lock().unwrap();

            if state.service_handle == Some(service_handle) {
                state.gatt_if = None;
                state.service_handle = None;
            }

            Ok(())
        }

        /// Configure and start the service
        /// Called from within the event callback once we are notified that the service is created
        fn configure_and_start_service(&self, service_handle: Handle) -> Result<(), EspError> {
            self.state.lock().unwrap().service_handle = Some(service_handle);

            self.gatts.start_service(service_handle)?;
            self.add_characteristics(service_handle)?;

            Ok(())
        }

        /// Add our two characteristics to the service
        /// Called from within the event callback once we are notified that the service is created
        fn add_characteristics(&self, service_handle: Handle) -> Result<(), EspError> {
            self.gatts.add_characteristic(
                service_handle,
                &GattCharacteristic {
                    uuid: BtUuid::uuid128(RECV_CHARACTERISTIC_UUID),
                    permissions: enum_set!(Permission::Write),
                    properties: enum_set!(Property::Write),
                    max_len: 200, // Max recv data
                    auto_rsp: AutoResponse::ByApp,
                },
                &[],
            )?;

            self.gatts.add_characteristic(
                service_handle,
                &GattCharacteristic {
                    uuid: BtUuid::uuid128(IND_CHARACTERISTIC_UUID),
                    permissions: enum_set!(Permission::Write | Permission::Read),
                    properties: enum_set!(Property::Indicate),
                    max_len: 200, // Mac iondicate data
                    auto_rsp: AutoResponse::ByApp,
                },
                &[],
            )?;

            Ok(())
        }

        /// Add the CCCD descriptor
        /// Called from within the event callback once we are notified that a char descriptor is added,
        /// however the method will do something only if the added char is the "indicate" characteristics of course
        fn register_characteristic(
            &self,
            service_handle: Handle,
            attr_handle: Handle,
            char_uuid: BtUuid,
        ) -> Result<(), EspError> {
            let indicate_char = {
                let mut state = self.state.lock().unwrap();

                if state.service_handle != Some(service_handle) {
                    false
                } else if char_uuid == BtUuid::uuid128(RECV_CHARACTERISTIC_UUID) {
                    state.recv_handle = Some(attr_handle);

                    false
                } else if char_uuid == BtUuid::uuid128(IND_CHARACTERISTIC_UUID) {
                    state.ind_handle = Some(attr_handle);

                    true
                } else {
                    false
                }
            };

            if indicate_char {
                self.gatts.add_descriptor(
                    service_handle,
                    &GattDescriptor {
                        uuid: BtUuid::uuid16(0x2902), // CCCD
                        permissions: enum_set!(Permission::Read | Permission::Write),
                    },
                )?;
            }

            Ok(())
        }

        /// Register the CCCD descriptor
        /// Called from within the event callback once we are notified that a descriptor is added,
        fn register_cccd_descriptor(
            &self,
            service_handle: Handle,
            attr_handle: Handle,
            descr_uuid: BtUuid,
        ) -> Result<(), EspError> {
            let mut state = self.state.lock().unwrap();

            if descr_uuid == BtUuid::uuid16(0x2902) // CCCD UUID
                && state.service_handle == Some(service_handle)
            {
                state.ind_cccd_handle = Some(attr_handle);
            }

            Ok(())
        }

        /// Receive data from a client
        /// Called from within the event callback once we are notified for the connection MTU
        fn register_conn_mtu(&self, conn_id: ConnectionId, mtu: u16) -> Result<(), EspError> {
            let mut state = self.state.lock().unwrap();

            if let Some(conn) = state
                .connections
                .iter_mut()
                .find(|conn| conn.conn_id == conn_id)
            {
                conn.mtu = Some(mtu);
            }

            Ok(())
        }

        /// Create a new connection
        /// Called from within the event callback once we are notified for a new connection
        fn create_conn(&self, conn_id: ConnectionId, addr: BdAddr) -> Result<(), EspError> {
            let added = {
                let mut state = self.state.lock().unwrap();

                if state.connections.len() < MAX_CONNECTIONS {
                    state
                        .connections
                        .push(Connection {
                            peer: addr,
                            conn_id,
                            subscribed: false,
                            mtu: None,
                        })
                        .map_err(|_| ())
                        .unwrap();

                    // restart advertising so we can get a new connection
                    self.set_adv_conf()?;

                    true
                } else {
                    false
                }
            };

            if added {
                self.gap.set_conn_params_conf(addr, 10, 20, 0, 400)?;
            }

            Ok(())
        }

        /// Delete a connection
        /// Called from within the event callback once we are notified for a disconnected peer
        fn delete_conn(&self, addr: BdAddr) -> Result<(), EspError> {
            let mut state = self.state.lock().unwrap();

            if let Some(index) = state
                .connections
                .iter()
                .position(|Connection { peer, .. }| *peer == addr)
            {
                state.connections.swap_remove(index);

                // restart advertising so we can get a new connection
                self.set_adv_conf()?;
            }

            Ok(())
        }

        /// A helper method to process a client sending us data to the "recv" characteristic
        #[allow(clippy::too_many_arguments)]
        fn recv(
            &self,
            _gatt_if: GattInterface,
            conn_id: ConnectionId,
            _trans_id: TransferId,
            addr: BdAddr,
            handle: Handle,
            offset: u16,
            _need_rsp: bool,
            _is_prep: bool,
            value: &[u8],
        ) -> Result<bool, EspError> {
            let mut state = self.state.lock().unwrap();

            let recv_handle = state.recv_handle;
            let ind_cccd_handle = state.ind_cccd_handle;

            let Some(conn) = state
                .connections
                .iter_mut()
                .find(|conn| conn.conn_id == conn_id)
            else {
                return Ok(false);
            };

            if Some(handle) == ind_cccd_handle {
                // Subscribe or unsubscribe to our indication characteristic

                if offset == 0 && value.len() == 2 {
                    let value = u16::from_le_bytes([value[0], value[1]]);
                    if value == 0x02 {
                        if !conn.subscribed {
                            conn.subscribed = true;
                            self.on_subscribed(conn.peer);
                        }
                    } else if conn.subscribed {
                        conn.subscribed = false;
                        self.on_unsubscribed(conn.peer);
                    }
                }
            } else if Some(handle) == recv_handle {
                // Receive data on the recv characteristic

                self.on_recv(addr, value, offset, conn.mtu);
            } else {
                return Ok(false);
            }

            Ok(true)
        }

        /// A helper method that sends a response to the peer that just sent us some data on the "recv"
        /// characteristic.
        ///
        /// This is only necessary, because we support write confirmation
        /// (which is the more complex case as compared to unconfirmed writes).
        #[allow(clippy::too_many_arguments)]
        fn send_write_response(
            &self,
            gatt_if: GattInterface,
            conn_id: ConnectionId,
            trans_id: TransferId,
            handle: Handle,
            offset: u16,
            need_rsp: bool,
            is_prep: bool,
            value: &[u8],
        ) -> Result<(), EspError> {
            if !need_rsp {
                return Ok(());
            }

            if is_prep {
                let mut state = self.state.lock().unwrap();

                state
                    .response
                    .attr_handle(handle)
                    .auth_req(0)
                    .offset(offset)
                    .value(value)
                    .map_err(|_| EspError::from_infallible::<ESP_FAIL>())?;

                self.gatts.send_response(
                    gatt_if,
                    conn_id,
                    trans_id,
                    GattStatus::Ok,
                    Some(&state.response),
                )?;
            } else {
                self.gatts
                    .send_response(gatt_if, conn_id, trans_id, GattStatus::Ok, None)?;
            }

            Ok(())
        }

        /// A helper method to handle an indication conrimation.
        /// Basically, we need to notify the "indicate" method that sending the indication was
        /// confirmed, so that it is free to send the next indication.
        fn confirm_indication(&self) -> Result<(), EspError> {
            let mut state = self.state.lock().unwrap();
            if state.ind_confirmed.is_none() {
                // Should not happen: means we have received a confirmation for
                // an indication we did not send.
                unreachable!();
            }

            state.ind_confirmed = None; // So that the main loop can send the next indication
            self.condvar.notify_all();

            Ok(())
        }

        fn check_esp_status(&self, status: Result<(), EspError>) {
            if let Err(e) = status {
                warn!("Got status: {e:?}");
            }
        }

        fn check_bt_status(&self, status: BtStatus) -> Result<(), EspError> {
            if !matches!(status, BtStatus::Success) {
                warn!("Got status: {status:?}");
                Err(EspError::from_infallible::<ESP_FAIL>())
            } else {
                Ok(())
            }
        }

        fn check_gatt_status(&self, status: GattStatus) -> Result<(), EspError> {
            if !matches!(status, GattStatus::Ok) {
                warn!("Got status: {status:?}");
                Err(EspError::from_infallible::<ESP_FAIL>())
            } else {
                Ok(())
            }
        }
    }
}
