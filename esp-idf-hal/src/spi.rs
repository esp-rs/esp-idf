//! SPI peripheral control
//!
//! SPI0 is reserved for accessing flash and sram and therefore not usable for other purposes.
//! SPI1 shares its external pins with SPI0 and therefore has severe restrictions in use.
//!
//! SPI2 & 3 can be used freely.
//!
//! The CS pin can be controlled by hardware on esp32 variants (contrary to the description of embedded_hal).
//!
//! Look at the following table to determine which driver best suits your requirements:
//!
//! |   |                  | SpiDeviceDriver::new | SpiDeviceDriver::new (no CS) | SpiSoftCsDeviceDriver::new | SpiBusDriver::new |
//! |---|------------------|----------------------|------------------------------|----------------------------|-------------------|
//! |   | Managed CS       |       Hardware       |              N               |     Software triggered     |         N         |
//! |   | 1 device         |           Y          |              Y               |              Y             |         Y         |
//! |   | 1-3 devices      |           Y          |              N               |              Y             |         N         |
//! |   | 4-6 devices      |    Only on esp32CX   |              N               |              Y             |         N         |
//! |   | More than 6      |           N          |              N               |              Y             |         N         |
//! |   | DMA              |           Y          |              Y               |              Y             |         Y         |
//! |   | Polling transmit |           Y          |              Y               |              Y             |         Y         |
//! |   | ISR transmit     |           Y          |              Y               |              Y             |         Y         |
//! |   | Async support*   |           Y          |              Y               |              Y             |         Y         |
//!
//! * True non-blocking async possible only when all devices attached to the SPI bus are used in async mode (i.e. calling methods `xxx_async()`
//!   instead of their blocking `xxx()` counterparts)
//!
//! The [Transfer::transfer], [Write::write] and [WriteIter::write_iter] functions lock the
//! APB frequency and therefore the requests are always run at the requested baudrate.
//! The primitive [FullDuplex::read] and [FullDuplex::send] do not lock the APB frequency and
//! therefore may run at a different frequency.
//!
//! # TODO
//! - Slave SPI

use core::borrow::{Borrow, BorrowMut};
use core::cell::Cell;
use core::cell::UnsafeCell;
use core::cmp::{max, min, Ordering};
use core::future::Future;
use core::iter::once;
use core::marker::PhantomData;
use core::ptr;

use embassy_sync::mutex::Mutex;
use embedded_hal::spi::{SpiBus, SpiDevice};

use esp_idf_sys::*;
use heapless::Deque;

use crate::delay::{self, Ets, BLOCK};
use crate::gpio::{AnyOutputPin, InputPin, Level, Output, OutputMode, OutputPin, PinDriver};
use crate::interrupt::asynch::HalIsrNotification;
use crate::interrupt::InterruptType;
use crate::task::embassy_sync::EspRawMutex;
use crate::task::CriticalSection;

crate::embedded_hal_error!(
    SpiError,
    embedded_hal::spi::Error,
    embedded_hal::spi::ErrorKind
);

use config::{Duplex, LineWidth};

pub trait Spi: Send {
    fn device() -> spi_host_device_t;
}

/// A marker interface implemented by all SPI peripherals except SPI1 which
/// should use a fixed set of pins
pub trait SpiAnyPins: Spi {}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Dma {
    Disabled,
    Channel1(usize),
    Channel2(usize),
    Auto(usize),
}

impl From<Dma> for spi_dma_chan_t {
    fn from(dma: Dma) -> Self {
        match dma {
            Dma::Channel1(_) => 1,
            Dma::Channel2(_) => 2,
            Dma::Auto(_) => 3,
            _ => 0,
        }
    }
}

impl Dma {
    pub const fn max_transfer_size(&self) -> usize {
        let max_transfer_size = match self {
            Dma::Disabled => TRANS_LEN,
            Dma::Channel1(size) | Dma::Channel2(size) | Dma::Auto(size) => *size,
        };
        match max_transfer_size {
            0 => panic!("The max transfer size must be greater than 0"),
            x if x % 4 != 0 => panic!("The max transfer size must be a multiple of 4"),
            _ => max_transfer_size,
        }
    }
}

pub type SpiDriverConfig = config::DriverConfig;
pub type SpiConfig = config::Config;

/// SPI configuration
pub mod config {
    use crate::{interrupt::InterruptType, units::*};
    use enumset::EnumSet;
    use esp_idf_sys::*;

    use super::Dma;

    pub use embedded_hal::spi::{Mode, Phase, Polarity, MODE_0, MODE_1, MODE_2, MODE_3};

    pub struct V02Type<T>(pub T);

    impl From<V02Type<embedded_hal_0_2::spi::Polarity>> for Polarity {
        fn from(polarity: V02Type<embedded_hal_0_2::spi::Polarity>) -> Self {
            match polarity.0 {
                embedded_hal_0_2::spi::Polarity::IdleHigh => Polarity::IdleHigh,
                embedded_hal_0_2::spi::Polarity::IdleLow => Polarity::IdleLow,
            }
        }
    }

    impl From<V02Type<embedded_hal_0_2::spi::Phase>> for Phase {
        fn from(phase: V02Type<embedded_hal_0_2::spi::Phase>) -> Self {
            match phase.0 {
                embedded_hal_0_2::spi::Phase::CaptureOnFirstTransition => {
                    Phase::CaptureOnFirstTransition
                }
                embedded_hal_0_2::spi::Phase::CaptureOnSecondTransition => {
                    Phase::CaptureOnSecondTransition
                }
            }
        }
    }

    impl From<V02Type<embedded_hal_0_2::spi::Mode>> for Mode {
        fn from(mode: V02Type<embedded_hal_0_2::spi::Mode>) -> Self {
            Self {
                polarity: V02Type(mode.0.polarity).into(),
                phase: V02Type(mode.0.phase).into(),
            }
        }
    }

    /// Specify the communication mode with the device
    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    pub enum Duplex {
        /// Full duplex is the default
        Full,
        /// Half duplex in some cases
        Half,
        /// Use MOSI (=spid) for both sending and receiving data (implies half duplex)
        Half3Wire,
    }

    impl Duplex {
        pub fn as_flags(&self) -> u32 {
            match self {
                Duplex::Full => 0,
                Duplex::Half => SPI_DEVICE_HALFDUPLEX,
                Duplex::Half3Wire => SPI_DEVICE_HALFDUPLEX | SPI_DEVICE_3WIRE,
            }
        }
    }

    /// Specifies the order in which the bits of data should be transfered/received
    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    pub enum BitOrder {
        /// Most significant bit first (default)
        MsbFirst,
        /// Least significant bit first
        LsbFirst,
        /// Least significant bit first, when sending
        TxLsbFirst,
        /// Least significant bit first, when receiving
        RxLsbFirst,
    }

    impl BitOrder {
        pub fn as_flags(&self) -> u32 {
            match self {
                Self::MsbFirst => 0,
                Self::LsbFirst => SPI_DEVICE_BIT_LSBFIRST,
                Self::TxLsbFirst => SPI_DEVICE_TXBIT_LSBFIRST,
                Self::RxLsbFirst => SPI_DEVICE_RXBIT_LSBFIRST,
            }
        }
    }

    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    pub enum LineWidth {
        /// 1-bit, 2 wire duplex or 1 wire half-duplex
        Single,
        /// 2-bit, 2 wire half-duplex
        Dual,
        /// 4-bit, 4 wire half-duplex
        Quad,
    }

    /// SPI Driver configuration
    #[derive(Debug, Clone)]
    pub struct DriverConfig {
        pub dma: Dma,
        pub intr_flags: EnumSet<InterruptType>,
    }

    impl DriverConfig {
        pub fn new() -> Self {
            Default::default()
        }

        #[must_use]
        pub fn dma(mut self, dma: Dma) -> Self {
            self.dma = dma;
            self
        }

        #[must_use]
        pub fn intr_flags(mut self, intr_flags: EnumSet<InterruptType>) -> Self {
            self.intr_flags = intr_flags;
            self
        }
    }

    impl Default for DriverConfig {
        fn default() -> Self {
            Self {
                dma: Dma::Disabled,
                intr_flags: EnumSet::<InterruptType>::empty(),
            }
        }
    }

    /// SPI Device configuration
    #[derive(Debug, Clone)]
    pub struct Config {
        pub baudrate: Hertz,
        pub data_mode: Mode,
        /// This property can be set to configure a SPI Device for being write only
        /// Thus the flag SPI_DEVICE_NO_DUMMY will be passed on initialization and
        /// it will unlock the possibility of using 80Mhz as the bus freq
        /// See https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/peripherals/spi_master.html#timing-considerations
        pub write_only: bool,
        pub duplex: Duplex,
        pub bit_order: BitOrder,
        pub cs_active_high: bool,
        /// On Half-Duplex transactions: `cs_pre_delay_us % 16`  corresponds to the number of SPI bit-cycles cs should be activated before the transmission.
        /// On Full-Duplex transactions: `cs_pre_delay_us != 0`  will add 1 microsecond of cs activation before transmission
        pub cs_pre_delay_us: Option<u16>, // u16 as per the C struct has a uint16_t, cf: esp-idf/components/driver/spi/include/driver/spi_master.h spi_device_interface_config_t
        ///< Amount of SPI bit-cycles the cs should stay active after the transmission (0-16)
        pub cs_post_delay_us: Option<u8>, // u8 as per the C struct had a uint8_t, cf: esp-idf/components/driver/spi/include/driver/spi_master.h spi_device_interface_config_t
        pub input_delay_ns: i32,
        pub polling: bool,
        pub allow_pre_post_delays: bool,
        pub queue_size: usize,
    }

