//! Tests light sleep
//!
//! Enables multiple light sleep wakeup sources and sleeps in a loop.
//! Prints wakeup reason and sleep time on wakeup.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

use core::time::Duration;
use std::thread;
use std::time::Instant;

use esp_idf_hal::gpio::{AnyIOPin, Level, PinDriver, Pull};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::reset::WakeupReason;
use esp_idf_hal::sleep::LightSleep;
use esp_idf_hal::uart::config::Config;
use esp_idf_hal::uart::UartDriver;
use esp_idf_hal::units::Hertz;

#[cfg(not(any(esp32c2, esp32c3, esp32h2)))]
use esp_idf_hal::sleep::{RtcWakeLevel, RtcWakeupPins};

fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();

    let peripherals = Peripherals::take()?;

    // Sample GPIO pins
    let gpio0 = PinDriver::input(peripherals.pins.gpio5, Pull::Down)?;
    let gpio1 = PinDriver::input(peripherals.pins.gpio6, Pull::Down)?;

    // Sample RTC pins
    #[cfg(not(any(esp32c2, esp32c3, esp32h2, esp32h4)))]
    let rtc0 = PinDriver::rtc_input(peripherals.pins.gpio2, Pull::Down)?;
    #[cfg(not(any(esp32c2, esp32c3, esp32h2, esp32h4)))]
    let rtc1 = PinDriver::rtc_input(peripherals.pins.gpio4, Pull::Down)?;

    // Sample UART driver
    let uart = UartDriver::new(
        peripherals.uart0,
        peripherals.pins.gpio7,
        peripherals.pins.gpio8,
        None::<AnyIOPin>,
        None::<AnyIOPin>,
        &Config::new().baudrate(Hertz(115_200)),
    )?;

    // Configure Wakeup Sources
    let mut sleep = LightSleep::new()?;

    sleep = sleep
        .wakeup_on_timer(Duration::from_secs(5))?
        .wakeup_on_gpio(&gpio0, Level::High)?
        .wakeup_on_gpio(&gpio1, Level::High)?
        .wakeup_on_uart(&uart, 3)?;

    #[cfg(not(any(esp32c2, esp32c3, esp32h2, esp32h4)))]
    {
        let pins = rtc0.chain(&rtc1);
        sleep = sleep.wakeup_on_rtc(&pins, RtcWakeLevel::AnyHigh)?;
    }

    loop {
        // short sleep to flush stdout
        thread::sleep(Duration::from_millis(60));
        println!("Entering light sleep...");

        let time_before = Instant::now();

        sleep.enter()?;

        let time_after = Instant::now();
        let wakeup_reason = WakeupReason::get();
        println!(
            "wake up from light sleep due to {wakeup_reason:?} which lasted for {:?}",
            time_after - time_before
        );

        println!("---");
    }
}
