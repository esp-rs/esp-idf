//! USB Serial / JTAG peripheral and driver
//!
//! Communication through a virtualized UART-like USB-CDC interface.
#![allow(non_camel_case_types)]

use core::marker::PhantomData;

use crate::io::EspIOError;
use crate::sys::{
    esp, usb_serial_jtag_driver_config_t, usb_serial_jtag_driver_install,
    usb_serial_jtag_driver_uninstall, usb_serial_jtag_is_connected, usb_serial_jtag_read_bytes,
    usb_serial_jtag_write_bytes, EspError, TickType_t,
};
use crate::{delay, gpio};

/// A type alias for the USB Serial driver configuration
pub type UsbSerialConfig = config::Config;

/// USB D- GPIO pin
#[cfg(esp32c3)]
pub type UsbDMinGpio<'d> = gpio::Gpio18<'d>;
/// USB D+ GPIO pin
#[cfg(esp32c3)]
pub type UsbDPlusGpio<'d> = gpio::Gpio19<'d>;
/// USB D- GPIO pin
#[cfg(esp32c5)]
pub type UsbDMinGpio<'d> = gpio::Gpio13<'d>;
/// USB D+ GPIO pin
#[cfg(esp32c5)]
pub type UsbDPlusGpio<'d> = gpio::Gpio14<'d>;
/// USB D- GPIO pin
#[cfg(any(esp32c6, esp32c61))]
pub type UsbDMinGpio<'d> = gpio::Gpio12<'d>;
/// USB D+ GPIO pin
#[cfg(any(esp32c6, esp32c61))]
pub type UsbDPlusGpio<'d> = gpio::Gpio13<'d>;
/// USB D- GPIO pin
#[cfg(esp32h2)]
pub type UsbDMinGpio<'d> = gpio::Gpio26<'d>;
/// USB D+ GPIO pin
#[cfg(esp32h2)]
pub type UsbDPlusGpio<'d> = gpio::Gpio27<'d>;
/// USB D- GPIO pin
#[cfg(esp32p4)]
pub type UsbDMinGpio<'d> = gpio::Gpio24<'d>;
/// USB D+ GPIO pin
#[cfg(esp32p4)]
pub type UsbDPlusGpio<'d> = gpio::Gpio25<'d>;
// TODO
// #[cfg(esp32p4)]
// pub type UsbDMinGpio2<'d> = gpio::Gpio26<'d>;
// #[cfg(esp32p4)]
// pub type UsbDPlusGpio2<'d> = gpio::Gpio27<'d>;
/// USB D- GPIO pin
#[cfg(esp32s3)]
pub type UsbDMinGpio<'d> = gpio::Gpio19<'d>;
/// USB D- GPIO pin
#[cfg(esp32s31)]
pub type UsbDMinGpio<'d> = gpio::Gpio33<'d>;
/// USB D+ GPIO pin
#[cfg(esp32s31)]
pub type UsbDPlusGpio<'d> = gpio::Gpio34<'d>;
/// USB D+ GPIO pin
#[cfg(esp32s3)]
pub type UsbDPlusGpio<'d> = gpio::Gpio20<'d>;

/// USB Serial driver configuration
pub mod config {
    /// USB Serial driver configuration
    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub struct Config {
        pub tx_buffer_size: usize,
        pub rx_buffer_size: usize,
    }

    impl Config {
        /// Create a new configuration with default values
        pub const fn new() -> Self {
            Self {
                tx_buffer_size: 256,
                rx_buffer_size: 256,
            }
        }

        /// Set the transmit buffer size
        #[must_use]
        pub fn tx_buffer_size(mut self, tx_buffer_size: usize) -> Self {
            self.tx_buffer_size = tx_buffer_size;
            self
        }

        /// Set the receive buffer size
        #[must_use]
        pub fn rx_buffer_size(mut self, rx_buffer_size: usize) -> Self {
            self.rx_buffer_size = rx_buffer_size;
            self
        }
    }

