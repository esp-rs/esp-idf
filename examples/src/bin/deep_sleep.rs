//! Tests deep sleep
//!
//! Enables multiple deep sleep wakeup sources and then enter deep sleep.
//! There is no loop here, since the program will not continue after deep sleep,
//! it always starts from the beginning after a deep sleep wake-up.
//! The program starts by printing reset and wakeup reason, since the deep
//! sleep effectively ends the program, this is how we get information about
//! the previous run.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

use core::time::Duration;

#[cfg(any(esp32c2, esp32c3))]
use esp_idf_hal::gpio::Level;
#[cfg(not(any(esp32h2, esp32h4)))]
use esp_idf_hal::gpio::{PinDriver, Pull};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::reset::{ResetReason, WakeupReason};
use esp_idf_hal::sleep::DeepSleep;

#[cfg(not(any(esp32c2, esp32c3, esp32h2, esp32h4)))]
use esp_idf_hal::sleep::{RtcWakeLevel, RtcWakeupPins};

fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();

    println!(
        "Reset after {:?}; wakeup due to {:?}",
        ResetReason::get(),
        WakeupReason::get()
    );

    #[allow(unused)]
    let peripherals = Peripherals::take()?;
    let mut sleep = DeepSleep::new()?;

    sleep = sleep.wakeup_on_timer(Duration::from_secs(5))?;

    #[cfg(any(esp32c2, esp32c3))]
    {
        let gpio0 = PinDriver::input(peripherals.pins.gpio0, Pull::Down)?;
        let gpio1 = PinDriver::input(peripherals.pins.gpio1, Pull::Down)?;

        sleep = sleep
            .wakeup_on_gpio(&gpio0, Level::High)?
            .wakeup_on_gpio(&gpio1, Level::Low)?;
    }

    #[cfg(not(any(esp32c2, esp32c3, esp32h2, esp32h4)))]
    {
        #[cfg(esp32)]
        let (pin0, pin1) = (peripherals.pins.gpio26, peripherals.pins.gpio27);
        #[cfg(not(esp32))]
        let (pin0, pin1) = (peripherals.pins.gpio1, peripherals.pins.gpio2);

        let rtc0 = PinDriver::rtc_input(pin0, Pull::Down)?;
        let rtc1 = PinDriver::rtc_input(pin1, Pull::Down)?;

        let pins = rtc0.chain(&rtc1);
        sleep = sleep.wakeup_on_rtc(&pins, RtcWakeLevel::AnyHigh)?;
    }

    println!("Entering deep sleep...");
    sleep.enter();
}
