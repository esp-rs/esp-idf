#![allow(unexpected_cfgs)]

use std::time::Duration;

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::ledc::{config::TimerConfig, LedcDriver, LedcTimerDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::units::*;

fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();

    let peripherals = Peripherals::take()?;

    let timer_driver = LedcTimerDriver::new(
        peripherals.ledc.timer0,
        &TimerConfig::default().frequency(25.kHz().into()),
    )?;

    let mut ledc_driver = LedcDriver::new(
        peripherals.ledc.channel0,
        timer_driver,
        peripherals.pins.gpio7,
    )?;

    for _ in 0..2 {
        // Fade up over 2 seconds
        ledc_driver.fade_with_time(
            ledc_driver.get_max_duty(),
            Duration::from_secs(2).as_millis() as i32,
            true,
        )?;

        // Fade down over 2 seconds
        ledc_driver.fade_with_time(0, Duration::from_secs(2).as_millis() as i32, true)?;
    }

    #[cfg(not(any(esp32, esp_idf_version_major = "4")))]
    {
        // Fade up over 2 seconds, but abort after 1
        ledc_driver.fade_with_time(
            ledc_driver.get_max_duty(),
            Duration::from_secs(2).as_millis() as i32,
            false,
        )?;
        FreeRtos::delay_ms(1000);
        // Fade will stop halfway
        ledc_driver.fade_stop()?;
    }

    ledc_driver.set_duty(ledc_driver.get_max_duty() / 10)?;
    FreeRtos::delay_ms(10000);

    Ok(())
}