    impl Config {
        pub fn new() -> Self {
            Default::default()
        }

        #[must_use]
        pub fn baudrate(mut self, baudrate: Hertz) -> Self {
            self.baudrate = baudrate;
            self
        }

        #[must_use]
        pub fn data_mode(mut self, data_mode: Mode) -> Self {
            self.data_mode = data_mode;
            self
        }

        #[must_use]
        pub fn write_only(mut self, write_only: bool) -> Self {
            self.write_only = write_only;
            self
        }

        #[must_use]
        pub fn duplex(mut self, duplex: Duplex) -> Self {
            self.duplex = duplex;
            self
        }

        #[must_use]
        pub fn bit_order(mut self, bit_order: BitOrder) -> Self {
            self.bit_order = bit_order;
            self
        }

        #[must_use]
        pub fn cs_active_high(mut self) -> Self {
            self.cs_active_high = true;
            self
        }

        /// On Half-Duplex transactions: `cs_pre_delay_us % 16`  corresponds to the number of SPI bit-cycles cs should be activated before the transmission
        /// On Full-Duplex transactions: `cs_pre_delay_us != 0`  will add 1 microsecond of cs activation before transmission
        #[must_use]
        pub fn cs_pre_delay_us(mut self, delay_us: u16) -> Self {
            self.cs_pre_delay_us = Some(delay_us);
            self
        }

        /// Add an aditional Amount of SPI bit-cycles the cs should be activated after the transmission (0-16).
        /// This only works on half-duplex transactions.
        #[must_use]
        pub fn cs_post_delay_us(mut self, delay_us: u8) -> Self {
            self.cs_post_delay_us = Some(delay_us);
            self
        }

        #[must_use]
        pub fn input_delay_ns(mut self, input_delay_ns: i32) -> Self {
            self.input_delay_ns = input_delay_ns;
            self
        }

        #[must_use]
        pub fn polling(mut self, polling: bool) -> Self {
            self.polling = polling;
            self
        }

        #[must_use]
        pub fn allow_pre_post_delays(mut self, allow_pre_post_delays: bool) -> Self {
            self.allow_pre_post_delays = allow_pre_post_delays;
            self
        }

        #[must_use]
        pub fn queue_size(mut self, queue_size: usize) -> Self {
            self.queue_size = queue_size;
            self
        }
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                baudrate: Hertz(1_000_000),
                data_mode: embedded_hal::spi::MODE_0,
                write_only: false,
                cs_active_high: false,
                duplex: Duplex::Full,
                bit_order: BitOrder::MsbFirst,
                cs_pre_delay_us: None,
                cs_post_delay_us: None,
                input_delay_ns: 0,
                polling: true,
                allow_pre_post_delays: false,
                queue_size: 1,
            }
        }
    }

    impl From<&Config> for spi_device_interface_config_t {
        fn from(config: &Config) -> Self {
            Self {
                spics_io_num: -1,
                clock_speed_hz: config.baudrate.0 as i32,
                mode: data_mode_to_u8(config.data_mode),
                queue_size: config.queue_size as i32,
                flags: if config.write_only {
                    SPI_DEVICE_NO_DUMMY
                } else {
                    0_u32
                } | if config.cs_active_high {
                    SPI_DEVICE_POSITIVE_CS
                } else {
                    0_u32
                } | config.duplex.as_flags()
                    | config.bit_order.as_flags(),
                cs_ena_pretrans: config.cs_pre_delay_us.unwrap_or(0),
                cs_ena_posttrans: config.cs_post_delay_us.unwrap_or(0),
                ..Default::default()
            }
        }
    }

    fn data_mode_to_u8(data_mode: Mode) -> u8 {
        (((data_mode.polarity == Polarity::IdleHigh) as u8) << 1)
            | ((data_mode.phase == Phase::CaptureOnSecondTransition) as u8)
    }
}

/// SPI transaction operation.
///
/// This allows composition of SPI operations into a single bus transaction.
#[non_exhaustive]
#[derive(Debug, PartialEq, Eq)]
pub enum Operation<'a> {
    /// Read data into the provided buffer.
    Read(&'a mut [u8]),
    /// Read data into the provided buffer with the provided line width in half-duplex mode.
    ReadWithWidth(&'a mut [u8], LineWidth),
    /// Write data from the provided buffer, discarding read data.
    Write(&'a [u8]),
    /// Write data from the provided buffer, using the provided line width in half-duplex mode,
    /// discarding read data.
    WriteWithWidth(&'a [u8], LineWidth),
    /// Read data into the first buffer, while writing data from the second buffer.
    Transfer(&'a mut [u8], &'a [u8]),
    /// Write data out while reading data into the provided buffer.
    TransferInPlace(&'a mut [u8]),
    /// Delay for at least the specified number of nanoseconds.
    DelayNs(u32),
}

pub struct SpiDriver<'d> {
    host: u8,
    max_transfer_size: usize,
    #[allow(dead_code)]
    bus_async_lock: Mutex<EspRawMutex, ()>,
    _p: PhantomData<&'d mut ()>,
}

impl<'d> SpiDriver<'d> {
    /// Create new instance of SPI controller for SPI1
    ///
    /// SPI1 can only use fixed pin for SCLK, SDO and SDI as they are shared with SPI0.
    #[cfg(esp32)]
    pub fn new_spi1(
        _spi: SPI1<'d>,
        sclk: crate::gpio::Gpio6<'d>,
        sdo: crate::gpio::Gpio7<'d>,
        sdi: Option<crate::gpio::Gpio8<'d>>,
        config: &config::DriverConfig,
    ) -> Result<Self, EspError> {
        use crate::gpio::Pin;

        let max_transfer_size = Self::new_internal(
            SPI1::device(),
            Some(sclk.pin() as _),
            Some(sdo.pin() as _),
            sdi.map(|p| p.pin() as _),
            None,
            None,
            config,
        )?;

        Ok(Self {
            host: SPI1::device() as _,
            max_transfer_size,
            bus_async_lock: Mutex::new(()),
            _p: PhantomData,
        })
    }

    /// Create new instance of SPI controller for all others
    pub fn new<SPI: SpiAnyPins + 'd>(
        _spi: SPI,
        sclk: impl OutputPin + 'd,
        sdo: impl OutputPin + 'd,
        sdi: Option<impl InputPin + 'd>,
        config: &config::DriverConfig,
    ) -> Result<Self, EspError> {
        let max_transfer_size = Self::new_internal(
            SPI::device(),
            Some(sclk.pin() as _),
            Some(sdo.pin() as _),
            sdi.map(|p| p.pin() as _),
            None,
            None,
            config,
        )?;

        Ok(Self {
            host: SPI::device() as _,
            max_transfer_size,
            bus_async_lock: Mutex::new(()),
            _p: PhantomData,
        })
    }

    pub fn new_without_sclk<SPI: SpiAnyPins + 'd>(
        _spi: SPI,
        sdo: impl OutputPin + 'd,
        sdi: Option<impl InputPin + 'd>,
        config: &config::DriverConfig,
    ) -> Result<Self, EspError> {
        let max_transfer_size = Self::new_internal(
            SPI::device(),
            None,
            Some(sdo.pin() as _),
            sdi.map(|p| p.pin() as _),
            None,
            None,
            config,
        )?;

        Ok(Self {
            host: SPI::device() as _,
            max_transfer_size,
            bus_async_lock: Mutex::new(()),
            _p: PhantomData,
        })
    }

    pub fn new_dual<SPI: SpiAnyPins + 'd>(
        _spi: SPI,
        sclk: impl OutputPin + 'd,
        data0: impl InputPin + OutputPin + 'd,
        data1: impl InputPin + OutputPin + 'd,
        config: &config::DriverConfig,
    ) -> Result<Self, EspError> {
        let max_transfer_size = Self::new_internal(
            SPI::device(),
            Some(sclk.pin() as _),
            Some(data0.pin() as _),
            Some(data1.pin() as _),
            None,
            None,
            config,
        )?;

        Ok(Self {
            host: SPI::device() as _,
            max_transfer_size,
            bus_async_lock: Mutex::new(()),
            _p: PhantomData,
        })
    }

    pub fn new_quad<SPI: SpiAnyPins + 'd>(
        _spi: SPI,
        sclk: impl OutputPin + 'd,
        data0: impl InputPin + OutputPin + 'd,
        data1: impl InputPin + OutputPin + 'd,
        data2: impl InputPin + OutputPin + 'd,
        data3: impl InputPin + OutputPin + 'd,
        config: &config::DriverConfig,
    ) -> Result<Self, EspError> {
        let max_transfer_size = Self::new_internal(
            SPI::device(),
            Some(sclk.pin() as _),
            Some(data0.pin() as _),
            Some(data1.pin() as _),
            Some(data2.pin() as _),
            Some(data3.pin() as _),
            config,
        )?;

        Ok(Self {
            host: SPI::device() as _,
            max_transfer_size,
            bus_async_lock: Mutex::new(()),
            _p: PhantomData,
        })
    }

    pub fn host(&self) -> spi_host_device_t {
        self.host as _
    }

    fn new_internal(
        host: spi_host_device_t,
        sclk: Option<i32>,
        sdo: Option<i32>,
        sdi: Option<i32>,
        data2: Option<i32>,
        data3: Option<i32>,
        config: &config::DriverConfig,
    ) -> Result<usize, EspError> {
        let max_transfer_sz = config.dma.max_transfer_size();
        let dma_chan: spi_dma_chan_t = config.dma.into();

        #[cfg(not(esp_idf_version_at_least_6_0_0))]
        #[allow(clippy::needless_update)]
        let bus_config = spi_bus_config_t {
            flags: SPICOMMON_BUSFLAG_MASTER,
            sclk_io_num: sclk.unwrap_or(-1),

            data4_io_num: -1,
            data5_io_num: -1,
            data6_io_num: -1,
            data7_io_num: -1,
            __bindgen_anon_1: spi_bus_config_t__bindgen_ty_1 {
                mosi_io_num: sdo.unwrap_or(-1),
                //data0_io_num: -1,
            },
            __bindgen_anon_2: spi_bus_config_t__bindgen_ty_2 {
                miso_io_num: sdi.unwrap_or(-1),
                //data1_io_num: -1,
            },
            __bindgen_anon_3: spi_bus_config_t__bindgen_ty_3 {
                quadwp_io_num: data2.unwrap_or(-1),
                //data2_io_num: -1,
            },
            __bindgen_anon_4: spi_bus_config_t__bindgen_ty_4 {
                quadhd_io_num: data3.unwrap_or(-1),
                //data3_io_num: -1,
            },
            max_transfer_sz: max_transfer_sz as i32,
            intr_flags: InterruptType::to_native(config.intr_flags) as _,
            ..Default::default()
        };

        #[cfg(esp_idf_version_at_least_6_0_0)]
        #[allow(clippy::needless_update)]
        let bus_config = spi_bus_config_t {
            __bindgen_anon_1: spi_bus_config_t__bindgen_ty_1 {
                __bindgen_anon_1: spi_bus_config_t__bindgen_ty_1__bindgen_ty_1 {
                    sclk_io_num: sclk.unwrap_or(-1),
                    data4_io_num: -1,
                    data5_io_num: -1,
                    data6_io_num: -1,
                    data7_io_num: -1,
                    __bindgen_anon_1: spi_bus_config_t__bindgen_ty_1__bindgen_ty_1__bindgen_ty_1 {
                        mosi_io_num: sdo.unwrap_or(-1),
                        //data0_io_num: -1,
                    },
                    __bindgen_anon_2: spi_bus_config_t__bindgen_ty_1__bindgen_ty_1__bindgen_ty_2 {
                        miso_io_num: sdi.unwrap_or(-1),
                        //data1_io_num: -1,
                    },
                    __bindgen_anon_3: spi_bus_config_t__bindgen_ty_1__bindgen_ty_1__bindgen_ty_3 {
                        quadwp_io_num: data2.unwrap_or(-1),
                        //data2_io_num: -1,
                    },
                    __bindgen_anon_4: spi_bus_config_t__bindgen_ty_1__bindgen_ty_1__bindgen_ty_4 {
                        quadhd_io_num: data3.unwrap_or(-1),
                        //data3_io_num: -1,
                    },
                },
            },

            flags: SPICOMMON_BUSFLAG_MASTER,
            max_transfer_sz: max_transfer_sz as i32,
            intr_flags: InterruptType::to_native(config.intr_flags) as _,
            ..Default::default()
        };

        esp!(unsafe { spi_bus_initialize(host, &bus_config, dma_chan) })?;

        Ok(max_transfer_sz)
    }
}

