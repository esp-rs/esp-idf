//! SPI loopback test
//!
//! Folowing pins are used:
//! SCLK   GPIO6
//! SDI    GPIO2
//! SDO    GPIO7
//! CS_1   GPIO10
//! CS_2   GPIO3
//!
//! Depending on your target and the board you are using you have to change the pins.
//!
//! This example transfers data via SPI.
//! Connect SDI and SDO pins to see the outgoing data is read as incoming data.

use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::spi::*;
use esp_idf_hal::task::*;
use esp_idf_hal::units::*;

fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();

    let peripherals = Peripherals::take()?;
    let spi = peripherals.spi2;

    let sclk = peripherals.pins.gpio15;
    let serial_in = peripherals.pins.gpio16; // SDI
    let serial_out = peripherals.pins.gpio17; // SDO
    let cs_1 = peripherals.pins.gpio18;
    let cs_2 = peripherals.pins.gpio19;

    println!("Starting SPI loopback test");

    let driver = SpiDriver::new::<SPI2>(
        spi,
        sclk,
        serial_out,
        Some(serial_in),
        &SpiDriverConfig::new(),
    )?;

    let config_1 = config::Config::new().baudrate(26.MHz().into());
    let mut device_1 = SpiDeviceDriver::new(&driver, Some(cs_1), &config_1)?;

    let config_2 = config::Config::new().baudrate(13.MHz().into());
    let mut device_2 = SpiDeviceDriver::new(&driver, Some(cs_2), &config_2)?;

    let write = [0xde, 0xad, 0xbe, 0xef];
    let write_buf = [0xde, 0xad, 0xbe, 0xef];

    let mut read = [0u8; 4];
    let mut write_in_place_buf = [0xde, 0xad, 0xbe, 0xef];
    let mut read_buf = [0; 8];

    block_on(async {
        loop {
            device_1.transfer_async(&mut read, &write).await?;
            println!("Device 1: Wrote {write:x?}, read {read:x?}");

            println!("Device 2: To write {write_in_place_buf:x?} ... ");
            // cascade multiple operations with different buffer length into one transaction
            device_2
                .transaction_async(&mut [
                    Operation::Write(&write_buf),
                    Operation::TransferInPlace(&mut write_in_place_buf),
                    Operation::Read(&mut read_buf),
                ])
                .await?;
            println!("... read {write_in_place_buf:x?}");
        }
    })
}
