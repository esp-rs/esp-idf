//! Simple websocket client example.
//!
//! This example connects to a websocket server that echoes messages,
//! sends a message, receives the same message, and closes the connection.
//!
//! See the comment below as to how to build the example with ESP IDF 5+.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(not(any(esp32h2, esp32h4, esp32p4)))]
fn main() {
    example::main()
}

#[cfg(any(esp32h2, esp32h4, esp32p4))]
fn main() {
    panic!("ESP32-H2, ESP32-H4 and ESP32-P4 do not have a Wifi radio (but you could enable the esp-wifi-remote component to use them with a WiFi co-processor)");
}

#[cfg(not(any(esp32h2, esp32h4, esp32p4)))]
mod example {
    pub fn main() {
        #[cfg(esp_idf_version_major = "4")]
        example::main().unwrap();

        // Note that the ESP IDF websocket client IS available on ESP IDF >= 5 too
        // It is just that it is now an external component, so to use it, you need
        // to put the following snippet at the end of the `Cargo.toml` file of your binary crate:
        //
        // ```toml
        // [[package.metadata.esp-idf-sys.extra_components]]
        // remote_component = { name = "espressif/esp_websocket_client", version = "1.1.0" }
        // ```
        #[cfg(not(esp_idf_version_major = "4"))]
        panic!("This example only compiles on ESP IDF 4. Check the comments in the example how to compile it on ESP IDF 5+")
    }

    #[cfg(esp_idf_version_major = "4")]
    pub mod example {
        use core::time::Duration;

        use esp_idf_svc::eventloop::EspSystemEventLoop;
        use esp_idf_svc::hal::peripherals::Peripherals;
        use esp_idf_svc::io::EspIOError;
        use esp_idf_svc::log::EspLogger;
        use esp_idf_svc::nvs::EspDefaultNvsPartition;
        use esp_idf_svc::tls::X509;
        use esp_idf_svc::wifi::*;
        use esp_idf_svc::ws::client::{
            EspWebSocketClient, EspWebSocketClientConfig, FrameType, WebSocketEvent,
            WebSocketEventType,
        };

        use log::info;

        use std::sync::mpsc;

        const SSID: &str = env!("WIFI_SSID");
        const PASSWORD: &str = env!("WIFI_PASS");
        const ECHO_SERVER_URI: &str = "wss://echo.websocket.org";

        /// The PEM-encoded ISRG Root X1 certificate at the end of the cert chain
        /// for the websocket server at echo.websocket.org.
        const SERVER_ROOT_CERT: &[u8] = b"
    -----BEGIN CERTIFICATE-----
    MIIFazCCA1OgAwIBAgIRAIIQz7DSQONZRGPgu2OCiwAwDQYJKoZIhvcNAQELBQAw
    TzELMAkGA1UEBhMCVVMxKTAnBgNVBAoTIEludGVybmV0IFNlY3VyaXR5IFJlc2Vh
    cmNoIEdyb3VwMRUwEwYDVQQDEwxJU1JHIFJvb3QgWDEwHhcNMTUwNjA0MTEwNDM4
    WhcNMzUwNjA0MTEwNDM4WjBPMQswCQYDVQQGEwJVUzEpMCcGA1UEChMgSW50ZXJu
    ZXQgU2VjdXJpdHkgUmVzZWFyY2ggR3JvdXAxFTATBgNVBAMTDElTUkcgUm9vdCBY
    MTCCAiIwDQYJKoZIhvcNAQEBBQADggIPADCCAgoCggIBAK3oJHP0FDfzm54rVygc
    h77ct984kIxuPOZXoHj3dcKi/vVqbvYATyjb3miGbESTtrFj/RQSa78f0uoxmyF+
    0TM8ukj13Xnfs7j/EvEhmkvBioZxaUpmZmyPfjxwv60pIgbz5MDmgK7iS4+3mX6U
    A5/TR5d8mUgjU+g4rk8Kb4Mu0UlXjIB0ttov0DiNewNwIRt18jA8+o+u3dpjq+sW
    T8KOEUt+zwvo/7V3LvSye0rgTBIlDHCNAymg4VMk7BPZ7hm/ELNKjD+Jo2FR3qyH
    B5T0Y3HsLuJvW5iB4YlcNHlsdu87kGJ55tukmi8mxdAQ4Q7e2RCOFvu396j3x+UC
    B5iPNgiV5+I3lg02dZ77DnKxHZu8A/lJBdiB3QW0KtZB6awBdpUKD9jf1b0SHzUv
    KBds0pjBqAlkd25HN7rOrFleaJ1/ctaJxQZBKT5ZPt0m9STJEadao0xAH0ahmbWn
    OlFuhjuefXKnEgV4We0+UXgVCwOPjdAvBbI+e0ocS3MFEvzG6uBQE3xDk3SzynTn
    jh8BCNAw1FtxNrQHusEwMFxIt4I7mKZ9YIqioymCzLq9gwQbooMDQaHWBfEbwrbw
    qHyGO0aoSCqI3Haadr8faqU9GY/rOPNk3sgrDQoo//fb4hVC1CLQJ13hef4Y53CI
    rU7m2Ys6xt0nUW7/vGT1M0NPAgMBAAGjQjBAMA4GA1UdDwEB/wQEAwIBBjAPBgNV
    HRMBAf8EBTADAQH/MB0GA1UdDgQWBBR5tFnme7bl5AFzgAiIyBpY9umbbjANBgkq
    hkiG9w0BAQsFAAOCAgEAVR9YqbyyqFDQDLHYGmkgJykIrGF1XIpu+ILlaS/V9lZL
    ubhzEFnTIZd+50xx+7LSYK05qAvqFyFWhfFQDlnrzuBZ6brJFe+GnY+EgPbk6ZGQ
    3BebYhtF8GaV0nxvwuo77x/Py9auJ/GpsMiu/X1+mvoiBOv/2X/qkSsisRcOj/KK
    NFtY2PwByVS5uCbMiogziUwthDyC3+6WVwW6LLv3xLfHTjuCvjHIInNzktHCgKQ5
    ORAzI4JMPJ+GslWYHb4phowim57iaztXOoJwTdwJx4nLCgdNbOhdjsnvzqvHu7Ur
    TkXWStAmzOVyyghqpZXjFaH3pO3JLF+l+/+sKAIuvtd7u+Nxe5AW0wdeRlN8NwdC
    jNPElpzVmbUq4JUagEiuTDkHzsxHpFKVK7q4+63SM1N95R1NbdWhscdCb+ZAJzVc
    oyi3B43njTOQ5yOf+1CceWxG1bQVs5ZufpsMljq4Ui0/1lvh+wjChP4kqKOJ2qxq
    4RgqsahDYVvTH9w7jXbyLeiNdd8XM2w9U/t7y0Ff/9yi0GE44Za4rF2LN9d11TPA
    mRGunUHBcnWEvgJBQl9nJEiU0Zsnvgc/ubhPgXRR4Xq37Z0j4r7g1SgEEzwxA57d
    emyPxgcYxn/eR44/KJ4EBs+lVDR3veyJm+kXQ99b21/+jh5Xos1AnX5iItreGCc=
    -----END CERTIFICATE-----\0";

