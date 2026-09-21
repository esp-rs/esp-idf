//! Turn an LED on/off depending on the state of a button
//!
//! This assumes that a LED is connected to GPIO4.
//! Additionally this assumes a button connected to GPIO9.
//! On an ESP32C3 development board this is the BOOT button.
//!
//! Depending on your target and the board you are using you should change the pins.
//! If your board doesn't have on-board LEDs don't forget to add an appropriate resistor.

use esp_idf_hal::gpio::*;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::task::*;

fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();

    let peripherals = Peripherals::take()?;

    let mut led = PinDriver::output(peripherals.pins.gpio4)?;
    let mut button = PinDriver::input(peripherals.pins.gpio9, Pull::Down)?;

    block_on(async {
        loop {
            button.wait_for_high().await?;

            led.set_high()?;

            button.wait_for_low().await?;

            led.set_low()?;
        }
    })
}