impl Drop for SpiDriver<'_> {
    fn drop(&mut self) {
        esp!(unsafe { spi_bus_free(self.host()) }).unwrap();
    }
}

unsafe impl Send for SpiDriver<'_> {}

pub struct SpiBusDriver<'d, T>
where
    T: BorrowMut<SpiDriver<'d>>,
{
    lock: Option<BusLock>,
    handle: spi_device_handle_t,
    driver: T,
    duplex: Duplex,
    polling: bool,
    queue_size: usize,
    _d: PhantomData<&'d ()>,
}

impl<'d, T> SpiBusDriver<'d, T>
where
    T: BorrowMut<SpiDriver<'d>>,
{
    pub fn new(driver: T, config: &config::Config) -> Result<Self, EspError> {
        let mut conf: spi_device_interface_config_t = config.into();
        conf.post_cb = Some(spi_notify);

        let mut handle: spi_device_handle_t = ptr::null_mut();
        esp!(unsafe { spi_bus_add_device(driver.borrow().host(), &conf, &mut handle as *mut _) })?;

        // From here on the device is registered on the bus, while `Self` - whose `Drop` is what
        // would remove it again - does not exist yet. Acquiring the bus lock below can fail, so
        // the registration has to be undone by hand: leaving the device attached would leak it
        // and, worse, make the eventual `spi_bus_free` of the owning `SpiDriver` fail with
        // `ESP_ERR_INVALID_STATE` ("not all CSses freed") long after the fact.
        let lock = match BusLock::new(handle) {
            Ok(lock) => lock,
            Err(err) => {
                // Infallible in practice: `handle` was just handed out by `spi_bus_add_device`
                // and cannot have transactions in flight yet, so a failure here is a bug.
                esp!(unsafe { spi_bus_remove_device(handle) }).unwrap();

                return Err(err);
            }
        };

        Ok(Self {
            lock: Some(lock),
            handle,
            driver,
            duplex: config.duplex,
            polling: config.polling,
            queue_size: config.queue_size,
            _d: PhantomData,
        })
    }

    pub fn read(&mut self, words: &mut [u8]) -> Result<(), EspError> {
        // Full-Duplex Mode:
        // The internal hardware 16*4 u8 FIFO buffer (shared for read/write) is not cleared
        // between transactions (read/write/transfer)
        // This can lead to rewriting the internal buffer to MOSI on a read call

        let chunk_size = self.driver.borrow().max_transfer_size;

        let transactions = spi_read_transactions(words, chunk_size, self.duplex, LineWidth::Single);
        spi_transmit(self.handle, transactions, self.polling, self.queue_size)?;

        Ok(())
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn read_async(&mut self, words: &mut [u8]) -> Result<(), EspError> {
        let chunk_size = self.driver.borrow().max_transfer_size;

        let transactions = spi_read_transactions(words, chunk_size, self.duplex, LineWidth::Single);
        core::pin::pin!(spi_transmit_async(
            self.handle,
            transactions,
            self.queue_size
        ))
        .await?;

        Ok(())
    }

    pub fn write(&mut self, words: &[u8]) -> Result<(), EspError> {
        // Full-Duplex Mode:
        // The internal hardware 16*4 u8 FIFO buffer (shared for read/write) is not cleared
        // between transactions ( read/write/transfer)
        // This can lead to re-reading the last internal buffer MOSI msg, in case the Slave fails to send a msg

        let chunk_size = self.driver.borrow().max_transfer_size;

        let transactions = spi_write_transactions(words, chunk_size, LineWidth::Single);
        spi_transmit(self.handle, transactions, self.polling, self.queue_size)?;

        Ok(())
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn write_async(&mut self, words: &[u8]) -> Result<(), EspError> {
        let chunk_size = self.driver.borrow().max_transfer_size;

        let transactions = spi_write_transactions(words, chunk_size, LineWidth::Single);
        core::pin::pin!(spi_transmit_async(
            self.handle,
            transactions,
            self.queue_size
        ))
        .await?;

        Ok(())
    }

    pub fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), EspError> {
        // In non-DMA mode, it will internally split the transfers every 64 bytes (max_transf_len).
        // - If the read and write buffers are not of the same length, it will first transfer the common buffer length
        // and then (separately aligned) the remaining buffer.
        // - Expect a delay time between every internally split (64-byte or remainder) package.

        // Half-Duplex & Half-3-Duplex Mode:
        // Data will be split into 64-byte write/read sections.
        // Example: write: [u8;96] - read [u8; 160]
        // Package 1: write 64, read 64 -> Package 2: write 32, read 32 -> Package 3: write 0, read 64.
        // Note that the first "package" is a 128-byte clock out while the later are respectively 64 bytes.

        let chunk_size = self.driver.borrow().max_transfer_size;

        let transactions = spi_transfer_transactions(read, write, chunk_size, self.duplex);
        spi_transmit(self.handle, transactions, self.polling, self.queue_size)?;

        Ok(())
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn transfer_async(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), EspError> {
        let chunk_size = self.driver.borrow().max_transfer_size;

        let transactions = spi_transfer_transactions(read, write, chunk_size, self.duplex);
        core::pin::pin!(spi_transmit_async(
            self.handle,
            transactions,
            self.queue_size
        ))
        .await?;

        Ok(())
    }

    pub fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), EspError> {
        let chunk_size = self.driver.borrow().max_transfer_size;

        let transactions = spi_transfer_in_place_transactions(words, chunk_size);
        spi_transmit(self.handle, transactions, self.polling, self.queue_size)?;

        Ok(())
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn transfer_in_place_async(&mut self, words: &mut [u8]) -> Result<(), EspError> {
        let chunk_size = self.driver.borrow().max_transfer_size;

        let transactions = spi_transfer_in_place_transactions(words, chunk_size);
        core::pin::pin!(spi_transmit_async(
            self.handle,
            transactions,
            self.queue_size
        ))
        .await?;

        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), EspError> {
        Ok(())
    }

    /// Run the provided [`Operation`] on the bus.
    ///
    /// Only Operations that result in a transfer are supported. For example,
    /// passing an [`Operation::DelayNs`] will return an error.
    pub fn operation(&mut self, operation: Operation<'_>) -> Result<(), EspError> {
        if let Operation::DelayNs(_) = operation {
            return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>());
        }

        let chunk_size = self.driver.borrow().max_transfer_size;
        let transactions = spi_operations(once(operation), chunk_size, self.duplex)
            .filter_map(|t| t.transaction());

        spi_transmit(self.handle, transactions, self.polling, self.queue_size)?;

        Ok(())
    }

    /// Run the provided [`Operation`] on the bus.
    ///
    /// Only Operations that result in a transfer are supported. For example,
    /// passing an [`Operation::DelayNs`] will return an error.
    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn operation_async(&mut self, operation: Operation<'_>) -> Result<(), EspError> {
        if let Operation::DelayNs(_) = operation {
            return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>());
        }

        let chunk_size = self.driver.borrow().max_transfer_size;
        let transactions = spi_operations(once(operation), chunk_size, self.duplex)
            .filter_map(|t| t.transaction());

        core::pin::pin!(spi_transmit_async(
            self.handle,
            transactions,
            self.queue_size
        ))
        .await?;

        Ok(())
    }
}

