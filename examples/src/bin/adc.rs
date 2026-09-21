//! ADC example, reading a value form a pin and printing it on the terminal
//!

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

use esp_idf_sys as _; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported

use std::thread;
use std::time::Duration;

#[cfg(any(feature = "adc-oneshot-legacy", esp_idf_version_major = "4"))]
fn main() -> anyhow::Result<()> {
    use esp_idf_hal::adc::config::Config;
    use esp_idf_hal::adc::*;
    use esp_idf_hal::peripherals::Peripherals;
    use esp_idf_hal::sys::adc_atten_t; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported

    let peripherals = Peripherals::take()?;

    #[cfg(not(esp32))]
    let mut adc = AdcDriver::new(peripherals.adc1, &Config::new().calibration(true))?;

    #[cfg(esp32)]
    let mut adc = AdcDriver::new(peripherals.adc2, &Config::new().calibration(true))?;

    const ATTENUATION: adc_atten_t = attenuation::DB_12;

    // configuring pin to analog read, you can regulate the adc input voltage range depending on your need
    // for this example we use the attenuation of 11db which sets the input voltage range to around 0-3.6V
    #[cfg(not(esp32))]
    let mut adc_pin: esp_idf_hal::adc::AdcChannelDriver<{ ATTENUATION }, _> =
        AdcChannelDriver::new(peripherals.pins.gpio4)?;

    #[cfg(esp32)]
    let mut adc_pin: esp_idf_hal::adc::AdcChannelDriver<{ ATTENUATION }, _> =
        AdcChannelDriver::new(peripherals.pins.gpio12)?;

    loop {
        // you can change the sleep duration depending on how often you want to sample
        thread::sleep(Duration::from_millis(10));
        println!("ADC value: {}", adc.read(&mut adc_pin)?);
    }
}

#[cfg(not(any(feature = "adc-oneshot-legacy", esp_idf_version_major = "4")))]
fn main() -> anyhow::Result<()> {
    println!("This example requires feature `adc-oneshot-legacy` enabled or using ESP-IDF v4.4.X");

    loop {
        thread::sleep(Duration::from_millis(1000));
    }
}