    impl Default for Config {
        fn default() -> Self {
            Self::new()
        }
    }
}

/// USB-SERIAL driver
pub struct UsbSerialDriver<'d>(PhantomData<&'d mut ()>);

impl<'d> UsbSerialDriver<'d> {
    /// Create a new USB Serial driver
    ///
    /// # Arguments
    /// - `usb_serial`: The USB Serial peripheral
    /// - `config`: The driver configuration
    /// - `usb_d_min`: The USB D- GPIO pin
    /// - `usb_d_plus`: The USB D+ GPIO pin
    pub fn new(
        _usb_serial: USB_SERIAL<'d>,
        _usb_d_min: UsbDMinGpio<'d>,
        _usb_d_plus: UsbDPlusGpio<'d>,
        config: &config::Config,
    ) -> Result<Self, EspError> {
        #[allow(clippy::needless_update)]
        let mut config = usb_serial_jtag_driver_config_t {
            tx_buffer_size: config.tx_buffer_size as _,
            rx_buffer_size: config.rx_buffer_size as _,
            ..Default::default()
        };

        esp!(unsafe { usb_serial_jtag_driver_install(&mut config) })?;

        Ok(Self(PhantomData))
    }

    /// Check if the USB Serial is connected
    pub fn is_connected(&self) -> bool {
        unsafe { usb_serial_jtag_is_connected() }
    }

    /// Read bytes into a slice
    ///
    /// # Arguments
    /// - `buf`: The buffer to read into
    /// - `timeout`: The timeout in ticks
    ///
    /// # Returns
    /// The number of bytes read or an error if the operation failed or the timeout was reached
    pub fn read(&mut self, buf: &mut [u8], timeout: TickType_t) -> Result<usize, EspError> {
        let len = unsafe {
            usb_serial_jtag_read_bytes(buf.as_mut_ptr() as *mut _, buf.len() as _, timeout)
        };

        Ok(len as _)
    }

    /// Write bytes from a slice
    ///
    /// # Arguments
    /// - `bytes`: The bytes to write
    /// - `timeout`: The timeout in ticks
    ///
    /// # Returns
    /// The number of bytes written or an error if the operation failed or the timeout was reached
    pub fn write(&mut self, bytes: &[u8], timeout: TickType_t) -> Result<usize, EspError> {
        let len = unsafe {
            usb_serial_jtag_write_bytes(bytes.as_ptr() as *const _, bytes.len() as _, timeout)
        };

        Ok(len as _)
    }
}

impl Drop for UsbSerialDriver<'_> {
    fn drop(&mut self) {
        esp!(unsafe { usb_serial_jtag_driver_uninstall() }).unwrap();
    }
}

unsafe impl Send for UsbSerialDriver<'_> {}

impl embedded_io::ErrorType for UsbSerialDriver<'_> {
    type Error = EspIOError;
}

impl embedded_io::Read for UsbSerialDriver<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        UsbSerialDriver::read(self, buf, delay::BLOCK).map_err(EspIOError)
    }
}

impl embedded_io::Write for UsbSerialDriver<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        UsbSerialDriver::write(self, buf, delay::BLOCK).map_err(EspIOError)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl embedded_hal_0_2::serial::Write<u8> for UsbSerialDriver<'_> {
    type Error = EspError;

    fn flush(&mut self) -> nb::Result<(), Self::Error> {
        Ok(())
    }

    fn write(&mut self, byte: u8) -> nb::Result<(), Self::Error> {
        UsbSerialDriver::write(self, &[byte], delay::BLOCK)?;

        Ok(())
    }
}

impl core::fmt::Write for UsbSerialDriver<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let buf = s.as_bytes();
        let mut offset = 0;

        while offset < buf.len() {
            offset += self
                .write(buf, delay::BLOCK)
                .map_err(|_| core::fmt::Error)?
        }

        Ok(())
    }
}

crate::impl_peripheral!(USB_SERIAL);