impl<'d, T> Drop for SpiBusDriver<'d, T>
where
    T: BorrowMut<SpiDriver<'d>>,
{
    fn drop(&mut self) {
        // Need to drop the lock first, because it holds the device
        // we are about to remove below
        self.lock = None;

        esp!(unsafe { spi_bus_remove_device(self.handle) }).unwrap();
    }
}

impl<'d, T> embedded_hal::spi::ErrorType for SpiBusDriver<'d, T>
where
    T: BorrowMut<SpiDriver<'d>>,
{
    type Error = SpiError;
}

impl<'d, T> SpiBus for SpiBusDriver<'d, T>
where
    T: BorrowMut<SpiDriver<'d>>,
{
    fn read(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        SpiBusDriver::read(self, words).map_err(to_spi_err)
    }

    fn write(&mut self, words: &[u8]) -> Result<(), Self::Error> {
        SpiBusDriver::write(self, words).map_err(to_spi_err)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        SpiBusDriver::flush(self).map_err(to_spi_err)
    }

    fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        SpiBusDriver::transfer(self, read, write).map_err(to_spi_err)
    }

    fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        SpiBusDriver::transfer_in_place(self, words).map_err(to_spi_err)
    }
}

#[cfg(not(esp_idf_spi_master_isr_in_iram))]
impl<'d, T> embedded_hal_async::spi::SpiBus for SpiBusDriver<'d, T>
where
    T: BorrowMut<SpiDriver<'d>>,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        SpiBusDriver::read_async(self, buf)
            .await
            .map_err(to_spi_err)
    }

    async fn write(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        SpiBusDriver::write_async(self, buf)
            .await
            .map_err(to_spi_err)
    }

    async fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        SpiBusDriver::transfer_async(self, read, write)
            .await
            .map_err(to_spi_err)
    }

    async fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        SpiBusDriver::transfer_in_place_async(self, words)
            .await
            .map_err(to_spi_err)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        SpiBusDriver::flush(self).map_err(to_spi_err)
    }
}

enum SpiOperation {
    Transaction(spi_transaction_t),
    Delay(u32),
}

impl SpiOperation {
    pub fn transaction(self) -> Option<spi_transaction_t> {
        if let Self::Transaction(transaction) = self {
            Some(transaction)
        } else {
            None
        }
    }
}

pub type SpiSingleDeviceDriver<'d> = SpiDeviceDriver<'d, SpiDriver<'d>>;

pub struct SpiDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    handle: spi_device_handle_t,
    driver: T,
    cs_pin_configured: bool,
    duplex: Duplex,
    polling: bool,
    allow_pre_post_delays: bool,
    queue_size: usize,
    _d: PhantomData<&'d ()>,
}

impl<'d> SpiDeviceDriver<'d, SpiDriver<'d>> {
    #[cfg(esp32)]
    pub fn new_single_spi1(
        spi: SPI1<'d>,
        sclk: crate::gpio::Gpio6<'d>,
        sdo: crate::gpio::Gpio7<'d>,
        sdi: Option<crate::gpio::Gpio8<'d>>,
        cs: Option<impl OutputPin + 'd>,
        bus_config: &config::DriverConfig,
        config: &config::Config,
    ) -> Result<Self, EspError> {
        Self::new(
            SpiDriver::new_spi1(spi, sclk, sdo, sdi, bus_config)?,
            cs,
            config,
        )
    }

    pub fn new_single<SPI: SpiAnyPins + 'd>(
        spi: SPI,
        sclk: impl OutputPin + 'd,
        sdo: impl OutputPin + 'd,
        sdi: Option<impl InputPin + 'd>,
        cs: Option<impl OutputPin + 'd>,
        bus_config: &config::DriverConfig,
        config: &config::Config,
    ) -> Result<Self, EspError> {
        Self::new(SpiDriver::new(spi, sclk, sdo, sdi, bus_config)?, cs, config)
    }
}

