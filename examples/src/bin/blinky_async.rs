//! Blinks an LED
//!
//! This assumes that a LED is connected to GPIO4.
//! Depending on your target and the board you are using you should change the pin.
//! If your board doesn't have on-board LEDs don't forget to add an appropriate resistor.
//!

#![allow(unexpected_cfgs)]

#[cfg(all(not(feature = "timer-legacy"), esp_idf_soc_gptimer_supported))]
mod example {
    use std::time::Duration;

    use anyhow::Context;
    use esp_idf_hal::gpio::PinDriver;
    use esp_idf_hal::peripherals::Peripherals;
    use esp_idf_hal::task::block_on;
    use esp_idf_hal::timer::config::TimerConfig;
    use esp_idf_hal::timer::TimerDriver;

    pub fn run() -> anyhow::Result<()> {
        esp_idf_hal::sys::link_patches();

        let peripherals = Peripherals::take().context("Failed to take peripherals")?;

        let mut led = PinDriver::output(peripherals.pins.gpio4)?;
        let mut timer = TimerDriver::new(&TimerConfig::default())?;
        timer.subscribe_default()?;
        timer.enable()?;

        block_on(async {
            timer.start()?;

            loop {
                led.set_high()?;

                timer.delay(Duration::from_millis(500)).await?;

                led.set_low()?;

                timer.delay(Duration::from_millis(500)).await?;
            }
        })
    }
}

#[cfg(all(not(feature = "timer-legacy"), esp_idf_soc_gptimer_supported))]
fn main() -> anyhow::Result<()> {
    example::run()
}

#[cfg(all(feature = "timer-legacy", not(esp_idf_version_at_least_6_0_0)))]
fn main() {
    use esp_idf_hal::gpio::*;
    use esp_idf_hal::peripherals::Peripherals;
    use esp_idf_hal::task::*;
    use esp_idf_hal::timer::*;

    esp_idf_hal::sys::link_patches();

    let peripherals = Peripherals::take()?;

    let mut led = PinDriver::output(peripherals.pins.gpio4)?;
    let mut timer = TimerDriver::new(peripherals.timer00, &TimerConfig::new())?;

    block_on(async {
        loop {
            led.set_high()?;

            timer.delay(timer.tick_hz()).await?;

            led.set_low()?;

            timer.delay(timer.tick_hz()).await?;
        }
    })
}
