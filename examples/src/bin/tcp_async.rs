//! Simple examples of an asynchronous TCP client communicating with an internet TCP server
//! (google.com) and of an asynchronous TCP server, that listens for incoming data and echoes it back.

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
    use std::io;
    use std::net::{TcpListener, TcpStream, ToSocketAddrs};

    use async_io::Async;

    use futures::executor::{LocalPool, LocalSpawner};
    use futures::task::LocalSpawnExt;
    use futures::{AsyncReadExt, AsyncWriteExt, FutureExt};

    use esp_idf_svc::sys::EspError;
    use esp_idf_svc::timer::EspTaskTimerService;

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    use log::{error, info};

    pub fn main() -> Result<(), anyhow::Error> {
        esp_idf_svc::sys::link_patches();
        esp_idf_svc::log::EspLogger::initialize_default();

        // `async-io` uses the ESP IDF `eventfd` syscall to implement async IO.
        // If you use `tokio`, you still have to do the same as it also uses the `eventfd` syscall
        let _mounted_eventfs = esp_idf_svc::io::vfs::MountedEventfs::mount(5)?;

        // This thread is necessary because the ESP IDF main task thread is running with a very low priority that cannot be raised
        // (lower than the hidden posix thread in `async-io`)
        // As a result, the main thread is constantly starving because of the higher prio `async-io` thread
        //
        // To use async networking IO, make your `main()` minimal by just spawning all work in a new thread
        std::thread::Builder::new()
            .stack_size(60000)
            .spawn(run_main)
            .unwrap()
            .join()
            .unwrap()
            .unwrap();

        Ok(())
    }

    fn run_main() -> Result<(), anyhow::Error> {
        // Any executor would do. We just use the local executor from the `futures` crate
        // As for why we need an executor - just for a simple way to spawn the accepted connections
        // in the `tcp_server` server
        let mut local_executor = LocalPool::new();
        let spawner = local_executor.spawner();

        local_executor.spawner().spawn_local(
            async move {
                // Keep it around or else the wifi will stop
                let _wifi = wifi_create().await?;

                tcp_client().await?;
                tcp_server(spawner).await?;

                Result::<_, anyhow::Error>::Ok(())
            }
            .map(Result::unwrap),
        )?;

        local_executor.run();

        Ok(())
    }

    async fn tcp_client() -> Result<(), io::Error> {
        info!("About to open a TCP connection to 1.1.1.1 port 80");

        let addr = "one.one.one.one:80".to_socket_addrs()?.next().unwrap();
        let mut stream = Async::<TcpStream>::connect(addr).await?;

        stream.write_all("GET / HTTP/1.0\n\n".as_bytes()).await?;

        let mut result = Vec::new();

        stream.read_to_end(&mut result).await?;

        info!(
            "1.1.1.1 returned:\n=================\n{}\n=================\nSince it returned something, all is OK",
            std::str::from_utf8(&result).map_err(|_| io::ErrorKind::InvalidData)?);

        Ok(())
    }

    async fn tcp_server(spawner: LocalSpawner) -> Result<(), io::Error> {
        async fn accept(spawner: LocalSpawner) -> Result<(), io::Error> {
            info!("About to bind a simple echo service to port 8080; do `telnet <ip-from-above>:8080`");

            let addr = "0.0.0.0:8080".to_socket_addrs()?.next().unwrap();
            let listener = Async::<TcpListener>::bind(addr)?;

            loop {
                let stream = listener.accept().await;
                match stream {
                    Ok((stream, addr)) => {
                        info!("Accepted client {addr}");

                        spawner.spawn_local(handle(stream)).unwrap();
                    }
                    Err(e) => {
                        error!("Error: {e}");
                    }
                }
            }
        }

        async fn handle(mut stream: Async<TcpStream>) {
            // read 128 bytes at a time from stream echoing back to stream
            loop {
                let mut read = [0; 128];

                match stream.read(&mut read).await {
                    Ok(n) => {
                        if n == 0 {
                            // connection was closed
                            break;
                        }

                        let _ = stream.write_all(&read[0..n]).await;
                    }
                    Err(err) => {
                        panic!("{}", err);
                    }
                }
            }
        }

        accept(spawner).await
    }

    async fn wifi_create() -> Result<esp_idf_svc::wifi::EspWifi<'static>, EspError> {
        use esp_idf_svc::eventloop::*;
        use esp_idf_svc::hal::peripherals::Peripherals;
        use esp_idf_svc::nvs::*;
        use esp_idf_svc::wifi::*;

        let sys_loop = EspSystemEventLoop::take()?;
        let timer_service = EspTaskTimerService::new()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let peripherals = Peripherals::take()?;

        let mut esp_wifi = EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs.clone()))?;
        let mut wifi = AsyncWifi::wrap(&mut esp_wifi, sys_loop.clone(), timer_service)?;

        wifi.set_configuration(&Configuration::Client(ClientConfiguration {
            ssid: SSID.try_into().unwrap(),
            password: PASSWORD.try_into().unwrap(),
            ..Default::default()
        }))?;

        wifi.start().await?;
        info!("Wifi started");

        wifi.connect().await?;
        info!("Wifi connected");

        wifi.wait_netif_up().await?;
        info!("Wifi netif up");

        Ok(esp_wifi)
    }
}