impl<'d, T> SpiDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    pub fn new(
        driver: T,
        cs: Option<impl OutputPin + 'd>,
        config: &config::Config,
    ) -> Result<Self, EspError> {
        let cs = cs.map(|cs| cs.pin() as _).unwrap_or(-1);

        let mut conf: spi_device_interface_config_t = config.into();
        conf.spics_io_num = cs;
        conf.post_cb = Some(spi_notify);

        let mut handle: spi_device_handle_t = ptr::null_mut();
        esp!(unsafe { spi_bus_add_device(driver.borrow().host(), &conf, &mut handle as *mut _) })?;

        Ok(Self {
            handle,
            driver,
            cs_pin_configured: cs >= 0,
            duplex: config.duplex,
            polling: config.polling,
            allow_pre_post_delays: config.allow_pre_post_delays,
            queue_size: config.queue_size,
            _d: PhantomData,
        })
    }

    pub fn device(&self) -> spi_device_handle_t {
        self.handle
    }

    pub fn transaction(&mut self, operations: &mut [Operation<'_>]) -> Result<(), EspError> {
        self.run(
            self.hardware_cs_ctl(operations.iter_mut().map(copy_operation))?,
            operations.iter_mut().map(copy_operation),
        )
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn transaction_async(
        &mut self,
        operations: &mut [Operation<'_>],
    ) -> Result<(), EspError> {
        core::pin::pin!(self.run_async(
            self.hardware_cs_ctl(operations.iter_mut().map(copy_operation))?,
            operations.iter_mut().map(copy_operation),
        ))
        .await
    }

    pub fn read(&mut self, read: &mut [u8]) -> Result<(), EspError> {
        self.transaction(&mut [Operation::Read(read)])
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn read_async(&mut self, read: &mut [u8]) -> Result<(), EspError> {
        let mut operation = [Operation::Read(read)];
        let work = core::pin::pin!(self.transaction_async(&mut operation));
        work.await
    }

    pub fn write(&mut self, write: &[u8]) -> Result<(), EspError> {
        self.transaction(&mut [Operation::Write(write)])
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn write_async(&mut self, write: &[u8]) -> Result<(), EspError> {
        let mut operation = [Operation::Write(write)];
        let work = core::pin::pin!(self.transaction_async(&mut operation));
        work.await
    }

    pub fn transfer_in_place(&mut self, buf: &mut [u8]) -> Result<(), EspError> {
        self.transaction(&mut [Operation::TransferInPlace(buf)])
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn transfer_in_place_async(&mut self, buf: &mut [u8]) -> Result<(), EspError> {
        let mut operation = [Operation::TransferInPlace(buf)];
        let work = core::pin::pin!(self.transaction_async(&mut operation));
        work.await
    }

    pub fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), EspError> {
        self.transaction(&mut [Operation::Transfer(read, write)])
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn transfer_async(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), EspError> {
        let mut operation = [Operation::Transfer(read, write)];
        let work = core::pin::pin!(self.transaction_async(&mut operation));
        work.await
    }

    fn run<'a, 'c, 'p, M>(
        &mut self,
        mut cs_pin: CsCtl<'c, 'p, M>,
        operations: impl Iterator<Item = Operation<'a>> + 'a,
    ) -> Result<(), EspError>
    where
        M: OutputMode,
    {
        let _lock = if cs_pin.needs_bus_lock() {
            Some(BusLock::new(self.device())?)
        } else {
            None
        };

        cs_pin.raise_cs()?;

        let mut spi_operations = self
            .spi_operations(operations)
            .enumerate()
            .map(|(index, mut operation)| {
                cs_pin.configure(&mut operation, index);
                operation
            })
            .peekable();

        let delay_impl = crate::delay::Delay::new_default();
        let mut result = Ok(());

        while spi_operations.peek().is_some() {
            if let Some(SpiOperation::Delay(delay)) = spi_operations.peek() {
                delay_impl.delay_us(*delay / 1000);
                spi_operations.next();
            } else {
                let transactions = core::iter::from_fn(|| {
                    spi_operations
                        .next_if(|operation| matches!(operation, SpiOperation::Transaction(_)))
                })
                .fuse()
                .filter_map(|operation| operation.transaction());

                result = spi_transmit(self.handle, transactions, self.polling, self.queue_size);

                if result.is_err() {
                    break;
                }
            }
        }

        cs_pin.lower_cs()?;

        result
    }

    #[allow(dead_code)]
    async fn run_async<'a, 'c, 'p, M>(
        &self,
        mut cs_pin: CsCtl<'c, 'p, M>,
        operations: impl Iterator<Item = Operation<'a>> + 'a,
    ) -> Result<(), EspError>
    where
        M: OutputMode,
    {
        let _async_bus_lock = if cs_pin.needs_bus_lock() {
            Some(self.driver.borrow().bus_async_lock.lock().await)
        } else {
            None
        };

        let _lock = if cs_pin.needs_bus_lock() {
            Some(BusLock::new(self.device())?)
        } else {
            None
        };

        cs_pin.raise_cs()?;

        let delay_impl = crate::delay::Delay::new_default(); // TODO: Need to wait asnchronously if in async mode
        let mut result = Ok(());

        let mut spi_operations = self
            .spi_operations(operations)
            .enumerate()
            .map(|(index, mut operation)| {
                cs_pin.configure(&mut operation, index);
                operation
            })
            .peekable();

        while spi_operations.peek().is_some() {
            if let Some(SpiOperation::Delay(delay)) = spi_operations.peek() {
                delay_impl.delay_us(*delay);
                spi_operations.next();
            } else {
                let transactions = core::iter::from_fn(|| {
                    spi_operations
                        .next_if(|operation| matches!(operation, SpiOperation::Transaction(_)))
                })
                .fuse()
                .filter_map(|operation| operation.transaction());

                result = core::pin::pin!(spi_transmit_async(
                    self.handle,
                    transactions,
                    self.queue_size
                ))
                .await;

                if result.is_err() {
                    break;
                }
            }
        }

        cs_pin.lower_cs()?;

        result
    }

    fn hardware_cs_ctl<'a, 'c, 'p>(
        &self,
        operations: impl Iterator<Item = Operation<'a>> + 'a,
    ) -> Result<CsCtl<'c, 'p, Output>, EspError> {
        let (total_count, transactions_count, first_transaction, last_transaction) =
            self.spi_operations_stats(operations);

        if !self.allow_pre_post_delays
            && self.cs_pin_configured
            && transactions_count > 0
            && (first_transaction != Some(0) || last_transaction != Some(total_count - 1))
        {
            Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>())?;
        }

        Ok(CsCtl::Hardware {
            enabled: self.cs_pin_configured,
            transactions_count,
            last_transaction,
        })
    }

    fn spi_operations_stats<'a>(
        &self,
        operations: impl Iterator<Item = Operation<'a>> + 'a,
    ) -> (usize, usize, Option<usize>, Option<usize>) {
        self.spi_operations(operations).enumerate().fold(
            (0, 0, None, None),
            |(total_count, transactions_count, first_transaction, last_transaction),
             (index, operation)| {
                if matches!(operation, SpiOperation::Transaction(_)) {
                    (
                        total_count + 1,
                        transactions_count + 1,
                        Some(first_transaction.unwrap_or(index)),
                        Some(index),
                    )
                } else {
                    (
                        total_count + 1,
                        transactions_count,
                        first_transaction,
                        last_transaction,
                    )
                }
            },
        )
    }

    fn spi_operations<'a>(
        &self,
        operations: impl Iterator<Item = Operation<'a>> + 'a,
    ) -> impl Iterator<Item = SpiOperation> + 'a {
        let chunk_size = self.driver.borrow().max_transfer_size;
        let duplex = self.duplex;
        spi_operations(operations, chunk_size, duplex)
    }
}

impl<'d, T> Drop for SpiDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    fn drop(&mut self) {
        esp!(unsafe { spi_bus_remove_device(self.handle) }).unwrap();
    }
}

unsafe impl<'d, T> Send for SpiDeviceDriver<'d, T> where T: Send + Borrow<SpiDriver<'d>> + 'd {}

impl<'d, T> embedded_hal::spi::ErrorType for SpiDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    type Error = SpiError;
}

impl<'d, T> SpiDevice for SpiDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        Self::read(self, buf).map_err(to_spi_err)
    }

    fn write(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        Self::write(self, buf).map_err(to_spi_err)
    }

    fn transaction(
        &mut self,
        operations: &mut [embedded_hal::spi::Operation<'_, u8>],
    ) -> Result<(), Self::Error> {
        self.run(
            self.hardware_cs_ctl(operations.iter_mut().map(copy_ehal_operation))?,
            operations.iter_mut().map(copy_ehal_operation),
        )
        .map_err(to_spi_err)
    }
}

impl<'d, T> embedded_hal_0_2::blocking::spi::Transfer<u8> for SpiDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    type Error = SpiError;

    fn transfer<'w>(&mut self, words: &'w mut [u8]) -> Result<&'w [u8], Self::Error> {
        self.transfer_in_place(words)?;

        Ok(words)
    }
}

impl<'d, T> embedded_hal_0_2::blocking::spi::Write<u8> for SpiDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    type Error = SpiError;

    fn write(&mut self, words: &[u8]) -> Result<(), Self::Error> {
        self.write(words).map_err(to_spi_err)
    }
}

/// All data is chunked into max(iter.len(), 64)
impl<'d, T> embedded_hal_0_2::blocking::spi::WriteIter<u8> for SpiDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    type Error = SpiError;

    fn write_iter<WI>(&mut self, words: WI) -> Result<(), Self::Error>
    where
        WI: IntoIterator<Item = u8>,
    {
        let mut lock = None;

        let mut words = words.into_iter().peekable();
        let mut buf = [0_u8; TRANS_LEN];

        loop {
            let mut offset = 0_usize;

            while offset < buf.len() {
                if let Some(word) = words.next() {
                    buf[offset] = word;
                    offset += 1;
                } else {
                    break;
                }
            }

            if offset == 0 {
                break;
            }

            let mut transaction = spi_create_transaction(
                core::ptr::null_mut(),
                buf[..offset].as_ptr(),
                offset,
                0,
                LineWidth::Single,
            );

            if lock.is_none() && words.peek().is_some() {
                lock = Some(BusLock::new(self.handle)?);
            }

            set_keep_cs_active(
                &mut transaction,
                self.cs_pin_configured && words.peek().is_some(),
            );

            spi_transmit(
                self.handle,
                once(transaction),
                self.polling,
                self.queue_size,
            )?;
        }

        Ok(())
    }
}

impl<'d, T> embedded_hal_0_2::blocking::spi::Transactional<u8> for SpiDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    type Error = SpiError;

    fn exec(
        &mut self,
        operations: &mut [embedded_hal_0_2::blocking::spi::Operation<'_, u8>],
    ) -> Result<(), Self::Error> {
        self.run(
            self.hardware_cs_ctl(operations.iter_mut().map(|op| match op {
                embedded_hal_0_2::blocking::spi::Operation::Write(words) => Operation::Write(words),
                embedded_hal_0_2::blocking::spi::Operation::Transfer(words) => {
                    Operation::TransferInPlace(words)
                }
            }))?,
            operations.iter_mut().map(|op| match op {
                embedded_hal_0_2::blocking::spi::Operation::Write(words) => Operation::Write(words),
                embedded_hal_0_2::blocking::spi::Operation::Transfer(words) => {
                    Operation::TransferInPlace(words)
                }
            }),
        )
        .map_err(to_spi_err)
    }
}

