use std::rc::Rc;
use std::time::Duration;

use embedded_hal_0_2::PwmPin;

use esp_idf_hal::ledc::*;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::units::*;

const CYCLES: usize = 3;

fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();

    println!("Setting up PWM output channels");

    let peripherals = Peripherals::take()?;
    let config = config::TimerConfig::new().frequency(25.kHz().into());
    let timer = Rc::new(LedcTimerDriver::new(peripherals.ledc.timer0, &config)?);
    let channel0 = LedcDriver::new(
        peripherals.ledc.channel0,
        timer.clone(),
        peripherals.pins.gpio4,
    )?;
    let channel1 = LedcDriver::new(peripherals.ledc.channel1, timer, peripherals.pins.gpio5)?;

    println!("Spawning PWM threads");

    let thread0 = std::thread::Builder::new()
        .stack_size(7000)
        .spawn(move || cycle_duty(channel0, CYCLES, "PWM 0", Duration::from_millis(1000)))?;
    let thread1 = std::thread::Builder::new()
        .stack_size(7000)
        .spawn(move || cycle_duty(channel1, CYCLES, "PWM 1", Duration::from_millis(1750)))?;

    println!("Waiting for PWM threads");

    thread0.join().unwrap()?;
    thread1.join().unwrap()?;

    println!("Joined PWM threads");

    println!("Done");

    loop {
        // Don't let the idle task starve and trigger warnings from the watchdog.
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn cycle_duty<P>(mut pwm: P, times: usize, log_prefix: &str, sleep: Duration) -> anyhow::Result<()>
where
    P: PwmPin<Duty = u32>,
{
    let max_duty = pwm.get_max_duty();

    for cycle in 0..times {
        println!("{log_prefix} cycle: {cycle}");

        for numerator in [0, 1, 2, 3, 4, 5].iter() {
            println!("{log_prefix} duty: {numerator}/5");
            pwm.set_duty(max_duty * numerator / 5);
            std::thread::sleep(sleep);
        }
    }

    Ok(())
}
