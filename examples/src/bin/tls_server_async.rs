//! Async TLS "Hello, world" HTTP/1.0 server using `EspAsyncTls::negotiate_server`
//! (ESP-IDF >= 5.5.0, mbedTLS).
//!
//! Multiple connections are served concurrently, each by its own task spawned
//! on a local executor.
//!
//! Add your own ssid and password, flash, then point your browser to
//! `https://<esp-ip>` (the demo cert is self-signed, so accept the browser
//! warning), or run:
//!
//! ```text
//! curl -k https://<esp-ip>
//! ```

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(
    not(any(esp32h2, esp32h4, esp32p4)),
    esp_idf_esp_tls_using_mbedtls,
    esp_idf_version_at_least_5_5_0,
))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(any(esp32h2, esp32h4, esp32p4))]
fn main() -> anyhow::Result<()> {
    panic!("ESP32-H2, ESP32-H4 and ESP32-P4 do not have a Wifi radio (but you could enable the esp-wifi-remote component to use them with a WiFi co-processor)");
}

#[cfg(all(
    not(any(esp32h2, esp32h4, esp32p4)),
    not(all(esp_idf_esp_tls_using_mbedtls, esp_idf_version_at_least_5_5_0)),
))]
fn main() -> anyhow::Result<()> {
    panic!(
        "This example requires ESP-IDF ≥ 5.5.0 with the mbedTLS ESP-TLS stack \
         (CONFIG_ESP_TLS_USING_MBEDTLS=y) for EspAsyncTls::negotiate_server."
    );
}

#[cfg(all(
    not(any(esp32h2, esp32h4, esp32p4)),
    esp_idf_esp_tls_using_mbedtls,
    esp_idf_version_at_least_5_5_0,
))]
mod example {
    use core::cell::Cell;
    use core::pin::pin;

    use std::net::{TcpListener, TcpStream};
    use std::os::fd::{AsRawFd, IntoRawFd};
    use std::rc::Rc;
    use std::time::Duration;

    use async_io::Async;

    use futures::executor::{LocalPool, LocalSpawner};
    use futures::future::{select, Either};
    use futures::task::LocalSpawnExt;

    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::sys::EspError;
    use esp_idf_svc::timer::EspTaskTimerService;
    use esp_idf_svc::tls::{EspAsyncTls, ServerConfig, X509};

    use log::{info, warn};

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    const PORT: u16 = 443;

    /// Each TLS session allocates tens of KBs of heap (mbedTLS buffers), so
    /// too many concurrent sessions exhaust the heap and fail with
    /// `MBEDTLS_ERR_SSL_ALLOC_FAILED` (-0x7F00); connections beyond the cap
    /// are dropped at accept time. Browsers easily hit this by opening
    /// several speculative sockets at once.
    const MAX_SESSIONS: usize = 2;

    /// Drop peers that never finish the handshake. `tls_handshake_timeout_ms`
    /// is only honored by the blocking `EspTls::negotiate_server`, so the async
    /// handshake is raced against a timer instead.
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

    // Demo self-signed cert/key. Generate your own with:
    //   openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem \
    //       -days 3650 -nodes -subj "/CN=esp32.local"
    // Trailing "\0": ESP-TLS expects NUL-terminated PEM.
    const SERVER_CERT: &[u8] = concat!(include_str!("tls_server_cert.pem"), "\0").as_bytes();
    const SERVER_KEY: &[u8] = concat!(include_str!("tls_server_key.pem"), "\0").as_bytes();

    /// The canned response served to every request. HTTP/1.0 with
    /// `Connection: close`, so the connection is simply closed after the
    /// response and no keep-alive/chunked machinery is necessary.
    const RESPONSE: &[u8] = b"HTTP/1.0 200 OK\r\n\
        Connection: close\r\n\
        Content-Type: text/plain\r\n\
        Content-Length: 13\r\n\
        \r\n\
        Hello, world!";

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        // This thread is necessary because the ESP IDF main task thread is running with a very low priority that cannot be raised
        // (lower than the hidden posix thread in `async-io-mini`)
        // As a result, the main thread is constantly starving because of the higher prio `async-io-mini` thread
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
    }

    fn run_main() -> anyhow::Result<()> {
        // `async-io-mini` uses the ESP IDF `eventfd` syscall to implement async IO.
        // If you use `tokio`, you still have to do the same as it also uses the `eventfd` syscall
        let _mounted_eventfs = esp_idf_svc::io::vfs::MountedEventfs::mount(5)?;

        // A local executor rather than a plain `block_on`, so that each accepted
        // connection can be served concurrently by its own spawned task
        let mut pool = LocalPool::new();
        let spawner = pool.spawner();

        pool.run_until(async move {
            let wifi = wifi_create().await?; // Keep it around so that the wifi connection is not dropped

            let ip_info = wifi.sta_netif().get_ip_info()?;

            run(&spawner, ip_info.ip).await
        })
    }

    async fn run(spawner: &LocalSpawner, ip: impl core::fmt::Display) -> anyhow::Result<()> {
        let listener = Async::<TcpListener>::bind(([0, 0, 0, 0], PORT))?;

        info!(
            "TLS Hello-World server on port {PORT}; point your browser to https://{ip}:{PORT} \
             (accept the self-signed cert warning) or run `curl -k https://{ip}:{PORT}`"
        );

        // All tasks run on the same thread, so a plain `Rc<Cell>` is enough
        // to track the number of active sessions
        let active = Rc::new(Cell::new(0));

        loop {
            let (stream, peer) = listener.accept().await?;

            if active.get() >= MAX_SESSIONS {
                warn!("Too many sessions, dropping {peer}");
                continue;
            }

            info!("Accepted {peer}");

            active.set(active.get() + 1);

            let active = active.clone();

            spawner.spawn_local(async move {
                if let Err(e) = handle(stream).await {
                    warn!("Connection to {peer} failed: {e:?}");
                }

                active.set(active.get() - 1);
            })?;
        }
    }

    async fn handle(stream: Async<TcpStream>) -> anyhow::Result<()> {
        let mut tls = EspAsyncTls::adopt(EspTlsSocket::new(stream))?;

        let cfg = ServerConfig {
            server_cert: Some(X509::pem_until_nul(SERVER_CERT)),
            server_key: Some(X509::pem_until_nul(SERVER_KEY)),
            ..ServerConfig::new()
        };

        let mut timer = EspTaskTimerService::new()?.timer_async()?;

        {
            let tls_task = pin!(tls.negotiate_server(&cfg));
            let timer_task = pin!(timer.after(HANDSHAKE_TIMEOUT));

            match select(tls_task, timer_task).await {
                Either::Left((res, _)) => res?,
                Either::Right(_) => anyhow::bail!("Handshake timed out"),
            }
        }

        info!("Handshake complete");

        // Read the request up to the end of its headers (an empty line);
        // this demo replies the same to every request, so the contents are ignored.
        // Requests with a body (e.g. POST) are not handled, as the body would
        // arrive after the headers and is simply never read.
        let mut buf = [0; 1024];
        let mut len = 0;

        loop {
            let n = tls.read(&mut buf[len..]).await?;
            if n == 0 {
                info!("Peer closed the connection mid-request");
                return Ok(());
            }

            len += n;

            if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }

            if len == buf.len() {
                // Request headers bigger than our buffer; just respond anyway
                break;
            }
        }

        tls.write_all(RESPONSE).await?;

        info!("Request served");

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
