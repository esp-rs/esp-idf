//! Test communication between a master and a slave I2C on the same device
//!
//! Wiring required, but can be changed in main():
//! - GPIO21 to GPIO18
//! - GPIO22 to GPIO19
//!
//! ESP32-C2/C3 does not have two I2C peripherals, so this example will not work.
//!
//! Description:
//! Consists of three parts:
//! 1. Simple master write, master writes 8 bytes and print out what slave receives
//! 2. Simple master read, master read 8 bytes and print out.
//! 3. Read/write register, write a value to a register addr and read it back.
//!
#![allow(unused)]
#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

use esp_idf_hal::delay::BLOCK;
use esp_idf_hal::gpio::{AnyIOPin, InputPin, OutputPin};
use esp_idf_hal::i2c::{I2c, I2cConfig, I2cDriver, I2cSlaveConfig, I2cSlaveDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::units::*;

const SLAVE_ADDR: u8 = 0x22;
const SLAVE_BUFFER_SIZE: usize = 128;

fn i2c_master_init<'d>(
    i2c: impl I2c + 'd,
    sda: AnyIOPin<'d>,
    scl: AnyIOPin<'d>,
    baudrate: Hertz,
) -> anyhow::Result<I2cDriver<'d>> {
    let config = I2cConfig::new().baudrate(baudrate);
    let driver = I2cDriver::new(i2c, sda, scl, &config)?;
    Ok(driver)
}

fn i2c_slave_init<'d>(
    i2c: impl I2c + 'd,
    sda: AnyIOPin<'d>,
    scl: AnyIOPin<'d>,
    buflen: usize,
    slave_addr: u8,
) -> anyhow::Result<I2cSlaveDriver<'d>> {
    let config = I2cSlaveConfig::new()
        .rx_buffer_length(buflen)
        .tx_buffer_length(buflen);
    let driver = I2cSlaveDriver::new(i2c, sda, scl, slave_addr, &config)?;
    Ok(driver)
}

#[cfg(not(esp32))]
fn main() -> anyhow::Result<()> {
    println!("Test only configured for ESP32");
    Ok(())
}

#[cfg(esp32)]
fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();

    println!("Starting I2C self test");

    let peripherals = Peripherals::take()?;

    let mut i2c_master = i2c_master_init(
        peripherals.i2c0,
        peripherals.pins.gpio21.into(),
        peripherals.pins.gpio22.into(),
        100.kHz().into(),
    )?;

    let mut i2c_slave = i2c_slave_init(
        peripherals.i2c1,
        peripherals.pins.gpio18.into(),
        peripherals.pins.gpio19.into(),
        SLAVE_BUFFER_SIZE,
        SLAVE_ADDR,
    )?;

    let tx_buf: [u8; 8] = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];

    println!("-------- TESTING SIMPLE MASTER WRITE --------");
    i2c_master.write(SLAVE_ADDR, &tx_buf, BLOCK)?;

    let mut rx_buf: [u8; 8] = [0; 8];
    match i2c_slave.read(&mut rx_buf, BLOCK) {
        Ok(_) => println!("Master send {tx_buf:?} Slave receives {rx_buf:?}"),
        Err(e) => println!("Error: {e:?}"),
    }

    println!("-------- TESTING SIMPLE MASTER READ --------");
    i2c_slave.write(&tx_buf, BLOCK)?; // fill slave buffer before master tries to read

    let mut rx_buf: [u8; 8] = [0; 8];
    match i2c_master.read(SLAVE_ADDR, &mut rx_buf, BLOCK) {
        Ok(_) => println!("Slave send {tx_buf:?} Master receives {rx_buf:?}"),
        Err(e) => println!("Error: {e:?}"),
    }

    println!("-------- TESTING READ/WRITE REGISTER --------");

    let thread0 = std::thread::Builder::new()
        .stack_size(7000)
        .spawn(move || {
            let mut data: [u8; 256] = [0; 256];
            loop {
                let mut reg_addr: [u8; 1] = [0];
                let res = i2c_slave.read(&mut reg_addr, BLOCK);
                if let Err(e) = res {
                    println!("SLAVE: failed to read register address from master: Error: {e:?}");
                    continue;
                }
                let mut rx_data: [u8; 1] = [0];
                match i2c_slave.read(&mut rx_data, 0) {
                    Ok(_) => {
                        println!(
                            "SLAVE: write operation {:#04x} to reg addr {:#04x}",
                            rx_data[0], reg_addr[0]
                        );
                        data[reg_addr[0] as usize] = rx_data[0];
                    }
                    Err(_) => {
                        let d = data[reg_addr[0] as usize];
                        println!(
                            "SLAVE: read operation {:#04x} from reg addr {:#04x}",
                            d, reg_addr[0]
                        );
                        i2c_slave.write(&[d], BLOCK).unwrap();
                    }
                }
            }
        })?;

    // allow thread to run
    std::thread::yield_now();

    let reg_addr: u8 = 0x05;
    let new_value: u8 = 0x42;

    println!("MASTER: read reg addr {reg_addr:#04x}");
    let mut rx_buf: [u8; 1] = [0; 1];
    // TODO: make write_read work
    i2c_master.write(SLAVE_ADDR, &[reg_addr], BLOCK)?;
    i2c_master.read(SLAVE_ADDR, &mut rx_buf, BLOCK)?;
    println!(
        "MASTER: value of reg addr {:#04x} is {:#04x}",
        reg_addr, rx_buf[0]
    );

    println!("---------------------");

    println!("MASTER: write {new_value:#04x} to reg addr {reg_addr:#04x}");
    i2c_master.write(SLAVE_ADDR, &[reg_addr, new_value], BLOCK)?;

    println!("---------------------");

    println!("MASTER: read reg addr {reg_addr:#04x}");
    let mut rx_buf: [u8; 1] = [0; 1];
    // TODO: make write_read work
    i2c_master.write(SLAVE_ADDR, &[reg_addr], BLOCK)?;
    i2c_master.read(SLAVE_ADDR, &mut rx_buf, BLOCK)?;
    println!(
        "MASTER: value of reg addr {:#04x} is {:#04x}",
        reg_addr, rx_buf[0]
    );

    thread0.join().unwrap();
    Ok(())
}