#[cfg(not(esp_idf_spi_master_isr_in_iram))]
impl<'d, T> embedded_hal_async::spi::SpiDevice for SpiDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        Self::read_async(self, buf).await.map_err(to_spi_err)
    }

    async fn write(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        Self::write_async(self, buf).await.map_err(to_spi_err)
    }

    async fn transaction(
        &mut self,
        operations: &mut [embedded_hal::spi::Operation<'_, u8>],
    ) -> Result<(), Self::Error> {
        core::pin::pin!(self.run_async(
            self.hardware_cs_ctl(operations.iter_mut().map(copy_ehal_operation))?,
            operations.iter_mut().map(copy_ehal_operation),
        ))
        .await
        .map_err(to_spi_err)
    }
}

pub struct SpiSharedDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    driver: UnsafeCell<SpiDeviceDriver<'d, T>>,
    lock: CriticalSection,
    #[allow(dead_code)]
    async_lock: Mutex<EspRawMutex, ()>,
}

impl<'d, T> SpiSharedDeviceDriver<'d, T>
where
    T: Borrow<SpiDriver<'d>> + 'd,
{
    pub fn new(driver: T, config: &config::Config) -> Result<Self, EspError> {
        Ok(Self::wrap(SpiDeviceDriver::new(
            driver,
            Option::<AnyOutputPin>::None,
            config,
        )?))
    }

    pub const fn wrap(device: SpiDeviceDriver<'d, T>) -> Self {
        Self {
            driver: UnsafeCell::new(device),
            lock: CriticalSection::new(),
            async_lock: Mutex::new(()),
        }
    }

    pub fn lock<R>(&self, f: impl FnOnce(&mut SpiDeviceDriver<'d, T>) -> R) -> R {
        let _guard = self.lock.enter();

        let device = unsafe { self.driver_mut() };

        f(device)
    }

    pub fn release(self) -> SpiDeviceDriver<'d, T> {
        self.driver.into_inner()
    }

    #[allow(clippy::mut_from_ref)]
    unsafe fn driver_mut(&self) -> &mut SpiDeviceDriver<'d, T> {
        &mut *self.driver.get()
    }
}

pub struct SpiSoftCsDeviceDriver<'d, DEVICE, DRIVER> {
    shared_device: DEVICE,
    cs_pin: PinDriver<'d, Output>,
    pre_delay_us: Option<u32>,
    post_delay_us: Option<u32>,
    _p: PhantomData<fn() -> DRIVER>,
}

impl<'d, DEVICE, DRIVER> SpiSoftCsDeviceDriver<'d, DEVICE, DRIVER>
where
    DEVICE: Borrow<SpiSharedDeviceDriver<'d, DRIVER>>,
    DRIVER: Borrow<SpiDriver<'d>> + 'd,
{
    pub fn new(
        shared_device: DEVICE,
        cs: impl OutputPin + 'd,
        cs_level: Level,
    ) -> Result<Self, EspError> {
        let mut cs_pin: PinDriver<Output> = PinDriver::output(cs)?;

        cs_pin.set_level(cs_level)?;

        Ok(Self {
            shared_device,
            cs_pin,
            pre_delay_us: None,
            post_delay_us: None,
            _p: PhantomData,
        })
    }

    /// Add an aditional Amount of SPI bit-cycles the cs should be activated before the transmission (0-16).
    /// This only works on half-duplex transactions.
    pub fn cs_pre_delay_us(&mut self, delay_us: u32) -> &mut Self {
        self.pre_delay_us = Some(delay_us);

        self
    }

    /// Add an aditional delay of x in uSeconds after transaction
    /// between last clk out and chip select
    pub fn cs_post_delay_us(&mut self, delay_us: u32) -> &mut Self {
        self.post_delay_us = Some(delay_us);

        self
    }

    pub fn transaction(&mut self, operations: &mut [Operation<'_>]) -> Result<(), EspError> {
        self.run(operations.iter_mut().map(copy_operation))
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn transaction_async(
        &mut self,
        operations: &mut [Operation<'_>],
    ) -> Result<(), EspError> {
        core::pin::pin!(self.run_async(operations.iter_mut().map(copy_operation))).await
    }

    pub fn read(&mut self, read: &mut [u8]) -> Result<(), EspError> {
        self.transaction(&mut [Operation::Read(read)])
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn read_async(&mut self, read: &mut [u8]) -> Result<(), EspError> {
        let mut operation = [Operation::Read(read)];
        let work = core::pin::pin!(self.transaction_async(&mut operation));
        work.await
    }

    pub fn write(&mut self, write: &[u8]) -> Result<(), EspError> {
        self.transaction(&mut [Operation::Write(write)])
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn write_async(&mut self, write: &[u8]) -> Result<(), EspError> {
        let mut operation = [Operation::Write(write)];
        let work = core::pin::pin!(self.transaction_async(&mut operation));
        work.await
    }

    pub fn transfer_in_place(&mut self, buf: &mut [u8]) -> Result<(), EspError> {
        self.transaction(&mut [Operation::TransferInPlace(buf)])
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn transfer_in_place_async(&mut self, buf: &mut [u8]) -> Result<(), EspError> {
        let mut operation = [Operation::TransferInPlace(buf)];
        let work = core::pin::pin!(self.transaction_async(&mut operation));
        work.await
    }

    pub fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), EspError> {
        self.transaction(&mut [Operation::Transfer(read, write)])
    }

    #[cfg(not(esp_idf_spi_master_isr_in_iram))]
    pub async fn transfer_async(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), EspError> {
        let mut operation = [Operation::Transfer(read, write)];
        let work = core::pin::pin!(self.transaction_async(&mut operation));
        work.await
    }

    fn run<'a>(
        &mut self,
        operations: impl Iterator<Item = Operation<'a>> + 'a,
    ) -> Result<(), EspError> {
        let cs_pin = CsCtl::Software {
            cs: &mut self.cs_pin,
            pre_delay: self.pre_delay_us,
            post_delay: self.post_delay_us,
        };

        self.shared_device
            .borrow()
            .lock(move |device| device.run(cs_pin, operations))
    }

    #[allow(dead_code)]
    async fn run_async<'a>(
        &mut self,
        operations: impl Iterator<Item = Operation<'a>> + 'a,
    ) -> Result<(), EspError> {
        let cs_pin = CsCtl::Software {
            cs: &mut self.cs_pin,
            pre_delay: self.pre_delay_us,
            post_delay: self.post_delay_us,
        };

        let device = self.shared_device.borrow();

        let _async_guard = device.async_lock.lock().await;
        let _guard = device.lock.enter();

        let driver = unsafe { device.driver_mut() };

        driver.run_async(cs_pin, operations).await
    }
}

impl<'d, DEVICE, DRIVER> embedded_hal::spi::ErrorType for SpiSoftCsDeviceDriver<'d, DEVICE, DRIVER>
where
    DEVICE: Borrow<SpiSharedDeviceDriver<'d, DRIVER>> + 'd,
    DRIVER: Borrow<SpiDriver<'d>> + 'd,
{
    type Error = SpiError;
}

impl<'d, DEVICE, DRIVER> SpiDevice for SpiSoftCsDeviceDriver<'d, DEVICE, DRIVER>
where
    DEVICE: Borrow<SpiSharedDeviceDriver<'d, DRIVER>> + 'd,
    DRIVER: Borrow<SpiDriver<'d>> + 'd,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        Self::read(self, buf).map_err(to_spi_err)
    }

    fn write(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        Self::write(self, buf).map_err(to_spi_err)
    }

    fn transaction(
        &mut self,
        operations: &mut [embedded_hal::spi::Operation<'_, u8>],
    ) -> Result<(), Self::Error> {
        self.run(operations.iter_mut().map(copy_ehal_operation))
            .map_err(to_spi_err)
    }
}

#[cfg(not(esp_idf_spi_master_isr_in_iram))]
impl<'d, DEVICE, DRIVER> embedded_hal_async::spi::SpiDevice
    for SpiSoftCsDeviceDriver<'d, DEVICE, DRIVER>
where
    DEVICE: Borrow<SpiSharedDeviceDriver<'d, DRIVER>> + 'd,
    DRIVER: Borrow<SpiDriver<'d>> + 'd,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        Self::read_async(self, buf).await.map_err(to_spi_err)
    }

    async fn write(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        Self::write_async(self, buf).await.map_err(to_spi_err)
    }

    async fn transaction(
        &mut self,
        operations: &mut [embedded_hal::spi::Operation<'_, u8>],
    ) -> Result<(), Self::Error> {
        core::pin::pin!(self.run_async(operations.iter_mut().map(copy_ehal_operation)))
            .await
            .map_err(to_spi_err)
    }
}

fn to_spi_err(err: EspError) -> SpiError {
    SpiError::other(err)
}

// Limit to 64, as we sometimes allocate a buffer of size TRANS_LEN on the stack, so we have to keep it small
// SOC_SPI_MAXIMUM_BUFFER_SIZE equals 64 or 72 (esp32s2) anyway
const TRANS_LEN: usize = if SOC_SPI_MAXIMUM_BUFFER_SIZE < 64_u32 {
    SOC_SPI_MAXIMUM_BUFFER_SIZE as _
} else {
    64_usize
};

// Whilst ESP-IDF doesn't have a documented maximum for queued transactions, we need a compile time
// max to be able to place the transactions on the stack (without recursion hacks) and not be
// forced to use box. Perhaps this is something the user can inject in, via generics or slice.
// This means a spi_device_interface_config_t.queue_size higher than this constant will be clamped
// down in practice.
const MAX_QUEUED_TRANSACTIONS: usize = 6;

struct BusLock(spi_device_handle_t);

impl BusLock {
    fn new(device: spi_device_handle_t) -> Result<Self, EspError> {
        esp!(unsafe { spi_device_acquire_bus(device, BLOCK) })?;

        Ok(Self(device))
    }
}

impl Drop for BusLock {
    fn drop(&mut self) {
        unsafe {
            spi_device_release_bus(self.0);
        }
    }
}

enum CsCtl<'c, 'p, M>
where
    M: OutputMode,
{
    Hardware {
        enabled: bool,
        transactions_count: usize,
        last_transaction: Option<usize>,
    },
    Software {
        cs: &'c mut PinDriver<'p, M>,
        pre_delay: Option<u32>,
        post_delay: Option<u32>,
    },
}

impl<M> CsCtl<'_, '_, M>
where
    M: OutputMode,
{
    fn needs_bus_lock(&self) -> bool {
        match self {
            Self::Hardware {
                transactions_count, ..
            } => *transactions_count > 1,
            Self::Software { .. } => true,
        }
    }

    fn raise_cs(&mut self) -> Result<(), EspError> {
        if let CsCtl::Software { cs, pre_delay, .. } = self {
            cs.toggle()?;

            // TODO: Need to wait asnchronously if in async mode
            if let Some(delay) = pre_delay {
                Ets::delay_us(*delay);
            }
        }

        Ok(())
    }

    fn lower_cs(&mut self) -> Result<(), EspError> {
        if let CsCtl::Software { cs, post_delay, .. } = self {
            cs.toggle()?;

            // TODO: Need to wait asnchronously if in async mode
            if let Some(delay) = post_delay {
                Ets::delay_us(*delay);
            }
        }

        Ok(())
    }

    fn configure(&self, operation: &mut SpiOperation, index: usize) {
        if let SpiOperation::Transaction(transaction) = operation {
            self.configure_transaction(transaction, index)
        }
    }

    fn configure_transaction(&self, transaction: &mut spi_transaction_t, index: usize) {
        if let Self::Hardware {
            enabled,
            last_transaction,
            ..
        } = self
        {
            set_keep_cs_active(transaction, *enabled && Some(index) != *last_transaction);
        }
    }
}

fn spi_operations<'a>(
    operations: impl Iterator<Item = Operation<'a>> + 'a,
    chunk_size: usize,
    duplex: Duplex,
) -> impl Iterator<Item = SpiOperation> + 'a {
    enum OperationsIter<R, W, T, I, D> {
        Read(R),
        Write(W),
        Transfer(T),
        TransferInPlace(I),
        Delay(D),
    }

    impl<R, W, T, I, D> Iterator for OperationsIter<R, W, T, I, D>
    where
        R: Iterator<Item = SpiOperation>,
        W: Iterator<Item = SpiOperation>,
        T: Iterator<Item = SpiOperation>,
        I: Iterator<Item = SpiOperation>,
        D: Iterator<Item = SpiOperation>,
    {
        type Item = SpiOperation;

        fn next(&mut self) -> Option<Self::Item> {
            match self {
                Self::Read(iter) => iter.next(),
                Self::Write(iter) => iter.next(),
                Self::Transfer(iter) => iter.next(),
                Self::TransferInPlace(iter) => iter.next(),
                Self::Delay(iter) => iter.next(),
            }
        }
    }

    operations.flat_map(move |op| match op {
        Operation::Read(words) => OperationsIter::Read(
            spi_read_transactions(words, chunk_size, duplex, LineWidth::Single)
                .map(SpiOperation::Transaction),
        ),
        Operation::ReadWithWidth(words, line_width) => OperationsIter::Read(
            spi_read_transactions(words, chunk_size, duplex, line_width)
                .map(SpiOperation::Transaction),
        ),
        Operation::Write(words) => OperationsIter::Write(
            spi_write_transactions(words, chunk_size, LineWidth::Single)
                .map(SpiOperation::Transaction),
        ),
        Operation::WriteWithWidth(words, line_width) => OperationsIter::Write(
            spi_write_transactions(words, chunk_size, line_width).map(SpiOperation::Transaction),
        ),
        Operation::Transfer(read, write) => OperationsIter::Transfer(
            spi_transfer_transactions(read, write, chunk_size, duplex)
                .map(SpiOperation::Transaction),
        ),
        Operation::TransferInPlace(words) => OperationsIter::TransferInPlace(
            spi_transfer_in_place_transactions(words, chunk_size).map(SpiOperation::Transaction),
        ),
        Operation::DelayNs(delay) => {
            OperationsIter::Delay(core::iter::once(SpiOperation::Delay(delay)))
        }
    })
}

