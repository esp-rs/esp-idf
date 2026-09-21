//! ADC oneshot example, reading a value form a pin and printing it on the terminal
//! requires ESP-IDF v5.0 or newer

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

use std::thread;
use std::time::Duration;

#[cfg(not(any(feature = "adc-oneshot-legacy", esp_idf_version_major = "4")))]
fn main() -> anyhow::Result<()> {
    use esp_idf_hal::adc::attenuation::DB_12;
    use esp_idf_hal::adc::oneshot::config::AdcChannelConfig;
    use esp_idf_hal::adc::oneshot::*;
    use esp_idf_hal::peripherals::Peripherals;

    let peripherals = Peripherals::take()?;

    #[cfg(not(esp32))]
    let adc = AdcDriver::new(peripherals.adc1)?;

    #[cfg(esp32)]
    let adc = AdcDriver::new(peripherals.adc2)?;

    // configuring pin to analog read, you can regulate the adc input voltage range depending on your need
    // for this example we use the attenuation of 11db which sets the input voltage range to around 0-3.6V
    let config = AdcChannelConfig {
        attenuation: DB_12,
        ..Default::default()
    };

    #[cfg(not(any(esp32, esp32p4)))]
    let mut adc_pin = AdcChannelDriver::new(&adc, peripherals.pins.gpio2, &config)?;

    #[cfg(esp32)]
    let mut adc_pin = AdcChannelDriver::new(&adc, peripherals.pins.gpio12, &config)?;

    #[cfg(esp32p4)]
    let mut adc_pin = AdcChannelDriver::new(&adc, peripherals.pins.gpio16, &config)?;

    loop {
        // you can change the sleep duration depending on how often you want to sample
        thread::sleep(Duration::from_millis(100));
        println!("ADC value: {}", adc.read(&mut adc_pin)?);
    }
}

#[cfg(any(feature = "adc-oneshot-legacy", esp_idf_version_major = "4"))]
fn main() -> anyhow::Result<()> {
    println!(
        "This example requires ESP-IDF v5.X or newer and feature `adc-oneshot-legacy` disabled"
    );

    loop {
        thread::sleep(Duration::from_millis(1000));
    }
}
