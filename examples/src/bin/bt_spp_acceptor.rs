//! Example of a BT Classic Serial Port Profile server (SPP) using the ESP IDF Bluedroid BT bindings.
//!
//!
//! The example will start an SPP acceptor. The example will calculate the data rate and prints
//! the received data after the SPP connection is established.
//! You can connect to the server and send data with another ESP32 development board,
//! Andriod phone or computer which performs as the SPP initiator.
//!
//! Note that the Bluedroid stack consumes a lot of memory, so `sdkconfig.defaults` should be carefully configured
//! to avoid running out of memory.
//!
//! Here's a working configuration for BT Classic only, but you might need to adjust further to your concrete use-case:
//!
//! CONFIG_BT_CLASSIC_ENABLED=y
//! CONFIG_BT_ENABLED=y
//! CONFIG_BT_BLUEDROID_ENABLED=y
//! CONFIG_BT_SPP_ENABLED=y
//! CONFIG_BT_BTC_TASK_STACK_SIZE=15000
//! CONFIG_BTDM_CTRL_MODE_BLE_ONLY=n
//! CONFIG_BTDM_CTRL_MODE_BR_EDR_ONLY=y
//! CONFIG_BTDM_CTRL_MODE_BTDM=n

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(esp32, esp_idf_bt_bluedroid_enabled))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(not(all(esp32, esp_idf_bt_bluedroid_enabled)))]
fn main() -> anyhow::Result<()> {
    panic!("This example requires a Bluedroid-enabled build (CONFIG_BT_BLUEDROID_ENABLED=y) on an ESP32, the only chip with BT Classic");
}

#[cfg(all(esp32, esp_idf_bt_bluedroid_enabled))]
mod example {
    use std::fmt::Write;
    use std::ptr::slice_from_raw_parts;
    use std::sync::Arc;
    use std::time::{self, SystemTime};

    use esp_idf_svc::bt::gap::{DiscoveryMode, EspGap, GapEvent};
    use esp_idf_svc::bt::spp::{self, EspSpp, SppConfig, SppEvent, Status};
    use esp_idf_svc::bt::{reduce_bt_memory, BtClassic, BtDriver, BtStatus};
    use esp_idf_svc::hal::delay::FreeRtos;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::nvs::EspDefaultNvsPartition;
    use esp_idf_svc::sys::EspError;

    use esp_idf_sys::{
        esp, esp_bt_gap_set_security_param, esp_bt_gap_ssp_confirm_reply,
        esp_bt_sp_param_t_ESP_BT_SP_IOCAP_MODE, ESP_BT_IO_CAP_IO,
    };
    use log::{error, info, warn};

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let mut modem = peripherals.modem;

        reduce_bt_memory(unsafe { modem.reborrow() })?;

        let bt = Arc::new(BtDriver::<BtClassic>::new(modem, Some(nvs.clone()))?);

        let spp_config = SppConfig {
            mode: spp::Mode::Cb,
            enable_l2cap_ertm: true,
            tx_buffer_size: 0, // Only used for mode VFS
        };

        let gap = EspGap::new(bt.clone())?;

        gap.set_device_name("ESP32_SPP_SERVER")?;

        let server = ExampleServer::new(
            Arc::new(gap),
            Arc::new(EspSpp::new(bt.clone(), &spp_config)?),
        );

        info!("BT Gap and Spp initialized");

        let gap_server = server.clone();

        server.gap.subscribe(move |event| {
            gap_server.check_esp_status(gap_server.on_gap_event(event));
        })?;

        info!("BT Gap subscriptions initialized");

        let mut spp_server = server.clone();

        server.spp.subscribe(move |event| {
            let r = spp_server.on_spp_event(event);
            spp_server.check_esp_status(r);
        })?;

        info!("BT Spp subscriptions initialized");

        server.create_service()?;

        info!("Started BT service");

