//! MQTT asynchronous client example which subscribes to an internet MQTT server and then sends
//! and receives events in its own topic.
//!
//! Note: On ESP-IDF v6.0+, the MQTT component was moved out of the main tree. To enable it,
//! add the following to your `Cargo.toml`:
//! ```toml
//! [[package.metadata.esp-idf-sys.extra_components]]
//! remote_component = { name = "espressif/mqtt", version = "1.*" }
//! ```

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(
    not(any(esp32h2, esp32h4, esp32p4)),
    any(esp_idf_comp_mqtt_enabled, esp_idf_comp_espressif__mqtt_enabled)
))]
fn main() {
    example::main()
}

#[cfg(any(esp32h2, esp32h4, esp32p4))]
fn main() {
    panic!("ESP32-H2, ESP32-H4 and ESP32-P4 do not have a Wifi radio (but you could enable the esp-wifi-remote component to use them with a WiFi co-processor)");
}

#[cfg(all(
    not(any(esp32h2, esp32h4, esp32p4)),
    not(any(esp_idf_comp_mqtt_enabled, esp_idf_comp_espressif__mqtt_enabled))
))]
fn main() {
    panic!("MQTT component is not enabled. See the note at the top of this file.");
}

#[cfg(all(
    not(any(esp32h2, esp32h4, esp32p4)),
    any(esp_idf_comp_mqtt_enabled, esp_idf_comp_espressif__mqtt_enabled)
))]
mod example {
    use core::pin::pin;
    use core::time::Duration;

    use embassy_futures::select::{select, Either};

    use esp_idf_svc::eventloop::EspSystemEventLoop;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::mqtt::client::*;
    use esp_idf_svc::nvs::EspDefaultNvsPartition;
    use esp_idf_svc::sys::EspError;
    use esp_idf_svc::timer::{EspAsyncTimer, EspTaskTimerService, EspTimerService};
    use esp_idf_svc::wifi::*;

    use log::*;

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    const MQTT_URL: &str = "mqtt://broker.emqx.io:1883";
    const MQTT_CLIENT_ID: &str = "esp-mqtt-demo";
    const MQTT_TOPIC: &str = "esp-mqtt-demo";

    pub fn main() {
        esp_idf_svc::sys::link_patches();
        esp_idf_svc::log::EspLogger::initialize_default();

        let sys_loop = EspSystemEventLoop::take().unwrap();
        let timer_service = EspTimerService::new().unwrap();
        let nvs = EspDefaultNvsPartition::take().unwrap();

        esp_idf_svc::hal::task::block_on(async {
            let _wifi = wifi_create(&sys_loop, &timer_service, &nvs).await?;
            info!("Wifi created");

            let (mut client, mut conn) = mqtt_create(MQTT_URL, MQTT_CLIENT_ID)?;
            info!("MQTT client created");

            let mut timer = timer_service.timer_async()?;
            run(&mut client, &mut conn, &mut timer, MQTT_TOPIC).await
        })
        .unwrap();
    }

    async fn run(
        client: &mut EspAsyncMqttClient,
        connection: &mut EspAsyncMqttConnection,
        timer: &mut EspAsyncTimer,
        topic: &str,
    ) -> Result<(), EspError> {
        info!("About to start the MQTT client");

        let res = select(
            // Need to immediately start pumping the connection for messages, or else subscribe() and publish() below will not work
            // Note that when using the alternative structure and the alternative constructor - `EspMqttClient::new_cb` - you don't need to
            // spawn a new thread, as the messages will be pumped with a backpressure into the callback you provide.
            // Yet, you still need to efficiently process each message in the callback without blocking for too long.
            //
            // Note also that if you go to http://tools.emqx.io/ and then connect and send a message to topic
            // "esp-mqtt-demo", the client configured here should receive it.
            pin!(async move {
                info!("MQTT Listening for messages");

                while let Ok(event) = connection.next().await {
                    info!("[Queue] Event: {}", event.payload());
                }

                info!("Connection closed");

                Ok(())
            }),
            pin!(async move {
                // Using `pin!` is optional, but it optimizes the memory size of the Futures
                loop {
                    if let Err(e) = client.subscribe(topic, QoS::AtMostOnce).await {
                        error!("Failed to subscribe to topic \"{topic}\": {e}, retrying...");

                        // Re-try in 0.5s
                        timer.after(Duration::from_millis(500)).await?;

                        continue;
                    }

                    info!("Subscribed to topic \"{topic}\"");

                    // Just to give a chance of our connection to get even the first published message
                    timer.after(Duration::from_millis(500)).await?;

                    let payload = "Hello from esp-mqtt-demo!";

                    loop {
                        client
                            .publish(topic, QoS::AtMostOnce, false, payload.as_bytes())
                            .await?;

                        info!("Published \"{payload}\" to topic \"{topic}\"");

                        let sleep_secs = 2;

                        info!("Now sleeping for {sleep_secs}s...");
                        timer.after(Duration::from_secs(sleep_secs)).await?;
                    }
                }
            }),
        )
        .await;

        match res {
            Either::First(res) => res,
            Either::Second(res) => res,
        }
    }

    fn mqtt_create(
        url: &str,
        client_id: &str,
    ) -> Result<(EspAsyncMqttClient, EspAsyncMqttConnection), EspError> {
        let (mqtt_client, mqtt_conn) = EspAsyncMqttClient::new(
            url,
            &MqttClientConfiguration {
                client_id: Some(client_id),
                ..Default::default()
            },
        )?;

        Ok((mqtt_client, mqtt_conn))
    }

    async fn wifi_create(
        sys_loop: &EspSystemEventLoop,
        timer_service: &EspTaskTimerService,
        nvs: &EspDefaultNvsPartition,
    ) -> Result<EspWifi<'static>, EspError> {
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
}
