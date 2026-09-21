//! UART loopback test
//!
//! Folowing pins are used:
//! TX    GPIO12
//! RX    GPIO13
//!
//! Depending on your target and the board you are using you have to change the pins.
//!
//! This example transfers data via UART.
//! Connect TX and RX pins to see the outgoing data is read as incoming data.

use esp_idf_hal::delay::BLOCK;
use esp_idf_hal::gpio;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::uart::*;
use esp_idf_hal::units::*;

fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();

    let peripherals = Peripherals::take()?;
    let tx = peripherals.pins.gpio12;
    let rx = peripherals.pins.gpio13;

    println!("Starting UART loopback test");
    let config = config::Config::new().baudrate(Hertz(115_200));
    let uart = UartDriver::new(
        peripherals.uart1,
        tx,
        rx,
        Option::<gpio::Gpio0>::None,
        Option::<gpio::Gpio1>::None,
        &config,
    )?;

    loop {
        uart.write(&[0xaa])?;

        let mut buf = [0_u8; 1];
        uart.read(&mut buf, BLOCK)?;

        println!("Written 0xaa, read 0x{:02x}", buf[0]);
    }
}