        loop {
            FreeRtos::delay_ms(10000);
        }
    }

    // Name the types as they are used in the example to get shorter type signatures in the various functions below.
    // note that - rather than `Arc`s, you can use regular references as well, but then you have to deal with lifetimes
    // and the signatures below will not be `'static`.
    type ExBtDriver = BtDriver<'static, BtClassic>;
    type ExEspGap = Arc<EspGap<'static, BtClassic, Arc<ExBtDriver>>>;
    type ExEspSpp = Arc<EspSpp<'static, BtClassic, Arc<ExBtDriver>>>;

    #[derive(Clone)]
    pub struct ExampleServer {
        gap: ExEspGap,
        spp: ExEspSpp,
        data_num: usize,
        time_old: SystemTime,
        time_new: SystemTime,
    }

    impl ExampleServer {
        pub fn new(gap: ExEspGap, spp: ExEspSpp) -> Self {
            let time_old = time::SystemTime::now();
            Self {
                gap,
                spp,
                data_num: 0,
                time_old,
                time_new: time_old,
            }
        }

        /// The main event handler for the GAP events
        fn on_gap_event(&self, event: GapEvent) -> Result<(), EspError> {
            match event {
                GapEvent::AuthenticationCompleted {
                    bd_addr,
                    status,
                    device_name,
                } => {
                    if status == BtStatus::Success {
                        info!("ESP_BT_GAP_AUTH_CMPL_EVT authentication success: {device_name} bda:{bd_addr}");
                    } else {
                        error!("ESP_BT_GAP_AUTH_CMPL_EVT authentication failed, status:{status:?}");
                    }
                }
                GapEvent::PairingPinRequest {
                    bd_addr,
                    min_16_digit,
                } => {
                    info!("ESP_BT_GAP_PIN_REQ_EVT min_16_digit:{min_16_digit}");

                    if min_16_digit {
                        info!("Input pin code: 0000 0000 0000 0000");

                        let mut pin_code = heapless::Vec::<u8, 16>::new();

                        pin_code.fill(0);

                        self.gap.reply_variable_pin(&bd_addr, Some(&pin_code))?;
                    } else {
                        info!("Input pin code: 1234");

                        self.gap.reply_variable_pin(&bd_addr, Some(&[1, 2, 3, 4]))?;
                    };
                }
                GapEvent::PairingUserConfirmationRequest { bd_addr, number } => {
                    info!("ESP_BT_GAP_CFM_REQ_EVT Please compare the numeric value: {number}");

                    // bt_ssp_enable configuration parameter has been removed from esp idf,
                    // Directly set the reply instead of self.gap.reply_ssp_confirm(&bd_addr, true)
                    // due to cfg(esp_idf_bt_ssp_enabled) eliding the function.
                    esp!(unsafe {
                        esp_bt_gap_ssp_confirm_reply(&bd_addr as *const _ as *mut _, true)
                    })?;
                }
                GapEvent::SspPasskey { bd_addr, passkey } => {
                    info!("ESP_BT_GAP_KEY_NOTIF_EVT bda:{bd_addr} passkey:{passkey}");
                }
                GapEvent::SspPasskeyRequest { bd_addr } => {
                    info!("ESP_BT_GAP_KEY_REQ_EVT bda:{bd_addr} Please enter passkey!");
                }
                GapEvent::ModeChange { bd_addr, mode } => {
                    info!("ESP_BT_GAP_MODE_CHG_EVT mode:{mode} bda:{bd_addr}");
                }
                GapEvent::AclConnected {
                    bd_addr,
                    status,
                    handle,
                } => {
                    if status == BtStatus::Success {
                        info!("ESP_BT_GAP_ACL_CONNECTED_EVT success bda:{bd_addr} handle:{handle}");
                    } else {
                        info!("ESP_BT_GAP_ACL_CONNECTED_EVT failed, status:{status:?} bda:{bd_addr} handle:{handle}");
                    }
                }
                GapEvent::AclDisconnected {
                    bd_addr,
                    status,
                    handle,
                } => {
                    if status == BtStatus::Success {
                        info!(
                            "ESP_BT_GAP_ACL_DISCONNECTED_EVT success bda:{bd_addr} handle:{handle}"
                        );
                    } else {
                        info!("ESP_BT_GAP_ACL_DISCONNECTED_EVT status:{status:?} bda:{bd_addr} handle:{handle}");
                    }
                }

                _ => {
                    info!("Got ESP_BT_GAP event: {event:?}");
                }
            }

            Ok(())
        }

        /// The main event handler for the SPP events
        fn on_spp_event(&mut self, event: SppEvent) -> Result<(), EspError> {
            // note: the INIT event will not be received because the SPP callback and init are
            // done in EspSpp::new(), before the event subscription can be made.

            match event {
                SppEvent::Start {
                    status,
                    handle,
                    sec_id,
                    scn,
                    use_co,
                } => {
                    if status == Status::Success {
                        info!("ESP_SPP_START_EVT handle:{handle} sec_id:{sec_id} scn:{scn} use_co:{use_co}");

                        self.gap.set_scan_mode(true, DiscoveryMode::Discoverable)?;
                    } else {
                        error!("ESP_SPP_START_EVT status: {status:?}");
                    }
                }
                SppEvent::ServerOpen {
                    status,
                    handle,
                    listen_handle,
                    fd,
                    rem_bda,
                } => {
                    info!("ESP_SPP_SRV_OPEN_EVT status:{status:?} handle:{handle} listen_handle:{listen_handle} fd:{fd}, rem_bda:{rem_bda}");
                }
                SppEvent::Close {
                    status,
                    port_status,
                    handle,
                    async_,
                } => {
                    info!("ESP_SPP_CLOSE_EVT status:{status:?} port_status:{port_status} handle:{handle} close_by_remote:{async_}");
                }
                SppEvent::DataInd {
                    status,
                    handle,
                    length,
                    data,
                } => {
                    if status == Status::Success {
                        info!("ESP_SPP_DATA_IND_EVT len:{length} handle:{handle}");

                        if length < 128 {
                            const MAX_BYTES: usize = 127 * 2 + 127 - 1;
                            let mut hex_string = heapless::String::<MAX_BYTES>::new();

                            for b in unsafe { &*slice_from_raw_parts(data, length as _) } {
                                let _ = write!(&mut hex_string, "{b:02X} ");
                            }
                            info!("{hex_string}");
                        }

                        self.data_num += length as usize;
                        self.time_new = time::SystemTime::now();

                        if self
                            .time_new
                            .duration_since(self.time_old)
                            .expect("time")
                            .as_secs()
                            >= 3
                        {
                            self.print_speed();
                        }
                    } else {
                        error!("ESP_SPP_DATA_IND_EVT status: {status:?}");
                    }
                }
                _ => {
                    info!("Got ESP_SPP event: {event:?}");
                }
            }

            Ok(())
        }

        fn print_speed(&mut self) {
            let time_old_s: f64 = self
                .time_old
                .duration_since(time::UNIX_EPOCH)
                .expect("time")
                .as_secs() as f64;
            let time_new_s: f64 = self
                .time_new
                .duration_since(time::UNIX_EPOCH)
                .expect("time")
                .as_secs() as f64;
            let time_interval: f64 = time_new_s - time_old_s;
            let speed = self.data_num as f64 * 8.0 / time_interval / 1000.0;

            info!("speed({time_old_s} ~ {time_new_s}): {speed} kbit/s");

            self.data_num = 0;
            self.time_old = self.time_new;
        }

        /// Start the SPP server
        fn create_service(&self) -> Result<(), EspError> {
            self.spp.start_server(
                spp::Security::Authenticate,
                spp::Role::Slave,
                0,
                "ESP SPP SERVER",
            )?;

            // CONFIG_BT_SSP_ENABLED has been removed from esp idf and been replaced with
            // a runtime cfg setting that can be passed into esp_bluedroid_init_with_cfg;
            // however, this method is not (yet) used by esp_idf_svc BtDriver for bluedroid init and
            // by default ssp is enabled in esp idf. So we assume SSP is always enabled.
            // Directly set the IO capabilities instead of using the
            // self.gap.set_ssp_io_cap(IOCapabilities::None) function
            // due to cfg(esp_idf_bt_ssp_enabled) eliding the function.
            // Set default parameters for Secure Simple Pairing
            esp!(unsafe {
                esp_bt_gap_set_security_param(
                    esp_bt_sp_param_t_ESP_BT_SP_IOCAP_MODE,
                    &ESP_BT_IO_CAP_IO as *const _ as *mut std::ffi::c_void,
                    1,
                )
            })?;

            // Set default parameters for Legacy Pairing
            // Use variable pin, input pin code when pairing
            self.gap.request_variable_pin()?;

            Ok(())
        }

        fn check_esp_status(&self, status: Result<(), EspError>) {
            if let Err(e) = status {
                warn!("Got status: {e:?}");
            }
        }
    }
}
