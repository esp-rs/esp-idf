//! Simple examples of a blocking TCP client communicating with an internet TCP server
//! (google.com) and of a blocking TCP server, that listens for incoming data and echoes it back.

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
    use std::env;
    use std::io::{self, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    use esp_idf_svc::sys::EspError;

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    use log::{error, info};

    pub fn main() -> Result<(), anyhow::Error> {
        esp_idf_svc::sys::link_patches();
        esp_idf_svc::log::EspLogger::initialize_default();

        // Keep it around or else the wifi will stop
        let _wifi = wifi_create()?;

        tcp_client()?;

        tcp_server()?;

        Ok(())
    }

    fn tcp_client() -> Result<(), io::Error> {
        info!("About to open a TCP connection to 1.1.1.1 port 80");

        let mut stream = TcpStream::connect("one.one.one.one:80")?;

        let err = stream.try_clone();
        if let Err(err) = err {
            info!(
                "Duplication of file descriptors does not work (yet) on the ESP-IDF, as expected: {err}"
            );
        }

        stream.write_all("GET / HTTP/1.0\n\n".as_bytes())?;

        let mut result = Vec::new();

        stream.read_to_end(&mut result)?;

        info!(
            "1.1.1.1 returned:\n=================\n{}\n=================\nSince it returned something, all is OK",
            std::str::from_utf8(&result).map_err(|_| io::ErrorKind::InvalidData)?);

        Ok(())
    }

    fn tcp_server() -> Result<(), io::Error> {
        fn accept() -> Result<(), io::Error> {
            info!("About to bind a simple echo service to port 8080; do `telnet <ip-from-above>:8080`");

            let listener = TcpListener::bind("0.0.0.0:8080")?;

            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        info!("Accepted client");

                        thread::spawn(move || {
                            handle(stream);
                        });
                    }
                    Err(e) => {
                        error!("Error: {e}");
                    }
                }
            }

            unreachable!()
        }

        fn handle(mut stream: TcpStream) {
            // Read 128 bytes at a time from stream echoing back to stream
            loop {
                let mut read = [0; 128];

                match stream.read(&mut read) {
                    Ok(n) => {
                        if n == 0 {
                            // connection was closed
                            break;
                        }

                        let _ = stream.write_all(&read[0..n]);
                    }
                    Err(err) => {
                        panic!("{}", err);
                    }
                }
            }
        }

        accept()
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