fn spi_read_transactions(
    words: &mut [u8],
    chunk_size: usize,
    duplex: Duplex,
    line_width: LineWidth,
) -> impl Iterator<Item = spi_transaction_t> + '_ {
    words.chunks_mut(chunk_size).map(move |chunk| {
        spi_create_transaction(
            chunk.as_mut_ptr(),
            core::ptr::null(),
            if duplex == Duplex::Full {
                chunk.len()
            } else {
                0
            },
            chunk.len(),
            line_width,
        )
    })
}

fn spi_write_transactions(
    words: &[u8],
    chunk_size: usize,
    line_width: LineWidth,
) -> impl Iterator<Item = spi_transaction_t> + '_ {
    words.chunks(chunk_size).map(move |chunk| {
        spi_create_transaction(
            core::ptr::null_mut(),
            chunk.as_ptr(),
            chunk.len(),
            0,
            line_width,
        )
    })
}

fn spi_transfer_in_place_transactions(
    words: &mut [u8],
    chunk_size: usize,
) -> impl Iterator<Item = spi_transaction_t> + '_ {
    words.chunks_mut(chunk_size).map(|chunk| {
        spi_create_transaction(
            chunk.as_mut_ptr(),
            chunk.as_mut_ptr(),
            chunk.len(),
            chunk.len(),
            LineWidth::Single,
        )
    })
}

fn spi_transfer_transactions<'a>(
    read: &'a mut [u8],
    write: &'a [u8],
    chunk_size: usize,
    duplex: Duplex,
) -> impl Iterator<Item = spi_transaction_t> + 'a {
    enum OperationsIter<E, R, W> {
        Equal(E),
        ReadLonger(R),
        WriteLonger(W),
    }

    impl<E, R, W> Iterator for OperationsIter<E, R, W>
    where
        E: Iterator<Item = spi_transaction_t>,
        R: Iterator<Item = spi_transaction_t>,
        W: Iterator<Item = spi_transaction_t>,
    {
        type Item = spi_transaction_t;

        fn next(&mut self) -> Option<Self::Item> {
            match self {
                Self::Equal(iter) => iter.next(),
                Self::ReadLonger(iter) => iter.next(),
                Self::WriteLonger(iter) => iter.next(),
            }
        }
    }

    match read.len().cmp(&write.len()) {
        Ordering::Equal => {
            OperationsIter::Equal(spi_transfer_equal_transactions(read, write, chunk_size))
        }
        Ordering::Greater => {
            let (read, read_trail) = read.split_at_mut(write.len());

            OperationsIter::ReadLonger(
                spi_transfer_equal_transactions(read, write, chunk_size).chain(
                    spi_read_transactions(read_trail, chunk_size, duplex, LineWidth::Single),
                ),
            )
        }
        Ordering::Less => {
            let (write, write_trail) = write.split_at(read.len());

            OperationsIter::WriteLonger(
                spi_transfer_equal_transactions(read, write, chunk_size).chain(
                    spi_write_transactions(write_trail, chunk_size, LineWidth::Single),
                ),
            )
        }
    }
}

fn spi_transfer_equal_transactions<'a>(
    read: &'a mut [u8],
    write: &'a [u8],
    chunk_size: usize,
) -> impl Iterator<Item = spi_transaction_t> + 'a {
    read.chunks_mut(chunk_size)
        .zip(write.chunks(chunk_size))
        .map(|(read_chunk, write_chunk)| {
            spi_create_transaction(
                read_chunk.as_mut_ptr(),
                write_chunk.as_ptr(),
                max(read_chunk.len(), write_chunk.len()),
                read_chunk.len(),
                LineWidth::Single,
            )
        })
}

