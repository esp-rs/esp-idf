//! An example of mounting, formatting, and using an SD card with Littlefs
//!
//! To use, put this in your `Cargo.toml`:
//! ```
//! [[package.metadata.esp-idf-sys.extra_components]]
//! remote_component = { name = "joltwallet/littlefs", version = "1.14" }
//! ```
//!
//! To use with an SD card, put this in your `sdkconfig.defaults`:
//! ```
//! CONFIG_LITTLEFS_SDMMC_SUPPORT=y
//! ```
//!
//! NOTE: While this example is using the SD card via the SPI interface,
//! it is also possible to use the SD card via the SDMMC interface. Moreover,
//! it is possible to initialize and use Littlefs with the internal Flash
//! storage as well, by using one of the two unsafe constructors that take
//! a partition label or a raw partition pointer.

#![allow(unexpected_cfgs)]

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    #[cfg(not(esp32))]
    {
        log::error!("This example is configured for esp32, please adjust pins to your module.");
    }

    #[cfg(esp32)]
    {
        #[cfg(i_have_done_all_configs_from_the_top_comment)]
        // Remove this `cfg` when you have done all of the above for the example to compile
        example_main()?;

        // Remove this whole code block when you have done all of the above for the example to compile
        #[cfg(not(i_have_done_all_configs_from_the_top_comment))]
        {
            log::error!("Please follow the instructions in the source code.");
        }
    }

    Ok(())
}

#[cfg(i_have_done_all_configs_from_the_top_comment)] // Remove this `cfg` when you have done all of the above for the example to compile
#[cfg(esp32)]
fn example_main() -> anyhow::Result<()> {
    use std::fs::{read_dir, File};
    use std::io::{Read, Seek, Write};

    use esp_idf_svc::fs::littlefs::Littlefs;
    use esp_idf_svc::hal::gpio::AnyIOPin;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::hal::sd::{spi::SdSpiHostDriver, SdCardConfiguration, SdCardDriver};
    use esp_idf_svc::hal::spi::{config::DriverConfig, Dma, SpiDriver};
    use esp_idf_svc::io::vfs::MountedLittlefs;
    use esp_idf_svc::log::EspLogger;

    use log::info;

    esp_idf_svc::sys::link_patches();

    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;

    let spi_driver = SpiDriver::new(
        peripherals.spi3,
        pins.gpio18,
        pins.gpio23,
        Some(pins.gpio19),
        &DriverConfig::default().dma(Dma::Auto(4096)),
    )?;

    let sd_card_driver = SdCardDriver::new_spi(
        SdSpiHostDriver::new(
            spi_driver,
            Some(pins.gpio5),
            AnyIOPin::none(),
            AnyIOPin::none(),
            AnyIOPin::none(),
            #[cfg(not(any(
                esp_idf_version_major = "4",
                all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
            )))] // For ESP-IDF v5.2 and later
            None,
        )?,
        &SdCardConfiguration::new(),
    )?;

    let littlefs = Littlefs::new_sdcard(sd_card_driver)?;

    // Format it first, as chances are, the user won't have easy access to the Littlefs filesystem from her PC
    // Commented out for safety
    // log::info!("Formatting the SD card");
    // littlefs.format()?;
    // log::info!("SD card formatted");

    // Keep it around or else it will be dropped and unmounted
    let mounted_littlefs = MountedLittlefs::mount(littlefs, "/sdcard")?;

    info!("Filesystem usage: {:?}", mounted_littlefs.info()?);

    let content = b"Hello, world!";

    {
        let mut file = File::create("/sdcard/test.txt")?;

        info!("File {file:?} created");

        file.write_all(content).expect("Write failed");

        info!("File {file:?} written with {content:?}");

        file.seek(std::io::SeekFrom::Start(0)).expect("Seek failed");

        info!("File {file:?} seeked");
    }

    {
        let mut file = File::open("/sdcard/test.txt")?;

        info!("File {file:?} opened");

        let mut file_content = String::new();

        file.read_to_string(&mut file_content).expect("Read failed");

        info!("File {file:?} read: {file_content}");

        assert_eq!(file_content.as_bytes(), content);
    }

    {
        let directory = read_dir("/sdcard")?;

        for entry in directory {
            log::info!("Entry: {:?}", entry?.file_name());
        }
    }

    Ok(())
}
