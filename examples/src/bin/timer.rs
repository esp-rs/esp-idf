//! An example of timers created via the ESP IDF Timer service:
//! - One of the timers is callback based and triggers continously every second.
//! - The other timer is asynchronous and triggers once only (so we have to re-load it every time after it had triggered).

use core::pin::pin;
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;

use std::sync::Arc;

use esp_idf_svc::log::EspLogger;
use esp_idf_svc::sys::EspError;
use esp_idf_svc::timer::EspTaskTimerService;

use log::info;

fn main() {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    run().unwrap();
}

fn run() -> Result<(), EspError> {
    let counter = Arc::new(AtomicU32::new(0));

    let timer_service = EspTaskTimerService::new()?;
    let callback_timer = {
        let counter = counter.clone();
        timer_service.timer(move || {
            let current = counter.fetch_add(1, Ordering::SeqCst);

            info!("Callback timer reports tick: {current}");
        })?
    };

    // Let it trigger every second
    callback_timer.every(Duration::from_secs(1))?;

    esp_idf_svc::hal::task::block_on(pin!(async move {
        let mut async_timer = timer_service.timer_async()?;

        loop {
            // Run it slower than the callback one because why not
            // Also use a oneshot timer to demonstrate that it works too
            async_timer.after(Duration::from_secs(3)).await?;

            let current = counter.fetch_add(1, Ordering::SeqCst);
            info!("Async timer reports tick: {current}");
        }
    }))
}
