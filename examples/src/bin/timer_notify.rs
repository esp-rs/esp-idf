#![allow(unexpected_cfgs)]

#[cfg(all(not(feature = "timer-legacy"), esp_idf_soc_gptimer_supported))]
mod example {
    use std::num::NonZeroU32;
    use std::time::Duration;

    use esp_idf_hal::task::notification::Notification;
    use esp_idf_hal::timer::config::{AlarmConfig, TimerConfig};
    use esp_idf_hal::timer::*;

    pub fn run() -> anyhow::Result<()> {
        // It is necessary to call this function once. Otherwise some patches to the runtime
        // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
        esp_idf_hal::sys::link_patches();

        // A safer abstraction over FreeRTOS/ESP-IDF task notifications.
        let notification = Notification::new();

        let mut timer = TimerDriver::new(&TimerConfig::default())?;
        timer.subscribe_default()?;
        timer.enable()?;

        // Every half a second
        timer.set_alarm_action(Some(&AlarmConfig {
            alarm_count: timer.duration_to_count(Duration::from_millis(500))?,
            auto_reload_on_alarm: true,
            ..Default::default()
        }))?;

        timer.start()?;

        let notifier = notification.notifier();

        // SAFETY: make sure the `Notification` object is not dropped while the subscription is active
        unsafe {
            timer.subscribe(move |_| {
                let bitset = 0b10001010101;
                notifier.notify_and_yield(NonZeroU32::new(bitset).unwrap());
            })?;
        }

        loop {
            // Notify approach
            // The benefit with this approach over checking a global static variable is
            // that the scheduler can block the task, and quickly resume it when notified
            // so no spinlock is needed / the CPU does not waste cycles.
            let bitset = notification.wait(esp_idf_hal::delay::BLOCK);

            if let Some(bitset) = bitset {
                println!("got event with bits {bitset:#b} from ISR");
            }
        }
    }
}

#[cfg(all(not(feature = "timer-legacy"), esp_idf_soc_gptimer_supported))]
fn main() -> anyhow::Result<()> {
    example::run()
}

#[cfg(all(feature = "timer-legacy", not(esp_idf_version_at_least_6_0_0)))]
fn main() -> Result<(), esp_idf_hal::sys::EspError> {
    use std::num::NonZeroU32;

    use esp_idf_hal::peripherals::*;
    use esp_idf_hal::task::notification::Notification;
    use esp_idf_hal::timer::*;

    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_hal::sys::link_patches();

    let per = Peripherals::take()?;

    // A safer abstraction over FreeRTOS/ESP-IDF task notifications.
    let notification = Notification::new();

    // BaseClock for the Timer is the APB_CLK that is running on 80MHz at default
    // The default clock-divider is -> 80
    // default APB clk is available with the APB_CLK_FREQ constant
    let timer_conf = config::Config::new().auto_reload(true);
    let mut timer = TimerDriver::new(per.timer00, &timer_conf)?;

    // Every half a second
    timer.set_alarm(timer.tick_hz() / 2)?;

    let notifier = notification.notifier();

    // Saftey: make sure the `Notification` object is not dropped while the subscription is active
    unsafe {
        timer.subscribe(move || {
            let bitset = 0b10001010101;
            notifier.notify_and_yield(NonZeroU32::new(bitset).unwrap());
        })?;
    }

    timer.enable_interrupt()?;
    timer.enable_alarm(true)?;
    timer.enable(true)?;

    loop {
        // Notify approach
        // The benefit with this approach over checking a global static variable is
        // that the scheduler can block the task, and quickly resume it when notified
        // so no spinlock is needed / the CPU does not waste cycles.
        let bitset = notification.wait(esp_idf_hal::delay::BLOCK);

        if let Some(bitset) = bitset {
            println!("got event with bits {bitset:#b} from ISR");
        }
    }
}
