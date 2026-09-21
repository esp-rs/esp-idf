//! A2DP sink example using the external-codec API.
//!
//! Initialises Bluedroid classic, advertises the device as A2DP sink with
//! both AAC and SBC stream endpoints, and logs incoming events including
//! the encoded audio buffers delivered by the peer source (typically a
//! phone). Decoding of those buffers is intentionally left out — pair this
//! with Espressif's `esp_audio_codec` managed component (or any other
//! decoder) on top.
//!
//! Required sdkconfig settings:
//!
//! ```ignore
//! CONFIG_BT_ENABLED=y
//! CONFIG_BT_BLUEDROID_ENABLED=y
//! CONFIG_BT_CLASSIC_ENABLED=y
//! CONFIG_BT_A2DP_ENABLE=y
//! CONFIG_BT_A2DP_USE_EXTERNAL_CODEC=y
//! CONFIG_BT_A2DP_SEP_NUM_MAX=2
//! CONFIG_BTDM_CTRL_MODE_BR_EDR_ONLY=y
//! CONFIG_BTDM_CTRL_MODE_BLE_ONLY=n
//! CONFIG_BTDM_CTRL_MODE_BTDM=n
//! CONFIG_BT_BTC_TASK_STACK_SIZE=15000
//! ```
//!
//! To run this example:
//! MCU=esp32 ESP_IDF_SDKCONFIG_DEFAULTS=sdkconfig.defaults.a2dp_external_codec cargo +esp espflash flash --target xtensa-esp32-espidf --bin bt_a2dp_sink_external_codec --monitor

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(esp32, esp_idf_bt_a2dp_use_external_codec))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(not(all(esp32, esp_idf_bt_a2dp_use_external_codec)))]
fn main() -> anyhow::Result<()> {
    println!("FALLBACK MAIN: cfg gate did not match");
    panic!("This example requires ESP32 with CONFIG_BT_A2DP_USE_EXTERNAL_CODEC=y");
}

#[cfg(all(esp32, esp_idf_bt_a2dp_use_external_codec))]
mod example {
    use std::sync::Arc;

    use esp_idf_svc::bt::a2dp::{A2dpEvent, Codec, EspA2dp, Sink};
    use esp_idf_svc::bt::gap::{DiscoveryMode, EspGap};
    use esp_idf_svc::bt::{reduce_bt_memory, BtClassic, BtDriver};
    use esp_idf_svc::hal::delay::FreeRtos;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::nvs::EspDefaultNvsPartition;

    use log::info;

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();
        info!("step 1: logger up");

        let peripherals = Peripherals::take()?;
        info!("step 2: peripherals taken");
        let nvs = EspDefaultNvsPartition::take()?;
        info!("step 3: nvs taken");

        let mut modem = peripherals.modem;

        reduce_bt_memory(unsafe { modem.reborrow() })?;
        info!("step 4: reduce_bt_memory ok");

        let bt = Arc::new(BtDriver::<BtClassic>::new(modem, Some(nvs.clone()))?);
        info!("step 5: BtDriver up");

        let gap = EspGap::new(bt.clone())?;
        info!("step 6: gap created");
        gap.set_device_name("ESP32_A2DP_SINK")?;
        gap.set_scan_mode(true, DiscoveryMode::Discoverable)?;
        info!("step 7: gap configured");

        let a2dp = EspA2dp::<'_, _, _, Sink>::new_external_codec(bt.clone())?;
        info!("step 8: a2dp external-codec sink up");

        a2dp.subscribe(|event| {
            match &event {
                A2dpEvent::SinkAudioData(buf) => {
                    info!(
                        "audio frame: {} frames, {} bytes, ts {}",
                        buf.frames(),
                        buf.data().len(),
                        buf.timestamp()
                    );
                }
                A2dpEvent::SinkEndpointRegistered { seid, state } => {
                    info!("SEP {seid} register result: {state:?}");
                }
                A2dpEvent::AudioCodecConfigured { bd_addr, codec } => {
                    info!("negotiated codec with {bd_addr:?}: {codec:?}");
                }
                other => info!("a2dp event: {other:?}"),
            }
            0
        })?;

        info!("step 9: subscribed; registering SEPs");
        // AAC at seid 0 (preferred), SBC at seid 1 (mandatory fallback).
        for (seid, codec) in [(0, Codec::aac_default()), (1, Codec::sbc_default())] {
            info!("registering {} at seid {}", codec.name(), seid);
            a2dp.register_sink_endpoint(seid, &codec)?;
        }

        info!("step 10: A2DP sink ready; pair with phone and play audio");

        loop {
            FreeRtos::delay_ms(10_000);
        }
    }
}
