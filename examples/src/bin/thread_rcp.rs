//! Example of using Thread in "RCP" mode.
//!
//! The `RCP` mode of `ThreadDriver` is a mode where the Thread stack is running on the MCU, however,
//! it communicates via UART or SPI to another - "master" MCU which - most often than not - does not
//! have a native Thread radio, but has other connectivity like Wifi. It is this other MCU which actually runs
//! Thread as a real "Node", from the POV of the user.
//!
//! NOTE: This example only works on MCUs that has Thread capabilities, like the ESP32-C6 or ESP32-H2.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    #[cfg(all(any(esp32h2, esp32c6), esp_idf_openthread_radio))]
    example::main()?;

    #[cfg(not(any(esp32h2, esp32c6)))]
    log::error!("This example only works on MCUs that have Thread capabilities, like the ESP32-C6 or ESP32-H2.");

    #[cfg(not(esp_idf_openthread_radio))]
    log::error!("Put `CONFIG_OPENTHREAD_RADIO=y` in your `sdkconfig.defaults`");

    Ok(())
}

#[cfg(all(any(esp32h2, esp32c6), esp_idf_openthread_radio))]
mod example {
    use std::sync::Arc;

    use log::info;

    use esp_idf_svc::eventloop::EspSystemSubscription;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::io::vfs::MountedEventfs;
    use esp_idf_svc::thread::{ThreadDriver, ThreadEvent};
    use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};

    pub fn main() -> anyhow::Result<()> {
        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let mounted_event_fs = Arc::new(MountedEventfs::mount(4)?);

        info!("Initializing Thread RCP...");

        let _subscription = log_thread_sysloop(sys_loop.clone())?;

        let mut thread = ThreadDriver::new_rcp_uart(
            peripherals.modem,
            peripherals.uart1,
            peripherals.pins.gpio10,
            peripherals.pins.gpio11,
            &esp_idf_svc::thread::config::uart_default_cfg(),
            sys_loop.clone(),
            nvs,
            mounted_event_fs,
        )?;

        info!("Thread RCP initialized, now running...");

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
            info!("Got: {:?}", event);
        })?;

        Ok(subscription)
    }
}
