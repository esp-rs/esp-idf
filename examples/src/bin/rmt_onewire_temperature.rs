//! RMT Onewire Example
//!
//! Example demonstrating the use of the onewire component to measure temperature from the ds18b20 temperature probe.
//!
//! In order to use this example, an overidden `Cargo.toml` must be defined with the following definitions:
//! ```
//! [[package.metadata.esp-idf-sys.extra_components]]
//! remote_component = { name = "onewire_bus", version = "^1.0.4" }
//! ```
//!
//! The example can then be run with
//! `MCU=<target> cargo run --example rmt_onewire --manifest-path /path/to/other/Cargo.toml`
//!
//! Below is a connection sketch, the signal pin must be externally pulled-up
//! with a 4.7kOhm resistor.
//! This example uses gpio 16, but any pin capable of
//! input AND output is suitable.
//!
//! If the example is successful, it should print the address of each
//! onewire device attached to the bus.
//!
//! ┌──────────────────────────┐
//! │                      3.3V├───────┬─────────────┬──────────────────────┐
//! │                          │      ┌┴┐            │VDD                   │VDD
//! │          ESP Board       │  4.7k│ │     ┌──────┴──────┐        ┌──────┴──────┐
//! │                          │      └┬┘   DQ│             │      DQ│             │
//! │          ONEWIRE_GPIO_PIN├───────┴──┬───┤   DS18B20   │    ┌───┤   DS18B20   │   ......
//! │                          │          └───│-------------│────┴───│-------------│──
//! │                          │              └──────┬──────┘        └──────┬──────┘
//! │                          │                     │GND                   │GND
//! │                       GND├─────────────────────┴──────────────────────┘
//! └──────────────────────────┘
//!
//!
//! This example demonstrates:
//! * A RMT device in both TX and RX mode.
//! * Usage of the onewire bus driver interface.
//! * How to iterate through a device search to discover devices on the bus.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(
    esp_idf_soc_rmt_supported,
    not(feature = "rmt-legacy"),
    not(esp_idf_version_major = "4"),
    esp_idf_comp_espressif__onewire_bus_enabled,
))]
mod example {
    use core::time::Duration;

    use esp_idf_hal::delay::FreeRtos;
    use esp_idf_hal::onewire::{OWAddress, OWCommand, OWDriver};
    use esp_idf_hal::peripherals::Peripherals;

    use esp_idf_sys::EspError;

    fn ds18b20_send_command<'a>(addr: &OWAddress, bus: &OWDriver, cmd: u8) -> Result<(), EspError> {
        let mut buf = [0; 10];
        buf[0] = OWCommand::MatchRom as _;
        let addr = addr.address().to_le_bytes();
        buf[1..9].copy_from_slice(&addr);
        buf[9] = cmd;

        bus.write(&buf)
    }

    #[allow(dead_code)]
    #[repr(u8)]
    enum Ds18b20Command {
        ConvertTemp = 0x44,
        WriteScratch = 0x4E,
        ReadScratch = 0xBE,
    }
    #[cfg(all(
        esp_idf_soc_rmt_supported,
        not(esp_idf_version_major = "4"),
        esp_idf_comp_espressif__onewire_bus_enabled,
    ))]
    fn ds18b20_trigger_temp_conversion<'a>(
        addr: &OWAddress,
        bus: &OWDriver,
    ) -> Result<(), EspError> {
        // reset bus and check if the ds18b20 is present
        bus.reset()?;

        ds18b20_send_command(addr, bus, Ds18b20Command::ConvertTemp as u8)?;

        // delay proper time for temp conversion,
        // assume max resolution (12-bits)
        std::thread::sleep(Duration::from_millis(800));

        Ok(())
    }
    #[cfg(all(
        esp_idf_soc_rmt_supported,
        not(esp_idf_version_major = "4"),
        esp_idf_comp_espressif__onewire_bus_enabled,
    ))]
    fn ds18b20_get_temperature<'a>(addr: &OWAddress, bus: &OWDriver) -> Result<f32, EspError> {
        bus.reset()?;

        ds18b20_send_command(addr, bus, Ds18b20Command::ReadScratch as u8)?;

        let mut buf = [0u8; 9];
        bus.read(&mut buf)?;
        let lsb = buf[0];
        let msb = buf[1];

        let temp_raw: u16 = (u16::from(msb) << 8) | u16::from(lsb);

        Ok(f32::from(temp_raw) / 16.0)
    }

    pub fn run() -> anyhow::Result<()> {
        println!("Starting APP!");

        let peripherals = Peripherals::take()?;

        let onewire_gpio_pin = peripherals.pins.gpio16;

        let mut onewire_bus: OWDriver = OWDriver::new(onewire_gpio_pin)?;
        let device = {
            let mut search = onewire_bus.search()?;
            search.next()
        };

        if device.is_none() {
            println!("No device found");
            return Ok(());
        }

        let device = device.unwrap();
        if let Err(err) = device {
            println!("An error occured searching for the device, err = {}", err);
            return Err(err.into());
        }
        let device = device.unwrap();
        println!(
            "Found Device: {:?}, family code = {}",
            device,
            device.family_code()
        );

        loop {
            ds18b20_trigger_temp_conversion(&device, &onewire_bus)?;
            let temp = ds18b20_get_temperature(&device, &onewire_bus)?;
            println!("Temperature: {}", temp);
            FreeRtos::delay_ms(3000);
        }
    }
}

#[cfg(all(
    esp_idf_soc_rmt_supported,
    not(feature = "rmt-legacy"),
    not(esp_idf_version_major = "4"),
    esp_idf_comp_espressif__onewire_bus_enabled,
))]
fn main() -> anyhow::Result<()> {
    example::run()
}

#[cfg(not(all(
    esp_idf_soc_rmt_supported,
    not(feature = "rmt-legacy"),
    not(esp_idf_version_major = "4"),
    esp_idf_comp_espressif__onewire_bus_enabled,
)))]
fn main() -> anyhow::Result<()> {
    println!("This example requires feature `rmt-legacy` disabled, using ESP-IDF > v4.4.X, the component included in `Cargo.toml`, or is not supported on this MCU");

    loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}
