#![allow(unexpected_cfgs)]

#[cfg(all(not(feature = "timer-legacy"), esp_idf_soc_gptimer_supported))]
mod example {
    use std::time::Duration;

    use esp_idf_hal::task::block_on;
    use esp_idf_hal::timer::config::TimerConfig;
    use esp_idf_hal::timer::TimerDriver;

    pub fn run() -> anyhow::Result<()> {
        esp_idf_hal::sys::link_patches();

        let mut timer = TimerDriver::new(&TimerConfig::default())?;
        timer.subscribe_default()?;
        timer.enable()?;

        block_on(async {
            timer.start()?;

            loop {
                timer.delay(Duration::from_secs(1)).await?;

                println!("Tick");
            }
        })
    }
}

#[cfg(all(not(feature = "timer-legacy"), esp_idf_soc_gptimer_supported))]
fn main() -> anyhow::Result<()> {
    example::run()
}

#[cfg(all(feature = "timer-legacy", not(esp_idf_version_at_least_6_0_0)))]
fn main() -> Result<(), esp_idf_hal::sys::EspError> {
    use esp_idf_hal::peripherals::*;
    use esp_idf_hal::task::*;
    use esp_idf_hal::timer::*;

    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_hal::sys::link_patches();

    let per = Peripherals::take()?;

    let mut timer = TimerDriver::new(per.timer00, &TimerConfig::new())?;

    block_on(async {
        loop {
            timer.delay(timer.tick_hz()).await?; // Every second

            println!("Tick");
        }
    })
}
