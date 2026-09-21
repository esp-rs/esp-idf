//! This example demonstrates how to post events in an ESP IDF event loop
//! - (the system one is used but you can create your own too) -
//!   as well as how to fetch events from the event loop in a callback and asynchronous fashion.
//!
//! Note that the example goes one step further by implementing and then posting and receiving a custom event.
//! However, you can also subscribe to and listen to (and post too) events which are already defined in the ESP IDF itself,
//! like - say - the Wifi events or the Netif events.

#![allow(unknown_lints)]

use core::ffi::CStr;
use core::pin::pin;
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;

use std::sync::Arc;

use esp_idf_svc::eventloop::*;
use esp_idf_svc::hal::delay;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::sys::EspError;
use esp_idf_svc::timer::EspTaskTimerService;

use log::info;

fn main() -> Result<(), EspError> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    run()?;

    Ok(())
}

fn run() -> Result<(), EspError> {
    let sys_loop = EspSystemEventLoop::take()?;

    let counter = Arc::new(AtomicU32::new(0));

    // Post events using a callback-based timer
    let timer_service = EspTaskTimerService::new()?;
    let timer = {
        let sys_loop = sys_loop.clone();
        let counter = counter.clone();

        timer_service.timer(move || {
            let current = counter.fetch_add(1, Ordering::SeqCst);
            sys_loop
                .post::<CustomEvent>(
                    &if current > 0 {
                        CustomEvent::Tick(current)
                    } else {
                        CustomEvent::Start
                    },
                    delay::BLOCK,
                )
                .unwrap();
        })?
    };

    // Let it trigger every second
    timer.every(Duration::from_secs(1))?;

    // Fetch posted events with a callback
    // Need to keep the subscription around, or else if dropped, we'll get unsubscribed
    let _subscription = sys_loop.subscribe::<CustomEvent, _>(|event| {
        info!("[Subscribe callback] Got event: {event:?}");
    })?;

    esp_idf_svc::hal::task::block_on(pin!(async move {
        // Fetch posted events with an async subscription as well
        let mut subscription = sys_loop.subscribe_async::<CustomEvent>()?;

        loop {
            let event = subscription.recv().await?;
            info!("[Subscribe async] Got event: {event:?}");
        }
    }))
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
enum CustomEvent {
    Start,
    Tick(u32),
}

unsafe impl EspEventSource for CustomEvent {
    #[allow(clippy::manual_c_str_literals)]
    fn source() -> Option<&'static CStr> {
        // String should be unique across the whole project and ESP IDF
        Some(CStr::from_bytes_with_nul(b"DEMO-SERVICE\0").unwrap())
    }
}

impl EspEventSerializer for CustomEvent {
    type Data<'a> = CustomEvent;

    fn serialize<F, R>(event: &Self::Data<'_>, f: F) -> R
    where
        F: FnOnce(&EspEventPostData) -> R,
    {
        // Go the easy way since our payload implements Copy and is `'static`
        f(&unsafe { EspEventPostData::new(Self::source().unwrap(), Self::event_id(), event) })
    }
}

impl EspEventDeserializer for CustomEvent {
    type Data<'a> = CustomEvent;

    fn deserialize<'a>(data: &EspEvent<'a>) -> Self::Data<'a> {
        // Just as easy as serializing
        *unsafe { data.as_payload::<CustomEvent>() }
    }
}
