//! HTTP Server with JSON POST handler
//!
//! Go to 192.168.71.1 to test

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

    use embedded_svc::{
        http::{Headers, Method},
        io::{Read, Write},
        wifi::{self, AccessPointConfiguration, AuthMethod},
    };

    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::{
        eventloop::EspSystemEventLoop,
        http::server::EspHttpServer,
        nvs::EspDefaultNvsPartition,
        wifi::{BlockingWifi, EspWifi},
    };

    use log::*;

    use serde::Deserialize;

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");
    static INDEX_HTML: &str = include_str!("http_server_page.html");

    // Max payload length
    const MAX_LEN: usize = 128;

    // Need lots of stack to parse JSON
    const STACK_SIZE: usize = 10240;

    // Wi-Fi channel, between 1 and 11
    const CHANNEL: u8 = 11;

    #[derive(Deserialize)]
    struct FormData<'a> {
        first_name: &'a str,
        age: u32,
        birthplace: &'a str,
    }

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        esp_idf_svc::log::EspLogger::initialize_default();

        // Setup Wifi

        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let mut wifi = BlockingWifi::wrap(
            EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
            sys_loop,
        )?;

        connect_wifi(&mut wifi)?;

        let mut server = create_server()?;

        server.fn_handler("/", Method::Get, |req| {
            req.into_ok_response()?
                .write_all(INDEX_HTML.as_bytes())
                .map(|_| ())
        })?;

        server.fn_handler::<anyhow::Error, _>("/post", Method::Post, |mut req| {
            let len = req.content_len().unwrap_or(0) as usize;

            if len > MAX_LEN {
                req.into_status_response(413)?
                    .write_all("Request too big".as_bytes())?;
                return Ok(());
            }

            let mut buf = vec![0; len];
            req.read_exact(&mut buf)?;
            let mut resp = req.into_ok_response()?;

            if let Ok(form) = serde_json::from_slice::<FormData>(&buf) {
                write!(
                    resp,
                    "Hello, {}-year-old {} from {}!",
                    form.age, form.first_name, form.birthplace
                )?;
            } else {
                resp.write_all("JSON error".as_bytes())?;
            }

            Ok(())
        })?;

        // Keep wifi and the server running beyond when main() returns (forever)
        // Do not call this if you ever want to stop or access them later.
        // Otherwise you can either add an infinite loop so the main task
        // never returns, or you can move them to another thread.
        // https://doc.rust-lang.org/stable/core/mem/fn.forget.html
        core::mem::forget(wifi);
        core::mem::forget(server);

        // Main task no longer needed, free up some memory
        Ok(())
    }

    fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
        // If instead of creating a new network you want to serve the page
        // on your local network, you can replace this configuration with
        // the client configuration from the http_client example.
        let wifi_configuration = wifi::Configuration::AccessPoint(AccessPointConfiguration {
            ssid: SSID.try_into().unwrap(),
            ssid_hidden: true,
            auth_method: AuthMethod::WPA2Personal,
            password: PASSWORD.try_into().unwrap(),
            channel: CHANNEL,
            ..Default::default()
        });

        wifi.set_configuration(&wifi_configuration)?;

        wifi.start()?;
        info!("Wifi started");

        // If using a client configuration you need
        // to connect to the network with:
        //
        //  ```
        //  wifi.connect()?;
        //  info!("Wifi connected");
        // ```

        wifi.wait_netif_up()?;
        info!("Wifi netif up");

        info!("Created Wi-Fi with WIFI_SSID `{SSID}` and WIFI_PASS `{PASSWORD}`");

        Ok(())
    }

    fn create_server() -> anyhow::Result<EspHttpServer<'static>> {
        let server_configuration = esp_idf_svc::http::server::Configuration {
            stack_size: STACK_SIZE,
            ..Default::default()
        };

        Ok(EspHttpServer::new(&server_configuration)?)
    }
}
