//! Example of using asynchronous TLS/TCP.
//!
//! Add your own ssid and password

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(not(any(esp32h2, esp32h4, esp32p4)))]
#[cfg(not(esp_idf_version_major = "4"))]
fn main() {
    example::main()
}

#[cfg(any(esp32h2, esp32h4, esp32p4))]
fn main() {
    panic!("ESP32-H2, ESP32-H4 and ESP32-P4 do not have a Wifi radio (but you could enable the esp-wifi-remote component to use them with a WiFi co-processor)");
}

#[cfg(esp_idf_version_major = "4")]
fn main() {
    panic!("This example requires ESP IDF >= 5");
}

#[cfg(not(any(esp32h2, esp32h4, esp32p4)))]
#[cfg(not(esp_idf_version_major = "4"))]
mod example {
    use core::pin::pin;

    use std::net::{TcpStream, ToSocketAddrs};
    use std::os::fd::{AsRawFd, IntoRawFd};

    use async_io::Async;

    use esp_idf_svc::io;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::sys::EspError;

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    use log::info;

    // Don't forget to raise the CONFIG_PTHREAD_TASK_STACK_SIZE_DEFAULT in `sdkconfig.defaults` to > 4K so that the
    // `async-io` background thread can work fine
    pub fn main() {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        // This thread is necessary because the ESP IDF main task thread is running with a very low priority that cannot be raised
        // (lower than the hidden posix thread in `async-io`)
        // As a result, the main thread is constantly starving because of the higher prio `async-io` thread
        //
        // To use async networking IO, make your `main()` minimal by just spawning all work in a new thread
        std::thread::Builder::new()
            // Enough for the mbedTLS handshake processing; the large TLS buffers
            // live on the heap. Note that on ESP-IDF thread stacks are allocated
            // from the heap too, so an over-sized stack directly reduces the
            // memory available for TLS sessions
            .stack_size(20000)
            .spawn(run_main)
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
    }

    fn run_main() -> anyhow::Result<()> {
        // `async-io` uses the ESP IDF `eventfd` syscall to implement async IO.
        // If you use `tokio`, you still have to do the same as it also uses the `eventfd` syscall
        let _mounted_eventfs = esp_idf_svc::io::vfs::MountedEventfs::mount(5)?;

        // Any executor can drive the future; we just use the ESP-IDF-native `block_on`
        esp_idf_svc::hal::task::block_on(pin!(async move {
            let _wifi = wifi_create().await?; // Keep it around so that the wifi connection is not dropped

            run().await?;

            Result::<_, anyhow::Error>::Ok(())
        }))
    }

    async fn run() -> anyhow::Result<()> {
        let addr = "google.com:443".to_socket_addrs()?.next().unwrap();
        let socket = Async::<TcpStream>::connect(addr).await.unwrap();

        info!("Opened a plain socket to google.com:443");

        let mut tls = esp_idf_svc::tls::EspAsyncTls::adopt(EspTlsSocket::new(socket)).unwrap();

        info!("Async TLS socket created");

        tls.negotiate("google.com", &esp_idf_svc::tls::Config::new())
            .await
            .unwrap();

        info!("TLS negotiation successful, doing a simple HTTPS GET request");

        tls.write_all(b"GET / HTTP/1.0\r\n\r\n").await?;

        let mut body = [0_u8; 3048];

        let read = io::utils::asynch::try_read_full(&mut tls, &mut body)
            .await
            .map_err(|(e, _)| e)?;

        info!(
            "Body (truncated to 3K):\n{:?}",
            String::from_utf8_lossy(&body[..read]).into_owned()
        );

        Ok(())
    }

    async fn wifi_create() -> Result<esp_idf_svc::wifi::EspWifi<'static>, EspError> {
        use esp_idf_svc::eventloop::*;
        use esp_idf_svc::hal::peripherals::Peripherals;
        use esp_idf_svc::nvs::*;
        use esp_idf_svc::timer::*;
        use esp_idf_svc::wifi::*;

        let sys_loop = EspSystemEventLoop::take().unwrap();
        let timer_service = EspTimerService::new().unwrap();
        let nvs = EspDefaultNvsPartition::take().unwrap();

        let peripherals = Peripherals::take()?;

        let mut esp_wifi = EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs.clone()))?;
        let mut wifi = AsyncWifi::wrap(&mut esp_wifi, sys_loop.clone(), timer_service.clone())?;

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

    //
    // Implement `esp_idf_svc::tls::PollableSocket` for `async-io` sockets
    // you can do the same for e.g. `tokio` if you plan to use `tokio` instead of `async-io`
    //

    pub struct EspTlsSocket(Option<async_io::Async<TcpStream>>);

    impl EspTlsSocket {
        pub const fn new(socket: async_io::Async<TcpStream>) -> Self {
            Self(Some(socket))
        }

        pub fn handle(&self) -> i32 {
            self.0.as_ref().unwrap().as_raw_fd()
        }

        pub fn poll_readable(
            &self,
            ctx: &mut core::task::Context,
        ) -> core::task::Poll<Result<(), esp_idf_svc::sys::EspError>> {
            self.0
                .as_ref()
                .unwrap()
                .poll_readable(ctx)
                .map_err(|_| EspError::from_infallible::<{ esp_idf_svc::sys::ESP_FAIL }>())
        }

        pub fn poll_writeable(
            &self,
            ctx: &mut core::task::Context,
        ) -> core::task::Poll<Result<(), esp_idf_svc::sys::EspError>> {
            self.0
                .as_ref()
                .unwrap()
                .poll_writable(ctx)
                .map_err(|_| EspError::from_infallible::<{ esp_idf_svc::sys::ESP_FAIL }>())
        }

        fn release(&mut self) -> Result<(), esp_idf_svc::sys::EspError> {
            let socket = self.0.take().unwrap();
            let _ = socket.into_inner().unwrap().into_raw_fd();

            Ok(())
        }
    }

    impl esp_idf_svc::tls::Socket for EspTlsSocket {
        fn handle(&self) -> i32 {
            EspTlsSocket::handle(self)
        }

        fn release(&mut self) -> Result<(), esp_idf_svc::sys::EspError> {
            EspTlsSocket::release(self)
        }
    }

    impl esp_idf_svc::tls::PollableSocket for EspTlsSocket {
        fn poll_readable(
            &self,
            ctx: &mut core::task::Context,
        ) -> core::task::Poll<Result<(), esp_idf_svc::sys::EspError>> {
            EspTlsSocket::poll_readable(self, ctx)
        }

        fn poll_writable(
            &self,
            ctx: &mut core::task::Context,
        ) -> core::task::Poll<Result<(), esp_idf_svc::sys::EspError>> {
            EspTlsSocket::poll_writeable(self, ctx)
        }
    }
}
