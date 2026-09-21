//! A simple example to change colours of a WS2812/NeoPixel compatible LED.
//!
//! This example demonstrates the use of [`FixedLengthSignal`][crate::rmt::FixedLengthSignal] which
//! lives on the stack and requires a known length before creating it.
//!
//! There is a similar implementation in the esp-idf project:
//! https://github.com/espressif/esp-idf/tree/20847eeb96/examples/peripherals/rmt/led_strip
//!
//! Datasheet (PDF) for a WS2812, which explains how the pulses are to be sent:
//! https://cdn-shop.adafruit.com/datasheets/WS2812.pdf

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(any(feature = "rmt-legacy", esp_idf_version_major = "4"))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(not(any(feature = "rmt-legacy", esp_idf_version_major = "4")))]
fn main() -> anyhow::Result<()> {
    println!("This example requires feature `rmt-legacy` enabled or using ESP-IDF v4.4.X");

    loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}

#[cfg(any(feature = "rmt-legacy", esp_idf_version_major = "4"))]
mod example {
    use std::time::Duration;

    use anyhow::{bail, Result};
    use esp_idf_hal::{
        delay::FreeRtos,
        peripherals::Peripherals,
        rmt::{config::TransmitConfig, FixedLengthSignal, PinState, Pulse, TxRmtDriver},
    };

    pub fn main() -> Result<()> {
        esp_idf_hal::sys::link_patches();

        let peripherals = Peripherals::take()?;
        // Onboard RGB LED pin
        // ESP32-C3-DevKitC-02 gpio8, ESP32-C3-DevKit-RUST-1 gpio2
        let led = peripherals.pins.gpio2;
        let channel = peripherals.rmt.channel0;
        let config = TransmitConfig::new().clock_divider(1);
        let mut tx = TxRmtDriver::new(channel, led, &config)?;

        // 3 seconds white at 10% brightness
        neopixel(Rgb::new(25, 25, 25), &mut tx)?;
        FreeRtos::delay_ms(3000);

        // infinite rainbow loop at 20% brightness
        (0..360).cycle().try_for_each(|hue| {
            FreeRtos::delay_ms(10);
            let rgb = Rgb::from_hsv(hue, 100, 20)?;
            neopixel(rgb, &mut tx)
        })
    }

    fn neopixel(rgb: Rgb, tx: &mut TxRmtDriver) -> Result<()> {
        let color: u32 = rgb.into();
        let ticks_hz = tx.counter_clock()?;
        let (t0h, t0l, t1h, t1l) = (
            Pulse::new_with_duration(ticks_hz, PinState::High, &Duration::from_nanos(350))?,
            Pulse::new_with_duration(ticks_hz, PinState::Low, &Duration::from_nanos(800))?,
            Pulse::new_with_duration(ticks_hz, PinState::High, &Duration::from_nanos(700))?,
            Pulse::new_with_duration(ticks_hz, PinState::Low, &Duration::from_nanos(600))?,
        );
        let mut signal = FixedLengthSignal::<24>::new();
        for i in (0..24).rev() {
            let p = 2_u32.pow(i);
            let bit: bool = p & color != 0;
            let (high_pulse, low_pulse) = if bit { (t1h, t1l) } else { (t0h, t0l) };
            signal.set(23 - i as usize, &(high_pulse, low_pulse))?;
        }
        tx.start_blocking(&signal)?;
        Ok(())
    }

    struct Rgb {
        r: u8,
        g: u8,
        b: u8,
    }

    impl Rgb {
        pub fn new(r: u8, g: u8, b: u8) -> Self {
            Self { r, g, b }
        }
        /// Converts hue, saturation, value to RGB
        pub fn from_hsv(h: u32, s: u32, v: u32) -> Result<Self> {
            if h > 360 || s > 100 || v > 100 {
                bail!("The given HSV values are not in valid range");
            }
            let s = s as f64 / 100.0;
            let v = v as f64 / 100.0;
            let c = s * v;
            let x = c * (1.0 - (((h as f64 / 60.0) % 2.0) - 1.0).abs());
            let m = v - c;
            let (r, g, b) = match h {
                0..=59 => (c, x, 0.0),
                60..=119 => (x, c, 0.0),
                120..=179 => (0.0, c, x),
                180..=239 => (0.0, x, c),
                240..=299 => (x, 0.0, c),
                _ => (c, 0.0, x),
            };
            Ok(Self {
                r: ((r + m) * 255.0) as u8,
                g: ((g + m) * 255.0) as u8,
                b: ((b + m) * 255.0) as u8,
            })
        }
    }

    impl From<Rgb> for u32 {
        /// Convert RGB to u32 color value
        ///
        /// e.g. rgb: (1,2,4)
        /// G        R        B
        /// 7      0 7      0 7      0
        /// 00000010 00000001 00000100
        fn from(rgb: Rgb) -> Self {
            ((rgb.g as u32) << 16) | ((rgb.r as u32) << 8) | rgb.b as u32
        }
    }
}
