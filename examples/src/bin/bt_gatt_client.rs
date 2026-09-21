//! Example of a BLE GATT client using the ESP IDF Bluedroid BLE bindings.
//!
//! You can test it with the `bt_gatt_server`
//!
//! The example client will scan for, and connect to the server name ESP32. Once connected the
//! client will subscribe and unsubscribe to the indications and write to the "recv" characteristic.
//! It will then disconnect and reconnect, repeating through the indication and writing again.
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
    use std::thread;
    use std::time::Duration;

    use esp_idf_svc::bt::ble::gap::{
        AdvertisingDataType, BleGapEvent, EspBleGap, GapSearchEvent, GapSearchResult, ScanParams,
    };
    use esp_idf_svc::bt::ble::gatt::client::{
        CharacteristicElement, ConnectionId, DbAttrType, DbElement, DescriptorElement, EspGattc,
        GattAuthReq, GattCreateConnParams, GattWriteType, GattcEvent, ServiceSource,
    };
    use esp_idf_svc::bt::ble::gatt::{self, GattInterface, GattStatus, Handle, Property};
    use esp_idf_svc::bt::{BdAddr, Ble, BtDriver, BtStatus, BtUuid};
    use esp_idf_svc::hal::delay::FreeRtos;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::nvs::EspDefaultNvsPartition;
    use esp_idf_svc::sys::{EspError, ESP_FAIL};

    use log::{error, info, warn};

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let bt = Arc::new(BtDriver::new(peripherals.modem, Some(nvs.clone()))?);

        let client = ExampleClient::new(
            Arc::new(EspBleGap::new(bt.clone())?),
            Arc::new(EspGattc::new(bt.clone())?),
        );

        info!("BLE Gap and Gattc initialized");

        let gap_client = client.clone();

        client.gap.subscribe(move |event| {
            gap_client.check_esp_status(gap_client.on_gap_event(event));
        })?;

        let gattc_client = client.clone();

        client.gattc.subscribe(move |(gatt_if, event)| {
            gattc_client.check_esp_status(gattc_client.on_gattc_event(gatt_if, event))
        })?;

        info!("BLE Gap and Gattc subscriptions initialized");

        client.gattc.register_app(APP_ID)?;

        info!("Gattc BTP app registered");

        gatt::set_local_mtu(500)?;

        info!("Gattc BTP app registered");

        client.wait_for_write_char_handle();
        let mut write_data = 0_u16;
        let mut indicate = true;

        loop {
            // Subscribe/unsubscribe to indications
            if write_data % 10 == 0 {
                client.request_indicate(indicate)?;
                indicate = !indicate;
            }

            client.write_characterisitic(&write_data.to_le_bytes())?;

            info!("Wrote characteristic: {write_data}");

            write_data = write_data.wrapping_add(1);

            FreeRtos::delay_ms(5000);

            if write_data % 30 == 0 {
                client.disconnect()?;
                FreeRtos::delay_ms(5000);
                client.connect()?;
            }
        }
    }

    const APP_ID: u16 = 0;

    // bt_gatt_server name
    pub const SERVER_NAME: &str = "ESP32";
    // bt_gatt_server service UUID
    pub const SERVICE_UUID: BtUuid = BtUuid::uuid128(0xad91b201734740479e173bed82d75f9d);
    // Write characteristic UUID
    pub const WRITE_CHARACTERISITIC_UUID: BtUuid =
        BtUuid::uuid128(0xb6fccb5087be44f3ae22f85485ea42c4);
    // Indicate characteristic UUID
    pub const IND_CHARACTERISTIC_UUID: BtUuid = BtUuid::uuid128(0x503de214868246c4828fd59144da41be);
    // Client Characteristic Configuration UUID
    pub const IND_DESCRIPTOR_UUID: BtUuid = BtUuid::uuid16(0x2902);

    // Name the types as they are used in the example to get shorter type signatures in the various functions below.
    // note that - rather than `Arc`s, you can use regular references as well, but then you have to deal with lifetimes
    // and the signatures below will not be `'static`.
    type ExBtDriver = BtDriver<'static, Ble>;
    type ExEspBleGap = Arc<EspBleGap<'static, Ble, Arc<ExBtDriver>>>;
    type ExEspGattc = Arc<EspGattc<'static, Ble, Arc<ExBtDriver>>>;

    #[derive(Default)]
    struct State {
        gattc_if: Option<GattInterface>,
        conn_id: Option<ConnectionId>,
        remote_addr: Option<BdAddr>,
        connect: bool,
        service_start_end_handle: Option<(Handle, Handle)>,
        ind_char_handle: Option<Handle>,
        ind_descr_handle: Option<Handle>,
        write_char_handle: Option<Handle>,
    }

    #[derive(Clone)]
    pub struct ExampleClient {
        gap: ExEspBleGap,
        gattc: ExEspGattc,
        state: Arc<Mutex<State>>,
        condvar: Arc<Condvar>,
    }

    impl ExampleClient {
        pub fn new(gap: ExEspBleGap, gattc: ExEspGattc) -> Self {
            Self {
                gap,
                gattc,
                state: Arc::new(Mutex::new(Default::default())),
                condvar: Arc::new(Condvar::new()),
            }
        }

        /// The main event handler for the GAP events
        fn on_gap_event(&self, event: BleGapEvent) -> Result<(), EspError> {
            info!("Got gap event: {event:?}");

            match event {
                BleGapEvent::ScanParameterConfigured(status) => {
                    self.check_bt_status(status)?;
                    self.gap.start_scanning(30)?;
                }
                BleGapEvent::ScanStarted(status) => {
                    self.check_bt_status(status)?;
                    info!("Scanning started");
                }
                BleGapEvent::ScanResult(search_evt) => {
                    if matches!(search_evt, GapSearchEvent::InquiryComplete(_)) {
                        info!("Scan completed, no server {SERVER_NAME} found");
                    } else if let GapSearchEvent::InquiryResult(GapSearchResult {
                        bda,
                        ble_addr_type,
                        rssi,
                        ble_adv,
                        ..
                    }) = search_evt
                    {
                        let name = ble_adv
                            .and_then(|ble_adv| {
                                self.gap.resolve_adv_data_by_type(
                                    ble_adv,
                                    AdvertisingDataType::NameCmpl,
                                )
                            })
                            .map(|n| std::str::from_utf8(n))
                            .transpose()
                            .ok()
                            .flatten();

                        info!("Scan result, device {bda} - rssi {rssi}, name: {name:?}");

                        if let Some(name) = name.filter(|n| *n == SERVER_NAME) {
                            info!("Device found: {name:?}");

                            let mut state = self.state.lock().unwrap();

                            if !state.connect {
                                state.connect = true;
                                info!("Connect to remote {bda}");
                                self.gap.stop_scanning()?;

                                let conn_params = GattCreateConnParams::new(bda, ble_addr_type);

                                self.gattc.enh_open(state.gattc_if.unwrap(), &conn_params)?;
                            }
                        }

                        // If there are many devices found the logging tends to take too long and the wdt kicks in
                        thread::sleep(Duration::from_millis(10));
                    }
                }
                BleGapEvent::ScanStopped(status) => {
                    self.check_bt_status(status)?;

                    info!("Scanning stopped");
                }
                BleGapEvent::ConnectionParamsConfigured {
                    addr,
                    status,
                    min_int_ms,
                    max_int_ms,
                    latency_ms,
                    conn_int,
                    timeout_ms,
                } => {
                    info!("Connection params update addr {addr}, status {status:?}, conn_int {conn_int}, latency {latency_ms}, timeout {timeout_ms}, min_int {min_int_ms}, max_int {max_int_ms}");
                }
                BleGapEvent::PacketLengthConfigured {
                    status,
                    rx_len,
                    tx_len,
                } => {
                    info!("Packet length update, status {status:?}, rx {rx_len}, tx {tx_len}");
                }
                _ => (),
            }

            Ok(())
        }

        /// The main event handler for the GATTC events
        fn on_gattc_event(
            &self,
            gattc_if: GattInterface,
            event: GattcEvent,
        ) -> Result<(), EspError> {
            info!("Got gattc event: {event:?}");

            match event {
                GattcEvent::ClientRegistered { status, app_id } => {
                    self.check_gatt_status(status)?;
                    if APP_ID == app_id {
                        self.state.lock().unwrap().gattc_if = Some(gattc_if);
                        self.connect()?;
                    }
                }
                GattcEvent::Connected { conn_id, addr, .. } => {
                    let mut state = self.state.lock().unwrap();

                    state.conn_id = Some(conn_id);
                    state.remote_addr = Some(addr);

                    self.gattc.mtu_req(gattc_if, conn_id)?;
                }
                GattcEvent::Open {
                    status, addr, mtu, ..
                } => {
                    self.check_gatt_status(status)?;

                    info!("Open successfully with {addr}, MTU {mtu}");
                }
                GattcEvent::DiscoveryCompleted { status, conn_id } => {
                    self.check_gatt_status(status)?;

                    info!("Service discover complete, conn_id {conn_id}");

                    self.gattc
                        .search_service(gattc_if, conn_id, Some(&SERVICE_UUID))?;
                }
                GattcEvent::Mtu { status, mtu, .. } => {
                    info!("MTU exchange, status {status:?}, MTU {mtu}");
                }
                GattcEvent::SearchResult {
                    conn_id,
                    start_handle,
                    end_handle,
                    srvc_id,
                    is_primary,
                } => {
                    info!("Service search result, conn_id {conn_id}, is primary service {is_primary}, start handle {start_handle}, end handle {end_handle}, current handle value {}", srvc_id.inst_id);

                    if srvc_id.uuid == SERVICE_UUID {
                        info!("Service found, uuid {:?}", srvc_id.uuid);

                        self.state.lock().unwrap().service_start_end_handle =
                            Some((start_handle, end_handle));
                    }
                }
                GattcEvent::SearchComplete {
                    status,
                    conn_id,
                    searched_service_source,
                } => {
                    self.check_gatt_status(status)?;

                    match searched_service_source {
                        ServiceSource::RemoteDevice => {
                            info!("Get service information from remote device")
                        }
                        ServiceSource::Nvs => {
                            info!("Get service information from flash")
                        }
                        _ => {
                            info!("Unknown service source")
                        }
                    };
                    info!("Service search complete");

                    let mut state = self.state.lock().unwrap();

                    if let Some((start_handle, end_handle)) = state.service_start_end_handle {
                        // Enumerate all the elements for info purposes
                        let mut db_results = [DbElement::new(); 10];
                        match self.gattc.get_db(
                            gattc_if,
                            conn_id,
                            start_handle,
                            end_handle,
                            &mut db_results,
                        ) {
                            Ok(db_count) => {
                                info!("Found {db_count} DB elements");

                                if db_count > 0 {
                                    for db_elem in db_results[..db_count].iter() {
                                        info!("DB element {db_elem:?}");
                                    }
                                } else {
                                    info!("No DB elements found?");
                                }
                            }
                            Err(status) => {
                                error!("Get all DB elements error {status:?}");
                            }
                        }

                        let char_count = self
                            .gattc
                            .get_attr_count(
                                gattc_if,
                                conn_id,
                                DbAttrType::Characteristic {
                                    start_handle,
                                    end_handle,
                                },
                            )
                            .map_err(|status| {
                                error!("Get attr count error for service {status:?}");
                                EspError::from_infallible::<ESP_FAIL>()
                            })?;

                        info!("Found {char_count} characteristics");

                        if char_count > 0 {
                            // Get the indicator characteristic handle and register for notification
                            let mut chars = [CharacteristicElement::new(); 1];

                            match self.gattc.get_characteristic_by_uuid(
                                gattc_if,
                                conn_id,
                                start_handle,
                                end_handle,
                                &IND_CHARACTERISTIC_UUID,
                                &mut chars,
                            ) {
                                Ok(char_count) => {
                                    if char_count > 0 {
                                        if let Some(ind_char_elem) = chars.first() {
                                            if ind_char_elem
                                                .properties()
                                                .contains(Property::Indicate)
                                            {
                                                if let Some(remote_addr) = state.remote_addr {
                                                    state.ind_char_handle =
                                                        Some(ind_char_elem.handle());
                                                    self.gattc.register_for_notify(
                                                        gattc_if,
                                                        &remote_addr,
                                                        ind_char_elem.handle(),
                                                    )?;
                                                }
                                            } else {
                                                error!("Ind characteristic does not have property Indicate");
                                            }
                                        }
                                    } else {
                                        error!("No ind characteristic found");
                                    }
                                }
                                Err(status) => {
                                    error!("Get ind characteristic error {status:?}");
                                }
                            };

                            // Get the write characteristic handle and start sending data to the server
                            match self.gattc.get_characteristic_by_uuid(
                                gattc_if,
                                conn_id,
                                start_handle,
                                end_handle,
                                &WRITE_CHARACTERISITIC_UUID,
                                &mut chars,
                            ) {
                                Ok(char_count) => {
                                    if char_count > 0 {
                                        if let Some(write_char_elem) = chars.first() {
                                            if write_char_elem
                                                .properties()
                                                .contains(Property::Write)
                                            {
                                                state.write_char_handle =
                                                    Some(write_char_elem.handle());

                                                // Let main loop send write
                                                self.condvar.notify_all();
                                            } else {
                                                error!(
                                                "Write characteristic does not have property Write"
                                            );
                                            }
                                        }
                                    } else {
                                        error!("No write characteristic found");
                                    }
                                }
                                Err(status) => {
                                    error!("Get write characteristic error {status:?}");
                                }
                            };
                        } else {
                            error!("No characteristics found");
                        }
                    };
                }
                GattcEvent::RegisterNotify { status, handle } => {
                    self.check_gatt_status(status)?;

                    info!("Notification register successfully");

                    let mut state = self.state.lock().unwrap();

                    if let Some(conn_id) = state.conn_id {
                        let count = self
                            .gattc
                            .get_attr_count(gattc_if, conn_id, DbAttrType::Descriptor { handle })
                            .map_err(|status| {
                                error!("Get attr count for ind char error {status:?}");
                                EspError::from_infallible::<ESP_FAIL>()
                            })?;

                        if count > 0 {
                            let mut descrs = [DescriptorElement::new(); 1];

                            match self.gattc.get_descriptor_by_char_handle(
                                gattc_if,
                                conn_id,
                                handle,
                                &IND_DESCRIPTOR_UUID,
                                &mut descrs,
                            ) {
                                Ok(descrs_count) => {
                                    if descrs_count > 0 {
                                        if let Some(descr) = descrs.first() {
                                            if descr.uuid() == IND_DESCRIPTOR_UUID {
                                                state.ind_descr_handle = Some(descr.handle());
                                            }
                                        }
                                    } else {
                                        error!("No ind descriptor found");
                                    }
                                }
                                Err(status) => {
                                    error!("Get ind char descriptors error {status:?}");
                                }
                            }
                        } else {
                            error!("No ind char descriptors found");
                        }
                    }
                }
                GattcEvent::Notify {
                    addr,
                    handle,
                    value,
                    is_notify,
                    ..
                } => {
                    info!(
                        "Got is_notify {is_notify}, addr {addr}, handle {handle}, value {value:?}"
                    );
                }
                GattcEvent::WriteDescriptor { status, .. } => {
                    self.check_gatt_status(status)?;

                    info!("Descriptor write successful");
                }
                GattcEvent::ServiceChanged { addr } => {
                    info!("Service change from {addr}");
                }
                GattcEvent::WriteCharacteristic { status, .. } => {
                    self.check_gatt_status(status)?;

                    info!("Characteristic write successful");
                }
                GattcEvent::Disconnected { addr, reason, .. } => {
                    let mut state = self.state.lock().unwrap();
                    state.connect = false;
                    state.remote_addr = None;
                    state.conn_id = None;
                    state.service_start_end_handle = None;
                    state.ind_char_handle = None;
                    state.ind_descr_handle = None;
                    state.write_char_handle = None;
                    info!("Disconnected, remote {addr}, reason {reason:?}");
                }
                _ => (),
            }
            Ok(())
        }

        /// Connect to the bt_gatt_server.
        ///
        /// This sets the scan params with triggers the event `BleGapEvent::ScanParameterConfigured` where the
        /// gap callback will start scanning. Scanning must happen before a connect can be made in `BleGapEvent::ScanResult`.
        pub fn connect(&self) -> Result<(), EspError> {
            if !self.state.lock().unwrap().connect {
                let scan_params = ScanParams {
                    scan_interval: 0x50,
                    ..Default::default()
                };

                self.gap.set_scan_params(&scan_params)?;
            }

            Ok(())
        }

        /// Disconnect from the bt_gatt_server.
        ///
        /// This does a physical disconnect, a `gattc.close` can also be used to close a virtual connection which will also disconnect
        /// if there are no more virtual connections.
        pub fn disconnect(&self) -> Result<(), EspError> {
            let state = self.state.lock().unwrap();

            if let Some(remote_addr) = state.remote_addr {
                self.gap.disconnect(remote_addr)?;
            }

            Ok(())
        }

        /// Subscribe or unsubsrcibe to the notifications.
        ///
        /// After registering for notify the CCCD descriptor is written to enable/disbale the notification.
        pub fn request_indicate(&self, indicate: bool) -> Result<(), EspError> {
            let state = self.state.lock().unwrap();

            let Some(gattc_if) = state.gattc_if else {
                return Ok(());
            };
            let Some(conn_id) = state.conn_id else {
                return Ok(());
            };

            if let Some(ind_descr_handle) = state.ind_descr_handle {
                let value = if indicate {
                    info!("Subscribe indicate");
                    2_u16
                } else {
                    info!("Unsubscribe indicate");
                    0_u16
                }
                .to_le_bytes();

                self.gattc.write_descriptor(
                    gattc_if,
                    conn_id,
                    ind_descr_handle,
                    &value,
                    GattWriteType::RequireResponse,
                    GattAuthReq::None,
                )?;
            }

            Ok(())
        }

        /// Wait for the discovery of the write characteristic handle.
        pub fn wait_for_write_char_handle(&self) {
            let mut state = self.state.lock().unwrap();
            while state.write_char_handle.is_none() {
                state = self.condvar.wait(state).unwrap();
            }
        }

        // Write some data to the write characteristic.
        pub fn write_characterisitic(&self, char_value: &[u8]) -> Result<(), EspError> {
            let state = self.state.lock().unwrap();

            let Some(gattc_if) = state.gattc_if else {
                return Ok(());
            };
            let Some(conn_id) = state.conn_id else {
                return Ok(());
            };

            if let Some(write_char_handle) = state.write_char_handle {
                self.gattc.write_characteristic(
                    gattc_if,
                    conn_id,
                    write_char_handle,
                    char_value,
                    GattWriteType::RequireResponse,
                    GattAuthReq::None,
                )?;
            }

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
