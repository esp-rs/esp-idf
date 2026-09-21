//! Simple HTTP client example.

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
        http::{client::Client as HttpClient, Method},
        io::Write,
        utils::io,
        wifi::{AuthMethod, ClientConfiguration, Configuration},
    };

    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::http::client::EspHttpConnection;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
    use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};

    use log::{error, info};

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

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

        // Create HTTP client
        //
        // Note: To send a request to an HTTPS server, you can do:
        //
        // ```
        // use esp_idf_svc::http::client::{Configuration as HttpConfiguration, EspHttpConnection};
        //
        // let config = &HttpConfiguration {
        //     crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        //     ..Default::default()
        // };
        //
        // let mut client = HttpClient::wrap(EspHttpConnection::new(&config)?);
        // ```
        let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);

        // GET
        get_request(&mut client)?;

        // POST
        post_request(&mut client)?;

        // POST chunked
        post_chunked_request(&mut client)?;

        Ok(())
    }

    /// Send an HTTP GET request.
    fn get_request(client: &mut HttpClient<EspHttpConnection>) -> anyhow::Result<()> {
        // Prepare headers and URL
        let headers = [("accept", "text/plain")];
        let url = "http://ifconfig.net/";

        // Send request
        //
        // Note: If you don't want to pass in any headers, you can also use `client.get(url, headers)`.
        let request = client.request(Method::Get, url, &headers)?;
        info!("-> GET {url}");
        let mut response = request.submit()?;

        // Process response
        let status = response.status();
        info!("<- {status}");
        let mut buf = [0u8; 1024];
        let bytes_read = io::try_read_full(&mut response, &mut buf).map_err(|e| e.0)?;
        info!("Read {bytes_read} bytes");
        match std::str::from_utf8(&buf[0..bytes_read]) {
            Ok(body_string) => info!(
                "Response body (truncated to {} bytes): {:?}",
                buf.len(),
                body_string
            ),
            Err(e) => error!("Error decoding response body: {e}"),
        };

        Ok(())
    }

    /// Send an HTTP POST request.
    fn post_request(client: &mut HttpClient<EspHttpConnection>) -> anyhow::Result<()> {
        // Prepare payload
        let payload = b"Hello world!";

        // Prepare headers and URL
        let content_length_header = format!("{}", payload.len());
        let headers = [
            ("content-type", "text/plain"),
            ("content-length", &*content_length_header),
        ];
        let url = "http://httpbin.org/post";

        // Send request
        let mut request = client.post(url, &headers)?;
        request.write_all(payload)?;
        request.flush()?;
        info!("-> POST {url}");
        let mut response = request.submit()?;

        // Process response
        let status = response.status();
        info!("<- {status}");
        let mut buf = [0u8; 1024];
        let bytes_read = io::try_read_full(&mut response, &mut buf).map_err(|e| e.0)?;
        info!("Read {bytes_read} bytes");
        match std::str::from_utf8(&buf[0..bytes_read]) {
            Ok(body_string) => info!(
                "Response body (truncated to {} bytes): {body_string:?}",
                buf.len()
            ),
            Err(e) => error!("Error decoding response body: {e}"),
        };

        Ok(())
    }

    /// Send an HTTP POST request using chunked transfer encoding.
    fn post_chunked_request(client: &mut HttpClient<EspHttpConnection>) -> anyhow::Result<()> {
        // Prepare payload
        let payload1 = b"Hello world!";
        let payload2 = b"From Rust!";

        // Prepare headers and URL
        let headers = [("content-type", "text/plain")];
        let url = "http://httpbin.org/post";

        // Send request
        let mut request = client.post(url, &headers)?;
        request.write_all(payload1)?;
        request.write_all(payload2)?;
        request.flush()?;
        info!("-> CHUNKED POST {url}");
        let mut response = request.submit()?;

        // Process response
        let status = response.status();
        info!("<- {status}");
        let mut buf = [0u8; 1024];
        let bytes_read = io::try_read_full(&mut response, &mut buf).map_err(|e| e.0)?;
        info!("Read {bytes_read} bytes");
        match std::str::from_utf8(&buf[0..bytes_read]) {
            Ok(body_string) => info!(
                "Response body (truncated to {} bytes): {body_string:?}",
                buf.len()
            ),
            Err(e) => error!("Error decoding response body: {e}"),
        };

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
