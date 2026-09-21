//! I2C test with SSD1306
//!
//! Folowing pins are used:
//! SDA     GPIO5
//! SCL     GPIO6
//!
//! Depending on your target and the board you are using you have to change the pins.
//!
//! For this example you need to hook up an SSD1306 I2C display.
//! The display will flash black and white.

use esp_idf_hal::delay::{FreeRtos, BLOCK};
use esp_idf_hal::i2c::*;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::units::*;

const SSD1306_ADDRESS: u8 = 0x3c;

fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();

    let peripherals = Peripherals::take()?;
    let i2c = peripherals.i2c0;
    let sda = peripherals.pins.gpio5;
    let scl = peripherals.pins.gpio6;

    println!("Starting I2C SSD1306 test");

    let config = I2cConfig::new().baudrate(100.kHz().into());
    let mut i2c = I2cDriver::new(i2c, sda, scl, &config)?;

    // initialze the display - don't worry about the meaning of these bytes - it's specific to SSD1306
    i2c.write(SSD1306_ADDRESS, &[0, 0xae], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xd4], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0x80], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xa8], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0x3f], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xd3], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0x00], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0x40], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0x8d], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0x14], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xa1], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xc8], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xda], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0x12], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0x81], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xcf], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xf1], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xdb], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0x40], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xa4], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xa6], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0xaf], BLOCK)?;
    i2c.write(SSD1306_ADDRESS, &[0, 0x20, 0x00], BLOCK)?;

    // fill the display
    for _ in 0..64 {
        let data: [u8; 17] = [
            0x40, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff,
        ];
        i2c.write(SSD1306_ADDRESS, &data, BLOCK)?;
    }

    loop {
        // we are sleeping here to make sure the watchdog isn't triggered
        FreeRtos::delay_ms(500);
        i2c.write(SSD1306_ADDRESS, &[0, 0xa6], BLOCK)?;
        FreeRtos::delay_ms(500);
        i2c.write(SSD1306_ADDRESS, &[0, 0xa7], BLOCK)?;
    }
}
