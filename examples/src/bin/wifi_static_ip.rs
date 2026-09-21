//! Example of using blocking wifi with a static IP configuration
//!
//! Add your own ssid and password for the access point
//! Add your own gateway IP, netmask, and local device IP for interface configuration

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
    use core::convert::TryInto;

    use std::net::Ipv4Addr;
    use std::str::FromStr;

    use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration as WifiConfiguration};

    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::ipv4::{
        ClientConfiguration as IpClientConfiguration, ClientSettings as IpClientSettings,
        Configuration as IpConfiguration, Mask, Subnet,
    };
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::netif::{EspNetif, NetifConfiguration, NetifStack};
    use esp_idf_svc::wifi::{BlockingWifi, EspWifi, WifiDriver};
    use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};

    use log::info;

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    // Expects IPv4 address
    const DEVICE_IP: &str = env!("ESP_DEVICE_IP");
    // Expects IPv4 address
    const GATEWAY_IP: &str = env!("GATEWAY_IP");
    // Expects a number between 0 and 32, defaults to 24
    const GATEWAY_NETMASK: Option<&str> = option_env!("GATEWAY_NETMASK");

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let wifi = WifiDriver::new(peripherals.modem, sys_loop.clone(), Some(nvs))?;
        let wifi = configure_wifi(wifi)?;

        let mut wifi = BlockingWifi::wrap(wifi, sys_loop)?;
        connect_wifi(&mut wifi)?;

        let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

        info!("Wifi Interface info: {ip_info:?}");

        info!("Shutting down in 5s...");

        std::thread::sleep(core::time::Duration::from_secs(5));

        Ok(())
    }

    fn configure_wifi(wifi: WifiDriver) -> anyhow::Result<EspWifi> {
        let netmask = GATEWAY_NETMASK.unwrap_or("24");
        let netmask = u8::from_str(netmask)?;
        let gateway_addr = Ipv4Addr::from_str(GATEWAY_IP)?;
        let static_ip = Ipv4Addr::from_str(DEVICE_IP)?;

        let mut wifi = EspWifi::wrap_all(
            wifi,
            EspNetif::new_with_conf(&NetifConfiguration {
                ip_configuration: Some(IpConfiguration::Client(IpClientConfiguration::Fixed(
                    IpClientSettings {
                        ip: static_ip,
                        subnet: Subnet {
                            gateway: gateway_addr,
                            mask: Mask(netmask),
                        },
                        // Can also be set to Ipv4Addrs if you need DNS
                        dns: None,
                        secondary_dns: None,
                    },
                ))),
                ..NetifConfiguration::wifi_default_client()
            })?,
            #[cfg(esp_idf_esp_wifi_softap_support)]
            EspNetif::new(NetifStack::Ap)?,
        )?;

        let wifi_configuration = WifiConfiguration::Client(ClientConfiguration {
            ssid: SSID.try_into().unwrap(),
            bssid: None,
            auth_method: AuthMethod::WPA2Personal,
            password: PASSWORD.try_into().unwrap(),
            channel: None,
            ..Default::default()
        });
        wifi.set_configuration(&wifi_configuration)?;

        Ok(wifi)
    }

    fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
        wifi.start()?;
        info!("Wifi started");

        wifi.connect()?;
        info!("Wifi connected");

        wifi.wait_netif_up()?;
        info!("Wifi netif up");

        Ok(())
    }
}
