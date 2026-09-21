//! A simple example of how to run the ESP IDF SNTP service so that it updates the current time
//! in the MCU by periodically consulting an NTP internet server

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(not(any(esp32h2, esp32h4, esp32p4)))]
fn main() -> Result<(), esp_idf_svc::sys::EspError> {
    example::main()
}

#[cfg(any(esp32h2, esp32h4, esp32p4))]
fn main() -> Result<(), esp_idf_svc::sys::EspError> {
    panic!("ESP32-H2, ESP32-H4 and ESP32-P4 do not have a Wifi radio (but you could enable the esp-wifi-remote component to use them with a WiFi co-processor)");
}

#[cfg(not(any(esp32h2, esp32h4, esp32p4)))]
mod example {
    use esp_idf_svc::sntp;
    use esp_idf_svc::sys::EspError;

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    use log::info;

    pub fn main() -> Result<(), EspError> {
        esp_idf_svc::sys::link_patches();
        esp_idf_svc::log::EspLogger::initialize_default();

        // Keep it around or else the wifi will stop
        let _wifi = wifi_create()?;

        // Keep it around or else the SNTP service will stop
        let _sntp = sntp::EspSntp::new_default()?;
        info!("SNTP initialized");

        loop {
            // To get a better formatting of the time, you can use the `chrono` or `time` Rust crates
            info!("Current time: {:?}", std::time::SystemTime::now());
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    fn wifi_create() -> Result<esp_idf_svc::wifi::EspWifi<'static>, EspError> {
        use esp_idf_svc::eventloop::*;
        use esp_idf_svc::hal::peripherals::Peripherals;
        use esp_idf_svc::nvs::*;
        use esp_idf_svc::wifi::*;

        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let peripherals = Peripherals::take()?;

        let mut esp_wifi = EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs.clone()))?;
        let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sys_loop.clone())?;

        wifi.set_configuration(&Configuration::Client(ClientConfiguration {
            ssid: SSID.try_into().unwrap(),
            password: PASSWORD.try_into().unwrap(),
            ..Default::default()
        }))?;

        wifi.start()?;
        info!("Wifi started");

        wifi.connect()?;
        info!("Wifi connected");

        wifi.wait_netif_up()?;
        info!("Wifi netif up");

        Ok(esp_wifi)
    }
}
