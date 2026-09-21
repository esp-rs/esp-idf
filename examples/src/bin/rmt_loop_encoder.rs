//! This example demonstrates how to create a custom RMT encoder that loops for a
//! specified number of times or indefinitely.
//!
//! The `Loop` option provided by the driver is not available on every ESP32 and
//! how often it can loop (given a count) is limited to `u32::MAX` (maybe less).
//!
//! This example creates a custom encoder that will not have these limitations.
//!
//! SPDX-License-Identifier: Unlicense OR CC0-1.0

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(any(
    feature = "rmt-legacy",
    not(esp_idf_version_at_least_5_3_0),
    not(esp_idf_soc_rmt_supported),
))]
fn main() -> anyhow::Result<()> {
    println!("This example requires feature `rmt-legacy` disabled, using ESP-IDF >= v5.1.2, or is not supported on this MCU");

    loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}

#[cfg(all(
    esp_idf_soc_rmt_supported,
    esp_idf_version_at_least_5_3_0,
    not(feature = "rmt-legacy")
))]
mod example {
    use esp_idf_hal::peripherals::Peripherals;
    use esp_idf_hal::rmt::config::TxChannelConfig;
    use esp_idf_hal::rmt::encoder::{
        CopyEncoder, Encoder, EncoderState, EncoderWrapper, RmtChannelHandle,
    };
    use esp_idf_hal::rmt::Symbol;
    use esp_idf_hal::rmt::TxChannelDriver;
    use esp_idf_hal::units::Hertz;

    use esp_idf_sys::*;

    const RMT_RESOLUTION: Hertz = Hertz(10_000_000); // 10MHz resolution, 1 tick = 0.1us

    pub struct LoopEncoder<E> {
        count: u128,
        target: Option<u128>,
        encoder: E,
    }

    impl<E: Encoder> LoopEncoder<E> {
        pub fn new(target: Option<u128>, encoder: E) -> Self {
            Self {
                count: 0,
                target,
                encoder,
            }
        }
    }

    impl<E: Encoder> Encoder for LoopEncoder<E> {
        type Item = E::Item;

        fn encode(
            &mut self,
            handle: &mut RmtChannelHandle,
            primary_data: &[Self::Item],
        ) -> (usize, EncoderState) {
            let mut written = 0;
            let mut state;

            loop {
                let (current_written, current_state) = self.encoder.encode(handle, primary_data);
                written += current_written;
                state = current_state;

                // If the inner encoder completed, one count has been completed.
                if let EncoderState::EncodingComplete = state {
                    // only increment the count if there is a target, otherwise it might crash in debug mode because of overflow
                    if self.target.is_some() {
                        self.count += 1;
                    }

                    // If the target count has been reached, stop encoding.
                    if self.target.is_some_and(|target| self.count >= target) {
                        break;
                    }
                } else {
                    break;
                }
            }

            (written, state)
        }

        fn reset(&mut self) -> Result<(), EspError> {
            self.encoder.reset()?;
            self.count = 0;
            Ok(())
        }
    }

    pub fn run() -> anyhow::Result<()> {
        println!("Create RMT TX channel");

        let peripherals = Peripherals::take()?;

        let mut channel = TxChannelDriver::new(
            peripherals.pins.gpio21,
            &TxChannelConfig {
                resolution: RMT_RESOLUTION,
                ..Default::default()
            },
        )?;

        println!("Start Encoding and sending symbols");

        let encoder = EncoderWrapper::new(LoopEncoder::new(Some(50), CopyEncoder::new()?))?;

        channel.send_and_wait(encoder, &[Symbol::default()], &Default::default())?;

        Ok(())
    }
}

#[cfg(all(
    esp_idf_soc_rmt_supported,
    esp_idf_version_at_least_5_3_0,
    not(feature = "rmt-legacy")
))]
fn main() -> anyhow::Result<()> {
    example::run()
}
