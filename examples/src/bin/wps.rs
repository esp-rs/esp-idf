//! Example of using blocking WPS.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(not(any(esp32h2, esp32h4, esp32p4)))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(any(esp32h2, esp32h4, esp32p4))]
fn main() -> anyhow::Result<()> {
    panic!("ESP32-H2, ESP32-H4 and ESP32-P4 do not have a Wifi radio (but you could enable the esp-wifi-remote component to use them with a WiFi co-processor)");
}

#[cfg(not(any(esp32h2, esp32h4, esp32p4)))]
mod example {
    use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};

    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
    use esp_idf_svc::wifi::{WpsConfig, WpsFactoryInfo, WpsStatus, WpsType};
    use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};

    use log::info;

    const WPS_CONFIG: WpsConfig = WpsConfig {
        wps_type: WpsType::Pbc,
        factory_info: WpsFactoryInfo {
            manufacturer: "ESPRESSIF",
            model_number: "esp32",
            model_name: "ESPRESSIF IOT",
            device_name: "ESP DEVICE",
        },
    };

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let mut wifi = BlockingWifi::wrap(
            EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
            sys_loop,
        )?;

        wifi.start()?;
        info!("Wifi started");

        match wifi.start_wps(&WPS_CONFIG)? {
            WpsStatus::SuccessConnected => (),
            WpsStatus::SuccessMultipleAccessPoints(credentials) => {
                log::info!("received multiple credentials, connecting to first one:");
                for i in &credentials {
                    log::info!(" - ssid: {}", i.ssid);
                }
                let wifi_configuration: Configuration =
                    Configuration::Client(ClientConfiguration {
                        ssid: credentials[0].ssid.clone(),
                        bssid: None,
                        auth_method: AuthMethod::WPA2Personal,
                        password: credentials[1].passphrase.clone(),
                        channel: None,
                        ..Default::default()
                    });
                wifi.set_configuration(&wifi_configuration)?;
            }
            WpsStatus::Failure => anyhow::bail!("WPS failure"),
            WpsStatus::Timeout => anyhow::bail!("WPS timeout"),
            WpsStatus::Pin(_) => anyhow::bail!("WPS pin"),
            WpsStatus::PbcOverlap => anyhow::bail!("WPS PBC overlap"),
        }

        match wifi.get_configuration()? {
            Configuration::Client(config) => {
                info!("Successfully connected to {} using WPS", config.ssid)
            }
            _ => anyhow::bail!("Not in station mode"),
        };

        wifi.connect()?;
        info!("Wifi connected");

        wifi.wait_netif_up()?;
        info!("Wifi netif up");

        let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
        info!("Wifi DHCP info: {ip_info:?}");

        info!("Shutting down in 5s...");

        std::thread::sleep(core::time::Duration::from_secs(5));

        Ok(())
    }
}
