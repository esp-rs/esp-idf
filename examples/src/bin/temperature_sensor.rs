#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(esp_idf_soc_temp_sensor_supported, esp_idf_version_major = "5"))]
fn main() -> anyhow::Result<()> {
    use esp_idf_hal::delay::FreeRtos;
    use esp_idf_hal::peripherals::Peripherals;
    use esp_idf_hal::temp_sensor::*;

    esp_idf_hal::sys::link_patches();

    let peripherals = Peripherals::take()?;
    let cfg = TempSensorConfig::default();
    let mut temp = TempSensorDriver::new(&cfg, peripherals.temp_sensor)?;
    temp.enable()?;

    loop {
        let t = temp.get_celsius()?;
        println!("Temperature {t}C");
        FreeRtos::delay_ms(1000);
    }
}

#[cfg(not(all(esp_idf_soc_temp_sensor_supported, esp_idf_version_major = "5")))]
fn main() -> anyhow::Result<()> {
    println!("This example requires feature `esp_idf_soc_temp_sensor_supported` enabled");

    loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}
