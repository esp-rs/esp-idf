//! This example demonstrates a how to ask for the reset reason and the wakeup reason.
use std::{thread, time};

use esp_idf_sys::{self as _}; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();

    let wakeup_reason = esp_idf_hal::reset::WakeupReason::get();
    println!("Wakeup reason: {wakeup_reason:?}");

    let reset_reason = esp_idf_hal::reset::ResetReason::get();
    println!("Reset reason: {reset_reason:?}");

    thread::sleep(time::Duration::from_millis(1000));

    let sleep_micros = 2_000_000;
    unsafe {
        esp_idf_sys::esp_sleep_enable_timer_wakeup(sleep_micros);

        println!("Going to deep sleep {} seconds", sleep_micros / 1_000_000);
        esp_idf_sys::esp_deep_sleep_start();
        // Software reset!
    }
}
