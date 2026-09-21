//! Example of a BLE GAP scanner using the ESP IDF Bluedroid BLE bindings.
//!
//! The example prints discovered ble devices on the console.
//!
//! Note that the Buedroid stack consumes a lot of memory, so `sdkconfig.defaults` should be carefully configured
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
    use core::fmt;
    use core::hash::{Hash, Hasher};
    use std::sync::{Arc, Mutex};

    use esp_idf_svc::bt::ble::gap::{BleGapEvent, EspBleGap, GapSearchEvent, GapSearchResult};
    use esp_idf_svc::bt::{BdAddr, Ble, BtDriver};
    use esp_idf_svc::hal::delay::FreeRtos;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::nvs::EspDefaultNvsPartition;
    use esp_idf_svc::sys::EspError;

    use log::{info, trace, warn};

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let bt = Arc::new(BtDriver::new(peripherals.modem, Some(nvs.clone()))?);

        let server = ExampleServer::new(Arc::new(EspBleGap::new(bt.clone())?));

        info!("BLE Gap initialized");

        let gap_server = server.clone();

        server.gap.subscribe(move |event| {
            gap_server.check_esp_status(gap_server.on_gap_event(event));
        })?;

        info!("BLE Gap subscriptions initialized");

        loop {
            server.start_scanning(0)?;
            info!("Started ble scanning...");

            FreeRtos::delay_ms(20000);
            server.stop_scanning()?;
            info!("Stopped ble scanning.");
        }
    }

    // Name the types as they are used in the example to get shorter type signatures in the various functions below.
    // note that - rather than `Arc`s, you can use regular references as well, but then you have to deal with lifetimes
    // and the signatures below will not be `'static`.
    type ExBtDriver = BtDriver<'static, Ble>;
    type ExEspBleGap = Arc<EspBleGap<'static, Ble, Arc<ExBtDriver>>>;

    #[derive(PartialEq, Eq, Debug, Copy, Clone)]
    struct BluetoothAddress(BdAddr);

    #[derive(Default)]
    struct State {
        discovered: heapless::index_set::FnvIndexSet<BluetoothAddress, 64>,
    }

    #[derive(Clone)]
    pub struct ExampleServer {
        gap: ExEspBleGap,
        state: Arc<Mutex<State>>,
    }

    impl fmt::Display for BluetoothAddress {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt(f)
        }
    }

    impl Hash for BluetoothAddress {
        fn hash<H: Hasher>(&self, state: &mut H) {
            state.write(&self.0.addr())
        }
    }

    impl ExampleServer {
        pub fn new(gap: ExEspBleGap) -> Self {
            Self {
                gap,
                state: Arc::new(Mutex::new(Default::default())),
            }
        }
    }

    impl ExampleServer {
        fn start_scanning(&self, duration: u32) -> Result<(), EspError> {
            let mut state = self.state.lock().unwrap();
            state.discovered.clear();

            self.gap.start_scanning(duration)
        }

        fn stop_scanning(&self) -> Result<(), EspError> {
            self.gap.stop_scanning()
        }

        /// The main event handler for the GAP events
        fn on_gap_event(&self, event: BleGapEvent) -> Result<(), EspError> {
            trace!("Got event: {event:?}");

            if let BleGapEvent::ScanResult(GapSearchEvent::InquiryResult(GapSearchResult {
                bda,
                ..
            })) = event
            {
                let mut state = self.state.lock().unwrap();
                let address = BluetoothAddress(bda);
                match state.discovered.insert(address) {
                    Ok(true) => info!("Discovered new device {address}"),
                    Err(_) => warn!("Error while storing address: {address}"),
                    _ => (),
                }
            }

            Ok(())
        }

        fn check_esp_status(&self, status: Result<(), EspError>) {
            if let Err(e) = status {
                warn!("Got status: {e:?}");
            }
        }
    }
}