        /// The relevant events for this example as it connects to the server,
        /// sends a message, receives the same message, and closes the connection.
        #[derive(Debug, PartialEq)]
        enum ExampleEvent {
            Connected,
            MessageReceived,
            Closed,
        }

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

            // Connect websocket
            let config = EspWebSocketClientConfig {
                server_cert: Some(X509::pem_until_nul(SERVER_ROOT_CERT)),
                ..Default::default()
            };
            let timeout = Duration::from_secs(10);
            let (tx, rx) = mpsc::channel::<ExampleEvent>();
            let mut client =
                EspWebSocketClient::new(ECHO_SERVER_URI, &config, timeout, move |event| {
                    handle_event(&tx, event)
                })?;
            assert_eq!(rx.recv(), Ok(ExampleEvent::Connected));
            assert!(client.is_connected());

            // Send message and receive it back
            let message = "Hello, World!";
            info!("Websocket send, text: {message}");
            client.send(FrameType::Text(false), message.as_bytes())?;
            assert_eq!(rx.recv(), Ok(ExampleEvent::MessageReceived));

            // Close websocket
            drop(client);
            assert_eq!(rx.recv(), Ok(ExampleEvent::Closed));

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

        fn handle_event(
            tx: &mpsc::Sender<ExampleEvent>,
            event: &Result<WebSocketEvent, EspIOError>,
        ) {
            if let Ok(event) = event {
                match event.event_type {
                    WebSocketEventType::BeforeConnect => {
                        info!("Websocket before connect");
                    }
                    WebSocketEventType::Connected => {
                        info!("Websocket connected");
                        tx.send(ExampleEvent::Connected).ok();
                    }
                    WebSocketEventType::Disconnected => {
                        info!("Websocket disconnected");
                    }
                    WebSocketEventType::Close(reason) => {
                        info!("Websocket close, reason: {reason:?}");
                    }
                    WebSocketEventType::Closed => {
                        info!("Websocket closed");
                        tx.send(ExampleEvent::Closed).ok();
                    }
                    WebSocketEventType::Text(text) => {
                        info!("Websocket recv, text: {text}");
                        if text == "Hello, World!" {
                            tx.send(ExampleEvent::MessageReceived).ok();
                        }
                    }
                    WebSocketEventType::Binary(binary) => {
                        info!("Websocket recv, binary: {binary:?}");
                    }
                    WebSocketEventType::Ping => {
                        info!("Websocket ping");
                    }
                    WebSocketEventType::Pong => {
                        info!("Websocket pong");
                    }
                }
            }
        }
    }
}