// These parameters assume full duplex.
fn spi_create_transaction(
    read: *mut u8,
    write: *const u8,
    transaction_length: usize,
    rx_length: usize,
    line_width: LineWidth,
) -> spi_transaction_t {
    let flags = match line_width {
        LineWidth::Single => 0,
        LineWidth::Dual => SPI_TRANS_MODE_DIO,
        LineWidth::Quad => SPI_TRANS_MODE_QIO,
    };
    spi_transaction_t {
        flags,
        __bindgen_anon_1: spi_transaction_t__bindgen_ty_1 {
            tx_buffer: write as *const _,
        },
        __bindgen_anon_2: spi_transaction_t__bindgen_ty_2 {
            rx_buffer: read as *mut _,
        },
        length: (transaction_length * 8) as _,
        rxlength: (rx_length * 8) as _,
        ..Default::default()
    }
}

fn set_keep_cs_active(transaction: &mut spi_transaction_t, _keep_cs_active: bool) {
    if _keep_cs_active {
        transaction.flags |= SPI_TRANS_CS_KEEP_ACTIVE
    }
}

fn spi_transmit(
    handle: spi_device_handle_t,
    transactions: impl Iterator<Item = spi_transaction_t>,
    polling: bool,
    queue_size: usize,
) -> Result<(), EspError> {
    if polling {
        for mut transaction in transactions {
            esp!(unsafe { spi_device_polling_transmit(handle, &mut transaction as *mut _) })?;
        }
    } else {
        pub type Queue = Deque<spi_transaction_t, MAX_QUEUED_TRANSACTIONS>;

        let mut queue = Queue::new();
        let queue_size = min(MAX_QUEUED_TRANSACTIONS, queue_size);

        let push = |queue: &mut Queue, transaction| {
            let _ = queue.push_back(transaction);
            esp!(unsafe { spi_device_queue_trans(handle, queue.back_mut().unwrap(), delay::BLOCK) })
        };

        let pop = |queue: &mut Queue| {
            let mut rtrans = ptr::null_mut();
            esp!(unsafe { spi_device_get_trans_result(handle, &mut rtrans, delay::BLOCK) })?;

            if rtrans != queue.front_mut().unwrap() {
                unreachable!();
            }
            queue.pop_front().unwrap();

            Ok(())
        };

        let pop_all = |queue: &mut Queue| {
            while !queue.is_empty() {
                pop(queue)?;
            }

            Ok(())
        };

        for transaction in transactions {
            if queue.len() == queue_size {
                // If the queue is full, we wait for the first transaction in the queue
                pop(&mut queue)?;
            }

            // Write transaction to a stable memory location
            push(&mut queue, transaction)?;
        }

        pop_all(&mut queue)?;
    }

    Ok(())
}

#[allow(dead_code)]
async fn spi_transmit_async(
    handle: spi_device_handle_t,
    transactions: impl Iterator<Item = spi_transaction_t>,
    queue_size: usize,
) -> Result<(), EspError> {
    pub type Queue = Deque<(spi_transaction_t, HalIsrNotification), MAX_QUEUED_TRANSACTIONS>;

    let mut queue = Queue::new();
    let queue = &mut queue;

    let queue_size = min(MAX_QUEUED_TRANSACTIONS, queue_size);
    let queued = Cell::new(0_usize);

    let fut = &mut core::pin::pin!(async {
        let push = |queue: &mut Queue, transaction| {
            queue
                .push_back((transaction, HalIsrNotification::new()))
                .map_err(|_| EspError::from_infallible::<{ ESP_ERR_INVALID_STATE }>())?;
            queued.set(queue.len());

            let last = queue.back_mut().unwrap();
            last.0.user = &last.1 as *const _ as *mut _;
            match esp!(unsafe { spi_device_queue_trans(handle, &mut last.0, delay::BLOCK) }) {
                Err(e) if e.code() == ESP_ERR_TIMEOUT => unreachable!(),
                other => other,
            }
        };

        let pop = |queue: &mut Queue, delay: TickType_t| {
            let mut rtrans = ptr::null_mut();
            match esp!(unsafe { spi_device_get_trans_result(handle, &mut rtrans, delay) }) {
                Err(e) if e.code() == ESP_ERR_TIMEOUT => return Ok(false),
                Err(e) => Err(e)?,
                Ok(()) => (),
            };

            if rtrans != &mut queue.front_mut().unwrap().0 {
                unreachable!();
            }

            queue.pop_front().unwrap();
            queued.set(queue.len());

            Ok(true)
        };

        // NOTE: after a successful `wait().await`, the front transaction result MUST be
        // fetched with a blocking `pop`: the SPI ISR invokes the post-transaction callback
        // (which fires the notification we just awaited) *before* it posts the result to
        // the driver return queue (see `spi_intr` in ESP-IDF's `spi_master.c`:
        // `spi_post_trans` runs before `xQueueSendFromISR`). On multi-core chips the
        // awoken task can thus observe the notification before the result is visible. As
        // the wait consumed the notification, going back to a non-blocking `pop` + `wait`
        // would hang forever. The blocking fetch is safe: the ISR posts the result right
        // after the callback.

        for transaction in transactions {
            // If the queue is full, we wait for the first transaction in the queue
            while queue.len() == queue_size {
                if !pop(queue, delay::NON_BLOCK)? {
                    queue.front_mut().unwrap().1.wait().await;
                    pop(queue, delay::BLOCK)?;
                }
            }

            // Write transaction to a stable memory location
            push(queue, transaction)?;
        }

        while !queue.is_empty() {
            if !pop(queue, delay::NON_BLOCK)? {
                queue.front_mut().unwrap().1.wait().await;
                pop(queue, delay::BLOCK)?;
            }
        }

        Ok(())
    });

    with_completion(fut, |completed| {
        if !completed {
            for _ in 0..queued.get() {
                let mut rtrans = ptr::null_mut();
                esp!(unsafe { spi_device_get_trans_result(handle, &mut rtrans, delay::BLOCK) })
                    .unwrap();
            }
        }
    })
    .await
}

extern "C" fn spi_notify(transaction: *mut spi_transaction_t) {
    if let Some(transaction) = unsafe { transaction.as_ref() } {
        if let Some(notification) = unsafe {
            (transaction.user as *mut HalIsrNotification as *const HalIsrNotification).as_ref()
        } {
            notification.notify_lsb();
        }
    }
}

fn copy_operation<'b>(operation: &'b mut Operation<'_>) -> Operation<'b> {
    match operation {
        Operation::Read(read) => Operation::Read(read),
        Operation::ReadWithWidth(read, line_width) => Operation::ReadWithWidth(read, *line_width),
        Operation::Write(write) => Operation::Write(write),
        Operation::WriteWithWidth(write, line_width) => {
            Operation::WriteWithWidth(write, *line_width)
        }
        Operation::Transfer(read, write) => Operation::Transfer(read, write),
        Operation::TransferInPlace(write) => Operation::TransferInPlace(write),
        Operation::DelayNs(delay) => Operation::DelayNs(*delay),
    }
}

fn copy_ehal_operation<'b>(
    operation: &'b mut embedded_hal::spi::Operation<'_, u8>,
) -> Operation<'b> {
    match operation {
        embedded_hal::spi::Operation::Read(read) => Operation::Read(read),
        embedded_hal::spi::Operation::Write(write) => Operation::Write(write),
        embedded_hal::spi::Operation::Transfer(read, write) => Operation::Transfer(read, write),
        embedded_hal::spi::Operation::TransferInPlace(write) => Operation::TransferInPlace(write),
        embedded_hal::spi::Operation::DelayNs(delay) => Operation::DelayNs(*delay),
    }
}

#[allow(dead_code)]
async fn with_completion<F, D>(fut: F, dtor: D) -> F::Output
where
    F: Future,
    D: FnMut(bool),
{
    struct Completion<D>
    where
        D: FnMut(bool),
    {
        dtor: D,
        completed: bool,
    }

    impl<D> Drop for Completion<D>
    where
        D: FnMut(bool),
    {
        fn drop(&mut self) {
            (self.dtor)(self.completed);
        }
    }

    let mut completion = Completion {
        dtor,
        completed: false,
    };

    let result = fut.await;

    completion.completed = true;

    result
}

macro_rules! impl_spi {
    ($spi:ident: $device:expr) => {
        crate::impl_peripheral!($spi);

        impl Spi for $spi<'_> {
            #[inline(always)]
            fn device() -> spi_host_device_t {
                $device
            }
        }
    };
}

macro_rules! impl_spi_any_pins {
    ($spi:ident) => {
        impl SpiAnyPins for $spi<'_> {}
    };
}

impl_spi!(SPI1: spi_host_device_t_SPI1_HOST);
impl_spi!(SPI2: spi_host_device_t_SPI2_HOST);
#[cfg(any(esp32, esp32s2, esp32s3, esp32p4, esp32s31))]
impl_spi!(SPI3: spi_host_device_t_SPI3_HOST);

impl_spi_any_pins!(SPI2);
#[cfg(any(esp32, esp32s2, esp32s3, esp32p4, esp32s31))]
impl_spi_any_pins!(SPI3);
