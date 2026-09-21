//! Example of using Thread in "Node" mode.
//!
//! The example just starts Thread and logs the events, without doing anything else useful.
//! However, in 99% of the case this is exactly what you want to do.
//!
//! NOTE: This example only works on MCUs that has Thread capabilities, like the ESP32-C6 or ESP32-H2.
//!
//! It is however possible to run this example on other MCUs, using the UART or SPI protocols, but then
//! you anyway would need _another_, Thread-capable MCU that runs Thread in RCP mode (see the `thread_rcp`) example.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    #[cfg(any(esp32h2, esp32c6))]
    example::main()?;

    #[cfg(not(any(esp32h2, esp32c6)))]
    log::error!("This example only works on MCUs that have Thread capabilities, like the ESP32-C6 or ESP32-H2.");

    Ok(())
}

#[cfg(any(esp32h2, esp32c6))]
mod example {
    use std::sync::Arc;

    use log::info;

    use esp_idf_svc::eventloop::EspSystemSubscription;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::io::vfs::MountedEventfs;
    use esp_idf_svc::thread::{EspThread, ThreadEvent};
    use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};

    pub fn main() -> anyhow::Result<()> {
        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let mounted_event_fs = Arc::new(MountedEventfs::mount(4)?);

        info!("Initializing Thread...");

        let _subscription = log_thread_sysloop(sys_loop.clone())?;

        let mut thread =
            EspThread::new(peripherals.modem, sys_loop.clone(), nvs, mounted_event_fs)?;

        info!("Thread initialized, now running...");

        thread.start()?;

        loop {
            // Keep the main thread alive to allow the Thread Border Router to run
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }

    fn log_thread_sysloop(
        sys_loop: EspSystemEventLoop,
    ) -> Result<EspSystemSubscription<'static>, anyhow::Error> {
        let subscription = sys_loop.subscribe::<ThreadEvent, _>(|event| {
            info!("Got: {event:?}");
        })?;

        Ok(subscription)
    }
}
