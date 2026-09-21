//! Simple mDNS advertise example.
//!
//! See the comment below as to how to build the example with ESP IDF 5+.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

pub fn main() {
    #[cfg(esp_idf_version_major = "4")]
    example::main().unwrap();

    // Note that ESP IDF mDNS IS available on ESP IDF >= 5 too
    // It is just that it is now an external component, so to use it, you need
    // to put the following snippet at the end of the `Cargo.toml` file of your binary crate:
    //
    // ```toml
    // [[package.metadata.esp-idf-sys.extra_components]]
    // remote_component = { name = "espressif/mdns", version = "1.8.2" }
    // ```
    #[cfg(not(esp_idf_version_major = "4"))]
    panic!("This example only compiles on ESP IDF 4. Check the comments in the example how to compile it on ESP IDF 5+")
}

#[cfg(esp_idf_version_major = "4")]
pub mod example {
    use esp_idf_hal::io::Write;
    use esp_idf_svc::eventloop::EspSystemEventLoop;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::http::server::EspHttpServer;
    use esp_idf_svc::http::Method;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::mdns::EspMdns;
    use esp_idf_svc::nvs::EspDefaultNvsPartition;
    use esp_idf_svc::wifi::*;

    use log::info;

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");
    static INDEX_HTML: &str = include_str!("http_server_page.html");

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        // Setup Wifi
        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;
        let mut wifi = BlockingWifi::wrap(
            EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
            sys_loop,
        )?;
        connect_wifi(&mut wifi)?;

        // Setup HTTP server
        let server_config = esp_idf_svc::http::server::Configuration::default();
        let mut server = EspHttpServer::new(&server_config)?;
        server.fn_handler("/", Method::Get, |req| {
            req.into_ok_response()?
                .write_all(INDEX_HTML.as_bytes())
                .map(|_| ())
        })?;

        // Setup mDNS
        let mut mdns = EspMdns::take()?;
        mdns.set_hostname("esp-advertiser")?;

        // Advertise the HTTP server
        mdns.add_service(
            Some("ESP HTTP Server"),
            "_http",
            "_tcp",
            server_config.http_port,
            &[],
        )?;

        // Keep the wifi, http server, and mDNS running beyond when main() returns (forever)
        // Do not call this if you ever want to stop or access them later.
        // Otherwise you can either add an infinite loop so the main task
        // never returns, or you can move them to another thread.
        // https://doc.rust-lang.org/stable/core/mem/fn.forget.html
        core::mem::forget(wifi);
        core::mem::forget(server);
        core::mem::forget(mdns);

        Ok(())
    }

    fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
        let wifi_configuration: Configuration = Configuration::Client(ClientConfiguration {
            ssid: SSID.try_into().unwrap(),
            bssid: None,
            auth_method: AuthMethod::WPA2Personal,
            password: PASSWORD.try_into().unwrap(),
            channel: None,
            ..Default::default()
        });

        wifi.set_configuration(&wifi_configuration)?;

        wifi.start()?;
        info!("Wifi started");

        wifi.connect()?;
        info!("Wifi connected");

        wifi.wait_netif_up()?;
        info!("Wifi netif up");

        Ok(())
    }
}
