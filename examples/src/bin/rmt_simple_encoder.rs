//! This example is based on the led strip example found in the esp-idf repository:
//! https://github.com/espressif/esp-idf/blob/master/examples/peripherals/rmt/led_strip_simple_encoder/main/led_strip_example_main.c
//!
//! It uses the RMT peripheral to control an addressable LED strip (WS2812).
//!
//! This example demonstrates how to create a custom RMT encoder using the `SimpleEncoder` and how to interact with the
//! RMT API in general.
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
    use esp_idf_hal::rmt::config::{MemoryAccess, TxChannelConfig};
    use esp_idf_hal::rmt::encoder::simple_encoder::{
        EncoderCallback, NotEnoughSpace, SimpleEncoder, SymbolBuffer,
    };
    use esp_idf_hal::rmt::TxChannelDriver;
    use esp_idf_hal::rmt::{PinState, PulseTicks, Symbol};
    use esp_idf_hal::units::Hertz;

    use core::f64::consts::PI;
    use core::time::Duration;

    const RMT_LED_STRIP_RESOLUTION_HZ: Hertz = Hertz(10_000_000); // 10MHz resolution, 1 tick = 0.1us
    const NUMBER_OF_LEDS: usize = 1;
    const ANGLE_INC_FRAME: f64 = 0.02;
    const ANGLE_INC_LED: f64 = 0.3;
    const FRAME_DURATION: Duration = Duration::from_millis(20);
    const BRIGHTNESS: f64 = 0.5; // 0.0 = dark, 1.0 = full brightness

    // From the WS2812 datasheet:
    const T0H: Duration = Duration::from_nanos(350); // 0.35us
    const T0L: Duration = Duration::from_nanos(800); // 0.8us
    const T1H: Duration = Duration::from_nanos(700); // 0.7us
    const T1L: Duration = Duration::from_nanos(600); // 0.6us
    const TRESET: Duration = Duration::from_micros(281); // >50us

    // From the WS2812b datasheet:
    // const T0H: Duration = Duration::from_nanos(300); // 0.22us ~ 0.38us
    // const T0L: Duration = Duration::from_nanos(790); // 0.58us ~ 1.0us
    // const T1H: Duration = Duration::from_nanos(790); // 0.58us ~ 1.0us
    // const T1L: Duration = Duration::from_nanos(790); // 0.58us ~ 1.0us
    // const TRESET: Duration = Duration::from_micros(281); // >280us

    #[derive(Debug, Clone, Copy, Default)]
    #[repr(C)]
    pub struct Color {
        pub red: u8,
        pub green: u8,
        pub blue: u8,
    }

    impl Color {
        #[must_use]
        pub fn apply_brightness(self, brightness: f64) -> Self {
            let factor = brightness.clamp(0.0, 1.0);

            Self {
                red: (self.red as f64 * factor).round() as u8,
                green: (self.green as f64 * factor).round() as u8,
                blue: (self.blue as f64 * factor).round() as u8,
            }
        }
    }

    fn byte_to_symbols(byte: u8) -> impl IntoIterator<Item = Symbol> {
        let ws2812_zero = Symbol::new_with(
            RMT_LED_STRIP_RESOLUTION_HZ,
            PinState::High,
            T0H,
            PinState::Low,
            T0L,
        )
        .unwrap();
        let ws2812_one = Symbol::new_with(
            RMT_LED_STRIP_RESOLUTION_HZ,
            PinState::High,
            T1H,
            PinState::Low,
            T1L,
        )
        .unwrap();

        (0..8)
            .map(|i| 0x80 >> i)
            .map(move |bitmask| (byte & bitmask) != 0)
            .map(move |is_one| if is_one { ws2812_one } else { ws2812_zero })
    }

    impl Color {
        #[must_use]
        pub fn to_vec(self) -> Vec<Symbol> {
            let mut result = Vec::with_capacity(24);
            for byte in [self.green, self.red, self.blue] {
                result.extend(byte_to_symbols(byte));
            }

            result
        }
    }

    pub struct LedEncoder {
        input_position: usize,
    }

    impl LedEncoder {
        pub fn new() -> Self {
            Self { input_position: 0 }
        }
    }

    impl EncoderCallback for LedEncoder {
        type Item = Color;

        fn encode(
            &mut self,
            input_data: &[Self::Item],
            buffer: &mut SymbolBuffer<'_>,
        ) -> Result<(), NotEnoughSpace> {
            let ws2812_reset = Symbol::new_half_split(
                RMT_LED_STRIP_RESOLUTION_HZ,
                PinState::Low,
                PinState::Low,
                TRESET,
            )
            .unwrap();

            if buffer.position() == 0 {
                self.input_position = 0;
            }

            for &next_color in &input_data[self.input_position..] {
                let mut symbols = vec![ws2812_reset];
                symbols.extend(next_color.apply_brightness(BRIGHTNESS).to_vec());

                buffer.write_all(&symbols)?;
                self.input_position += 1;
            }

            // finished encoding all input data

            // Add a delay between this signal and the next one:

            let max_duration = PulseTicks::max().duration(RMT_LED_STRIP_RESOLUTION_HZ) * 2; // times two, because we use half-split symbols

            let vec = Symbol::new_half_split(
                RMT_LED_STRIP_RESOLUTION_HZ,
                PinState::Low,
                PinState::Low,
                max_duration,
            )
            .unwrap()
            .repeat_for(RMT_LED_STRIP_RESOLUTION_HZ, FRAME_DURATION)
            .collect::<Vec<Symbol>>();

            buffer.write_all(&vec)?;

            Ok(())
        }
    }

    pub fn run() -> anyhow::Result<()> {
        println!("Create RMT TX channel");

        let peripherals = Peripherals::take()?;

        let mut channel = TxChannelDriver::new(
            peripherals.pins.gpio21, // set the pin of the led strip here
            &TxChannelConfig {
                clock_source: Default::default(),
                memory_access: MemoryAccess::Indirect {
                    memory_block_symbols: 64,
                }, // increasing might reduce flickering
                resolution: RMT_LED_STRIP_RESOLUTION_HZ,
                transaction_queue_depth: 4, // The number of transactions that can be pending in the background
                ..Default::default()
            },
        )?;

        println!("Start LED rainbow chase");
        let mut offset = 0.0;

        let iterator = core::iter::from_fn(|| {
            let mut signal = [Color::default(); NUMBER_OF_LEDS];
            for (i, color) in signal.iter_mut().enumerate() {
                // Build RGB pixels. Each color is an offset sine, which gives a
                // hue-like effect.
                let angle = offset + (i as f64 * ANGLE_INC_LED);
                let color_offset = (PI * 2.0) / 3.0;

                *color = Color {
                    red: ((angle + color_offset * 1.0).sin() * 127.0 + 128.0) as u8,
                    green: ((angle + color_offset * 0.0).sin() * 127.0 + 128.0) as u8,
                    blue: ((angle + color_offset * 2.0).sin() * 117.0 + 128.0) as u8,
                };
            }

            // Increase offset to shift pattern
            offset += ANGLE_INC_FRAME;
            if offset > 2.0 * PI {
                offset -= 2.0 * PI;
            }

            Some(signal)
        });

        // This will use 10 encoders to encode the data returned by the iterator:
        let mut encoders = [(); 10]
            .map(|_| SimpleEncoder::with_config(LedEncoder::new(), &Default::default()).unwrap());

        channel.send_iter(encoders.each_mut(), iterator, &Default::default())?;

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
