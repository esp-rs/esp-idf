//! HTTP/WebSocket Server with contexts
//!
//! Go to http://192.168.71.1 to play

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
    use core::cmp::Ordering;
    use core::convert::TryInto;

    use embedded_svc::{
        http::Method,
        io::Write,
        wifi::{self, AccessPointConfiguration, AuthMethod},
        ws::FrameType,
    };

    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::{
        eventloop::EspSystemEventLoop,
        http::server::EspHttpServer,
        nvs::EspDefaultNvsPartition,
        systime::EspSystemTime,
        wifi::{BlockingWifi, EspWifi},
    };

    use esp_idf_svc::sys::{EspError, ESP_ERR_INVALID_SIZE};

    use log::*;

    use std::{borrow::Cow, collections::BTreeMap, ffi::CStr, str, sync::Mutex};

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    static INDEX_HTML: &str = include_str!("http_ws_server_page.html");

    // Max payload length
    const MAX_LEN: usize = 8;

    // Need lots of stack to parse JSON
    const STACK_SIZE: usize = 10240;

    // Wi-Fi channel, between 1 and 11
    const CHANNEL: u8 = 11;

    struct GuessingGame {
        guesses: u32,
        secret: u32,
        done: bool,
    }

    impl GuessingGame {
        fn new(secret: u32) -> Self {
            Self {
                guesses: 0,
                secret,
                done: false,
            }
        }

        fn guess(&mut self, guess: u32) -> (Ordering, u32) {
            if self.done {
                (Ordering::Equal, self.guesses)
            } else {
                self.guesses += 1;
                let cmp = guess.cmp(&self.secret);
                if cmp == Ordering::Equal {
                    self.done = true;
                }
                (cmp, self.guesses)
            }
        }

        fn parse_guess(input: &str) -> Option<u32> {
            // Trim control codes (including null bytes) and/or whitespace
            let Ok(number) = input
                .trim_matches(|c: char| c.is_ascii_control() || c.is_whitespace())
                .parse::<u32>()
            else {
                warn!("Not a number: `{input}` (length {})", input.len());
                return None;
            };
            if !(1..=100).contains(&number) {
                warn!("Not in range ({number})");
                return None;
            }
            Some(number)
        }
    }

    // Super rudimentary pseudo-random numbers
    fn rand() -> u32 {
        EspSystemTime::now(&EspSystemTime {}).subsec_nanos() / 65537
    }

    // Serialize numbers in English
    fn nth(n: u32) -> Cow<'static, str> {
        match n {
            smaller @ (0..=13) => Cow::Borrowed(match smaller {
                0 => "zeroth",
                1 => "first",
                2 => "second",
                3 => "third",
                4 => "fourth",
                5 => "fifth",
                6 => "sixth",
                7 => "seventh",
                8 => "eighth",
                9 => "ninth",
                10 => "10th",
                11 => "11th",
                12 => "12th",
                13 => "13th",
                _ => unreachable!(),
            }),
            larger => Cow::Owned(match larger % 10 {
                1 => format!("{larger}st"),
                2 => format!("{larger}nd"),
                3 => format!("{larger}rd"),
                _ => format!("{larger}th"),
            }),
        }
    }

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        esp_idf_svc::log::EspLogger::initialize_default();

        let mut server = create_server()?;

        server.fn_handler("/", Method::Get, |req| {
            req.into_ok_response()?
                .write_all(INDEX_HTML.as_bytes())
                .map(|_| ())
        })?;

        let guessing_games = Mutex::new(BTreeMap::<i32, GuessingGame>::new());

        server.ws_handler("/ws/guess", None, move |ws| {
            let mut sessions = guessing_games.lock().unwrap();
            if ws.is_new() {
                sessions.insert(ws.session(), GuessingGame::new((rand() % 100) + 1));
                info!("New WebSocket session ({} open)", sessions.len());
                ws.send(
                    FrameType::Text(false),
                    "Welcome to the guessing game! Enter a number between 1 and 100".as_bytes(),
                )?;
                return Ok(());
            } else if ws.is_closed() {
                sessions.remove(&ws.session());
                info!("Closed WebSocket session ({} open)", sessions.len());
                return Ok(());
            }
            let session = sessions.get_mut(&ws.session()).unwrap();

            // NOTE: Due to the way the underlying C implementation works, ws.recv()
            // may only be called with an empty buffer exactly once to receive the
            // incoming buffer size, then must be called exactly once to receive the
            // actual payload.

            let (_frame_type, len) = match ws.recv(&mut []) {
                Ok(frame) => frame,
                Err(e) => return Err(e),
            };

            if len > MAX_LEN {
                ws.send(FrameType::Text(false), "Request too big".as_bytes())?;
                ws.send(FrameType::Close, &[])?;
                return Err(EspError::from_infallible::<ESP_ERR_INVALID_SIZE>());
            }

            let mut buf = [0; MAX_LEN]; // Small digit buffer can go on the stack
            ws.recv(buf.as_mut())?;

            let Ok(user_string) = CStr::from_bytes_until_nul(&buf[..len]) else {
                ws.send(FrameType::Text(false), "[CStr decode Error]".as_bytes())?;
                return Ok(());
            };

            let Ok(user_string) = user_string.to_str() else {
                ws.send(FrameType::Text(false), "[UTF-8 Error]".as_bytes())?;
                return Ok(());
            };

            let Some(user_guess) = GuessingGame::parse_guess(user_string) else {
                ws.send(
                    FrameType::Text(false),
                    "Please enter a number between 1 and 100".as_bytes(),
                )?;
                return Ok(());
            };

            match session.guess(user_guess) {
                (Ordering::Greater, n) => {
                    let reply = format!("Your {} guess was too high", nth(n));
                    ws.send(FrameType::Text(false), reply.as_ref())?;
                }
                (Ordering::Less, n) => {
                    let reply = format!("Your {} guess was too low", nth(n));
                    ws.send(FrameType::Text(false), reply.as_ref())?;
                }
                (Ordering::Equal, n) => {
                    let reply = format!(
                        "You guessed {} on your {} try! Refresh to play again",
                        session.secret,
                        nth(n)
                    );
                    ws.send(FrameType::Text(false), reply.as_ref())?;
                    ws.send(FrameType::Close, &[])?;
                }
            }
            Ok::<(), EspError>(())
        })?;

        // Keep server running beyond when main() returns (forever)
        // Do not call this if you ever want to stop or access it later.
        // Otherwise you can either add an infinite loop so the main task
        // never returns, or you can move it to another thread.
        // https://doc.rust-lang.org/stable/core/mem/fn.forget.html
        core::mem::forget(server);

        // Main task no longer needed, free up some memory
        Ok(())
    }

    fn create_server() -> anyhow::Result<EspHttpServer<'static>> {
        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let mut wifi = BlockingWifi::wrap(
            EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
            sys_loop,
        )?;

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
        wifi.wait_netif_up()?;

        info!("Created Wi-Fi with WIFI_SSID `{SSID}` and WIFI_PASS `{PASSWORD}`");

        let server_configuration = esp_idf_svc::http::server::Configuration {
            stack_size: STACK_SIZE,
            ..Default::default()
        };

        // Keep wifi running beyond when this function returns (forever)
        // Do not call this if you ever want to stop or access it later.
        // Otherwise it should be returned from this function and kept somewhere
        // so it does not go out of scope.
        // https://doc.rust-lang.org/stable/core/mem/fn.forget.html
        core::mem::forget(wifi);

        Ok(EspHttpServer::new(&server_configuration)?)
    }
}
