//! Toggle an LED on/off with a button
//!
//! This assumes that a LED is connected to GPIO4.
//! Additionally this assumes a button connected to GPIO9.
//! On an ESP32C3 development board this is the BOOT button.
//!
//! Depending on your target and the board you are using you should change the pins.
//! If your board doesn't have on-board LEDs don't forget to add an appropriate resistor.

use core::num::NonZero;
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{InterruptType, PinDriver, Pull};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::task::notification::Notification;

fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();

    let peripherals = Peripherals::take()?;
    let mut led = PinDriver::output(peripherals.pins.gpio4)?;
    let mut button = PinDriver::input(peripherals.pins.gpio9, Pull::Down)?;

    button.set_interrupt_type(InterruptType::PosEdge)?;

    let mut led_state = false;
    led.set_low()?;

    loop {
        // prepare communication channel
        let notification = Notification::new();
        let waker = notification.notifier();

        // register interrupt callback, here it's a closure on stack
        unsafe {
            button
                .subscribe_nonstatic(move || {
                    waker.notify(NonZero::new(1).unwrap());
                })
                .unwrap();
        }

        // enable interrupt, will be automatically disabled after being triggered
        button.enable_interrupt()?;
        // block until notified
        notification.wait_any();

        // toggle the LED
        if led_state {
            led.set_low()?;
            led_state = false;
        } else {
            led.set_high()?;
            led_state = true;
        }

        // debounce
        FreeRtos::delay_ms(200);
    }
}
