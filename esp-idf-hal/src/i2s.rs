//! Driver for the Inter-IC Sound (I2S) peripheral(s).

use core::ffi::c_void;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

use esp_idf_sys::{esp, EspError, TickType_t};

#[cfg(esp_idf_version_major = "5")]
use esp_idf_sys::i2s_port_t;

#[cfg(not(esp_idf_version_major = "4"))]
use {
    core::ptr::null_mut,
    esp_idf_sys::{
        i2s_chan_config_t, i2s_chan_handle_t, i2s_channel_disable, i2s_channel_enable,
        i2s_channel_read, i2s_channel_register_event_callback, i2s_channel_write, i2s_del_channel,
        i2s_event_callbacks_t, i2s_event_data_t, i2s_new_channel,
    },
};

#[cfg(esp_idf_version_major = "4")]
use esp_idf_sys::{
    i2s_config_t, i2s_driver_install, i2s_driver_uninstall, i2s_read, i2s_start, i2s_stop,
    i2s_write,
};

#[cfg(not(esp_idf_version_major = "4"))]
use crate::interrupt::asynch::HalIsrNotification;
use crate::{delay, io::EspIOError};

// For v5+, we rely configuration options for PDM/TDM support.
// For v4, we have to examine the chip type.
#[cfg(any(
    all(
        not(esp_idf_version_major = "4"),
        any(esp_idf_soc_i2s_supports_pdm_rx, esp_idf_soc_i2s_supports_pdm_tx)
    ),
    all(esp_idf_version_major = "4", any(esp32, esp32s3, esp32c3, esp32c6))
))]
mod pdm;

mod std;

#[cfg(any(
    all(not(esp_idf_version_major = "4"), esp_idf_soc_i2s_supports_tdm),
    all(esp_idf_version_major = "4", any(esp32s3, esp32c3, esp32c6))
))]
mod tdm;

#[cfg(esp_idf_version_at_least_6_0_0)]
#[allow(non_camel_case_types)]
type i2s_port_t = i32;

/// I2S channel base configuration.
pub type I2sConfig = config::Config;

/// I2S configuration
pub mod config {
    #[cfg(any(
        all(
            not(esp_idf_version_major = "4"),
            any(esp_idf_soc_i2s_supports_pdm_rx, esp_idf_soc_i2s_supports_pdm_tx)
        ),
        all(esp_idf_version_major = "4", any(esp32, esp32s3, esp32c3, esp32c6))
    ))]
    pub use super::pdm::config::*;

    pub use super::std::config::*;

    #[cfg(any(
        all(not(esp_idf_version_major = "4"), esp_idf_soc_i2s_supports_tdm),
        all(esp_idf_version_major = "4", any(esp32s3, esp32c3, esp32c6))
    ))]
    pub use super::tdm::config::*;

    use esp_idf_sys::{
        i2s_mclk_multiple_t, i2s_mclk_multiple_t_I2S_MCLK_MULTIPLE_128,
        i2s_mclk_multiple_t_I2S_MCLK_MULTIPLE_256, i2s_mclk_multiple_t_I2S_MCLK_MULTIPLE_384,
        EspError, ESP_ERR_INVALID_ARG,
    };

    #[cfg(esp_idf_version_major = "5")]
    use esp_idf_sys::i2s_port_t;
    #[cfg(not(esp_idf_version_major = "4"))]
    use esp_idf_sys::{
        i2s_chan_config_t, i2s_clock_src_t, i2s_data_bit_width_t,
        i2s_mclk_multiple_t_I2S_MCLK_MULTIPLE_512, i2s_role_t, i2s_slot_bit_width_t,
        i2s_slot_mode_t,
    };

    #[cfg(esp_idf_version_major = "4")]
    use esp_idf_sys::{
        i2s_bits_per_chan_t, i2s_bits_per_sample_t, i2s_mode_t, i2s_mode_t_I2S_MODE_MASTER,
        i2s_mode_t_I2S_MODE_SLAVE,
    };

    #[cfg(not(any(
        esp_idf_version_major = "4",
        all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
        all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
        all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
    )))] // ESP-IDF 5.3 and later
    use esp_idf_sys::i2s_chan_config_t__bindgen_ty_1; // introduces union type over auto_clear

    #[cfg(esp_idf_version_at_least_6_0_0)]
    #[allow(non_camel_case_types)]
    type i2s_port_t = i32;

    /// The default number of DMA buffers to use.
    pub const DEFAULT_DMA_BUFFER_COUNT: u32 = 6;

    /// The default number of frames per DMA buffer.
    pub const DEFAULT_FRAMES_PER_DMA_BUFFER: u32 = 240;

    /// I2S clock source.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
    pub enum ClockSource {
        /// Use PLL_F160M as the source clock
        ///
        /// Not the default on `esp32p4`: ESP-IDF only compiles the `I2S_CLK_SRC_PLL_160M` arm
        /// of `i2s_ll_get_clk_src` under `CONFIG_ESP_REV_MIN_FULL >= 300`, so selecting it from
        /// a build targeting an earlier minimum revision aborts in `HAL_ASSERT`.
        ///
        /// Not available on `esp32s31`, whose I2S clock sources are XTAL, APLL and RC_FAST.
        #[cfg(not(any(esp32h2, esp32c2, esp32s31)))]
        #[cfg_attr(not(esp32p4), default)]
        Pll160M,

        /// Use PLL_F60M as the source clock
        #[cfg(esp32c2)]
        #[default]
        Pll60M,

        /// Use PLL_F64M as the source clock
        #[cfg(esp32h2)]
        #[default]
        Pll64M,

        /// Use XTAL as the source clock
        #[cfg(any(esp32p4, esp32s31))]
        #[default]
        Xtal,

        /// Use APLL as the source clock
        ///
        /// `esp32` and `esp32s2` are named explicitly next to the capability, because the
        /// `SOC_*` based cfgs are only emitted for ESP-IDF 5.0 and later.
        #[cfg(any(esp_idf_soc_i2s_supports_apll, esp32, esp32s2))]
        Apll,
    }

    impl ClockSource {
        #[cfg(not(esp_idf_version_major = "4"))]
        #[allow(clippy::unnecessary_cast)]
        pub(super) fn as_sdk(&self) -> i2s_clock_src_t {
            match self {
                #[cfg(not(any(esp32h2, esp32c2, esp32s31)))]
                Self::Pll160M => core::convert::TryInto::try_into(
                    esp_idf_sys::soc_module_clk_t_SOC_MOD_CLK_PLL_F160M,
                )
                .unwrap(),
                #[cfg(esp32c2)]
                Self::Pll60M => core::convert::TryInto::try_into(
                    esp_idf_sys::soc_module_clk_t_SOC_MOD_CLK_PLL_F60M,
                )
                .unwrap(),
                #[cfg(esp32h2)]
                Self::Pll64M => core::convert::TryInto::try_into(
                    esp_idf_sys::soc_module_clk_t_SOC_MOD_CLK_PLL_F64M,
                )
                .unwrap(),
                #[cfg(any(esp32p4, esp32s31))]
                Self::Xtal => {
                    core::convert::TryInto::try_into(esp_idf_sys::soc_module_clk_t_SOC_MOD_CLK_XTAL)
                        .unwrap()
                }
                #[cfg(any(esp_idf_soc_i2s_supports_apll, esp32, esp32s2))]
                Self::Apll => {
                    core::convert::TryInto::try_into(esp_idf_sys::soc_module_clk_t_SOC_MOD_CLK_APLL)
                        .unwrap()
                }
            }
        }
    }

    /// I2S common channel configuration.
    ///
    /// To create a custom configuration, use the builder pattern built-in to this struct. For example:
    /// ```
    /// use esp_idf_hal::i2s::config::{ChannelOpen, Config, Role};
    /// let config = Config::default().role(Role::Target).channels(ChannelOpen::Rx);
    /// ```
    ///
    /// The default configuration is:
    /// * [`role`][Config::role]: [`Role::Controller`] (master)
    /// * [`dma_buffer_count`][Config::dma_buffer_count]: 6 ([`DEFAULT_DMA_BUFFER_COUNT`])
    /// * [`frames_per_buffer`][Config::frames_per_buffer]: 240 ([`DEFAULT_FRAMES_PER_DMA_BUFFER`])
    /// * [`auto_clear`][Config::auto_clear]: `false`
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct Config {
        /// The role of this channel: controller (master) or target (slave)
        pub(super) role: Role,

        /// The number of DMA buffers number to use.
        pub(super) dma_buffer_count: u32,

        /// The number of I2S frames in one DMA buffer.
        pub(super) frames_per_buffer: u32,

        /// If true, the transmit buffer will be automatically cleared upon sending.
        pub(super) auto_clear: bool,
    }

    impl Default for Config {
        #[inline(always)]
        fn default() -> Self {
            Self::new()
        }
    }

    impl Config {
        #[inline(always)]
        /// Create a new Config with the default settings.
        pub const fn new() -> Self {
            Self {
                role: Role::Controller,
                dma_buffer_count: DEFAULT_DMA_BUFFER_COUNT,
                frames_per_buffer: DEFAULT_FRAMES_PER_DMA_BUFFER,
                auto_clear: false,
            }
        }

        /// Set the role of this channel: controller (master) or target (slave).
        #[must_use]
        #[inline(always)]
        pub fn role(mut self, role: Role) -> Self {
            self.role = role;
            self
        }

        /// Set the number of DMA buffers to use.
        #[must_use]
        #[inline(always)]
        pub fn dma_buffer_count(mut self, dma_buffer_count: u32) -> Self {
            self.dma_buffer_count = dma_buffer_count;
            self
        }

        /// Set the number of I2S frames in one DMA buffer.
        #[must_use]
        #[inline(always)]
        pub fn frames_per_buffer(mut self, frames: u32) -> Self {
            self.frames_per_buffer = frames;
            self
        }

        /// Set if the transmit buffer will be automatically cleared upon sending.
        #[must_use]
        #[inline(always)]
        pub fn auto_clear(mut self, auto_clear: bool) -> Self {
            self.auto_clear = auto_clear;
            self
        }

        /// Convert to the ESP-IDF SDK `i2s_chan_config_t` representation.
        #[allow(clippy::needless_update)]
        #[cfg(not(esp_idf_version_major = "4"))]
        #[inline(always)]
        pub(super) fn as_sdk(&self, id: i2s_port_t) -> i2s_chan_config_t {
            i2s_chan_config_t {
                id,
                role: self.role.as_sdk(),
                dma_desc_num: self.dma_buffer_count,
                dma_frame_num: self.frames_per_buffer,
                #[cfg(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                ))]
                auto_clear: self.auto_clear,
                // ESP-IDF 5.3 introduced auto_clear for before_cb & after_cb 
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                )))] // For ESP-IDF v5.3 and later
                __bindgen_anon_1: i2s_chan_config_t__bindgen_ty_1{
                    auto_clear_after_cb: self.auto_clear,
                },
                ..Default::default()
            }
        }
    }

    /// Available data bit width in one slot.
    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    pub enum DataBitWidth {
        /// Channel data bit width is 8 bits.
        Bits8,

        /// Channel data bit width is 16 bits.
        Bits16,

        /// Channel data bit width is 24 bits.
        Bits24,

        /// Channel data bit width is 32 bits.
        Bits32,
    }

    impl From<DataBitWidth> for u32 {
        #[inline(always)]
        fn from(value: DataBitWidth) -> Self {
            match value {
                DataBitWidth::Bits8 => 8,
                DataBitWidth::Bits16 => 16,
                DataBitWidth::Bits24 => 24,
                DataBitWidth::Bits32 => 32,
            }
        }
    }

    impl DataBitWidth {
        /// Convert to the ESP-IDF SDK `i2s_data_bit_width_t` representation.
        #[cfg(not(esp_idf_version_major = "4"))]
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_data_bit_width_t {
            match self {
                Self::Bits8 => 8,
                Self::Bits16 => 16,
                Self::Bits24 => 24,
                Self::Bits32 => 32,
            }
        }

        /// Convert to the ESP-IDF SDK `i2s_bits_per_sample_t` representation.
        #[cfg(esp_idf_version_major = "4")]
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_bits_per_sample_t {
            match self {
                Self::Bits8 => 8,
                Self::Bits16 => 16,
                Self::Bits24 => 24,
                Self::Bits32 => 32,
            }
        }
    }

    impl TryFrom<usize> for DataBitWidth {
        type Error = EspError;

        fn try_from(value: usize) -> Result<Self, Self::Error> {
            match value {
                8 => Ok(Self::Bits8),
                16 => Ok(Self::Bits16),
                24 => Ok(Self::Bits24),
                32 => Ok(Self::Bits32),
                _ => Err(EspError::from(ESP_ERR_INVALID_ARG).unwrap()),
            }
        }
    }

    /// The multiple of MCLK to the sample rate.
    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    pub enum MclkMultiple {
        /// MCLK = sample rate * 128
        M128,

        /// MCLK = sample rate * 256
        M256,

        /// MCLK = sample rate * 384
        M384,

        /// MCLK = sample rate * 512
        #[cfg(not(esp_idf_version_major = "4"))]
        M512,
    }

    impl MclkMultiple {
        /// Convert to the ESP-IDF SDK `i2s_mclk_multiple_t` representation.
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_mclk_multiple_t {
            match self {
                Self::M128 => i2s_mclk_multiple_t_I2S_MCLK_MULTIPLE_128,
                Self::M256 => i2s_mclk_multiple_t_I2S_MCLK_MULTIPLE_256,
                Self::M384 => i2s_mclk_multiple_t_I2S_MCLK_MULTIPLE_384,
                #[cfg(not(esp_idf_version_major = "4"))]
                Self::M512 => i2s_mclk_multiple_t_I2S_MCLK_MULTIPLE_512,
            }
        }
    }

    impl From<MclkMultiple> for u32 {
        #[inline(always)]
        fn from(mclk_multiple: MclkMultiple) -> Self {
            match mclk_multiple {
                MclkMultiple::M128 => 128,
                MclkMultiple::M256 => 256,
                MclkMultiple::M384 => 384,
                #[cfg(not(esp_idf_version_major = "4"))]
                MclkMultiple::M512 => 512,
            }
        }
    }

    /// I2S channel operating role
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub enum Role {
        /// Controller (master)
        #[default]
        Controller,

        /// Target (slave)
        Target,
    }

    /// I2S peripheral in controller (master) role, bclk and ws signal will be set to output.
    #[cfg(not(esp_idf_version_major = "4"))]
    const I2S_ROLE_CONTROLLER: i2s_role_t = 0;

    /// I2S peripheral in target (slave) role, bclk and ws signal will be set to input.
    #[cfg(not(esp_idf_version_major = "4"))]
    const I2S_ROLE_TARGET: i2s_role_t = 1;

    impl Role {
        /// Convert to the ESP-IDF SDK `i2s_role_t` representation.
        #[cfg(not(esp_idf_version_major = "4"))]
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_role_t {
            match self {
                Self::Controller => I2S_ROLE_CONTROLLER,
                Self::Target => I2S_ROLE_TARGET,
            }
        }

        /// Convert to the ESP-IDF SDK `i2s_mode_t` representation.
        #[cfg(esp_idf_version_major = "4")]
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_mode_t {
            match self {
                Self::Controller => i2s_mode_t_I2S_MODE_MASTER,
                Self::Target => i2s_mode_t_I2S_MODE_SLAVE,
            }
        }
    }

    /// The total slot bit width in one slot.
    ///
    /// This is not necessarily the number of data bits in one slot. A slot may have additional bits padded to fill out
    /// the slot.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub enum SlotBitWidth {
        /// Slot bit width is automatically set to the data bit width.
        #[default]
        Auto,

        /// Slot bit width is 8 bits.
        Bits8,

        /// Slot bit width is 16 bits.
        Bits16,

        /// Slot bit width is 24 bits.
        Bits24,

        /// Slot bit width is 32 bits.
        Bits32,
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    type SlotBitWidthSdkType = i2s_slot_bit_width_t;

    #[cfg(esp_idf_version_major = "4")]
    type SlotBitWidthSdkType = i2s_bits_per_chan_t;

    impl SlotBitWidth {
        /// Convert this to the ESP-IDF SDK `i2s_slot_bit_width_t`/`i2s_bits_per_chan_t` representation.
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> SlotBitWidthSdkType {
            match self {
                Self::Auto => 0,
                Self::Bits8 => 8,
                Self::Bits16 => 16,
                Self::Bits24 => 24,
                Self::Bits32 => 32,
            }
        }
    }

    impl TryFrom<u32> for SlotBitWidth {
        type Error = EspError;

        fn try_from(value: u32) -> Result<Self, Self::Error> {
            match value {
                0 => Ok(Self::Auto),
                8 => Ok(Self::Bits8),
                16 => Ok(Self::Bits16),
                24 => Ok(Self::Bits24),
                32 => Ok(Self::Bits32),
                _ => Err(EspError::from(ESP_ERR_INVALID_ARG).unwrap()),
            }
        }
    }

    /// I2S channel slot mode.
    ///
    /// See the documentation for the mode of operation to see how this affects the data layout:
    /// * [PDM Rx][PdmRxSlotConfig]
    /// * [PDM Tx][PdmTxSlotConfig]
    /// * [Standard Rx/Tx][StdSlotConfig]
    /// * [TDM Rx/Tx][TdmSlotConfig]
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub enum SlotMode {
        /// Mono mode:
        /// * When transmitting, transmit the same data in all slots.
        /// * When receiving, only receive data from the first slot.
        Mono,

        /// Stereo mode:
        /// * When transmitting, transmit different data in each slot.
        /// * When receiving, receive data from all slots.
        #[default]
        Stereo,
    }

    impl SlotMode {
        /// Convert this to the ESP-IDF SDK `i2s_slot_mode_t` representation.
        #[cfg(not(esp_idf_version_major = "4"))]
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_slot_mode_t {
            match self {
                Self::Mono => 1,
                Self::Stereo => 2,
            }
        }
    }
}

/// Trait implemented by I2S peripherals to obtain their port number.
pub trait I2s: Send + sealed::Sealed {
    /// Return the port number for the peripheral.
    fn port() -> i2s_port_t;
}

mod sealed {
    pub trait Sealed {}

    impl Sealed for super::I2S0<'_> {}
    #[cfg(any(esp32, esp32s3, esp32p4, esp32s31))]
    impl Sealed for super::I2S1<'_> {}
    #[cfg(esp32p4)]
    impl Sealed for super::I2S2<'_> {}
}

pub trait I2sPort {
    /// Returns the I2S port number of this driver.
    fn port(&self) -> i2s_port_t;
}

/// Marker trait indicating that a driver supports receiving data via the [`I2sRx`] trait.
pub trait I2sRxSupported {}

/// Concrete implementation of [`I2sRxSupported`] for use in clients.
///
/// Example usage:
/// ```
/// use esp_idf_hal::gpio::*;
/// use esp_idf_hal::i2s::{config::{StdConfig, DataBitWidth}};
/// use esp_idf_hal::i2s::{I2sDriver, I2sRx};
/// let std_config = StdConfig::philips(48000, DataBitWidth::Bits16);
/// let peripherals = Peripherals::take().unwrap();
/// let bclk = peripherals.pins.gpio1;
/// let din = peripherals.pins.gpio4;
/// let mclk = AnyIOPin::none();
/// let ws = peripherals.pins.gpio2;
/// let i2s = I2sDriver::<I2sRx>::new_std_rx(peripherals.i2s0, &std_config, bclk, din, mclk, ws).unwrap();
/// ```
pub struct I2sRx {}
impl I2sRxSupported for I2sRx {}

/// Marker trait indicating that a driver supports transmitting data via the [`I2sTx`] trait.
pub trait I2sTxSupported {}

/// Concrete implementation of [`I2sTxSupported`] for use in clients.
///
/// Example usage:
/// ```
/// use esp_idf_hal::gpio::*;
/// use esp_idf_hal::i2s::{config::{StdConfig, DataBitWidth}};
/// use esp_idf_hal::i2s::{I2sDriver, I2sTx};
/// let std_config = StdConfig::philips(48000, DataBitWidth::Bits16);
/// let peripherals = Peripherals::take().unwrap();
/// let bclk = peripherals.pins.gpio1;
/// let dout = peripherals.pins.gpio6;
/// let mclk = AnyIOPin::none();
/// let ws = peripherals.pins.gpio2;
/// let i2s = I2sDriver::<I2sTx>::new_std_tx(peripherals.i2s0, &std_config, bclk, dout, mclk, ws).unwrap();
/// ```
pub struct I2sTx {}
impl I2sTxSupported for I2sTx {}

/// Concrete implementation of both [`I2sRxSupported`] and [`I2sTxSupported`] for use in clients.
///
/// Example usage:
/// ```
/// use esp_idf_hal::{gpio::*, peripherals::Peripherals};
/// use esp_idf_hal::i2s::{config::{StdConfig, DataBitWidth}};
/// use esp_idf_hal::i2s::{I2sDriver, I2sBiDir};
/// let std_config = StdConfig::philips(48000, DataBitWidth::Bits16);
/// let peripherals = Peripherals::take().unwrap();
/// let bclk = peripherals.pins.gpio1;
/// let din = peripherals.pins.gpio4;
/// let dout = peripherals.pins.gpio6;
/// let mclk = AnyIOPin::none();
/// let ws = peripherals.pins.gpio2;
/// let i2s = I2sDriver::<I2sBiDir>::new_std_bidir(peripherals.i2s0, &std_config, bclk, din, dout, mclk, ws).unwrap();
/// ```
pub struct I2sBiDir {}
impl I2sRxSupported for I2sBiDir {}
impl I2sTxSupported for I2sBiDir {}

/// Reference for I2S driver
pub struct I2sDriverRef<'d, Dir>(NonNull<I2sDriver<'d, Dir>>);

impl<'d, Dir> Deref for I2sDriverRef<'d, Dir> {
    type Target = I2sDriver<'d, Dir>;

    fn deref(&self) -> &Self::Target {
        unsafe { self.0.as_ref() }
    }
}

impl<Dir> DerefMut for I2sDriverRef<'_, Dir> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.0.as_mut() }
    }
}

/// Inter-IC Sound (I2S) driver.
pub struct I2sDriver<'d, Dir> {
    /// The Rx channel, possibly null.
    #[cfg(not(esp_idf_version_major = "4"))]
    rx_handle: i2s_chan_handle_t,

    /// The Tx channel, possibly null.
    #[cfg(not(esp_idf_version_major = "4"))]
    tx_handle: i2s_chan_handle_t,

    /// The I2S peripheral number. Either 0 (all devices) or 1 (ESP32 and ESP32-S3 only).
    port: u8,

    /// Driver lifetime -- mimics the lifetime of the peripheral.
    _p: PhantomData<&'d ()>,

    /// Directionality -- mimics the directionality of the peripheral.
    _dir: PhantomData<Dir>,
}

impl<Dir> I2sDriver<'_, Dir> {
    /// Create a new standard mode driver for the given I2S peripheral with both the receive and transmit channels open.
    #[cfg(not(esp_idf_version_major = "4"))]
    fn internal_new<I2S: I2s>(
        config: &i2s_chan_config_t,
        rx: bool,
        tx: bool,
    ) -> Result<Self, EspError> {
        let port = I2S::port();

        let mut rx_handle: i2s_chan_handle_t = null_mut();
        let mut tx_handle: i2s_chan_handle_t = null_mut();

        unsafe {
            esp!(i2s_new_channel(
                config,
                if tx {
                    &mut tx_handle as _
                } else {
                    core::ptr::null_mut()
                },
                if rx {
                    &mut rx_handle as _
                } else {
                    core::ptr::null_mut()
                },
            ))?
        };

        let mut this = Self {
            port: port as u8,
            rx_handle,
            tx_handle,
            _p: PhantomData,
            _dir: PhantomData,
        };

        this.subscribe_channel(this.rx_handle)?;
        this.subscribe_channel(this.tx_handle)?;

        Ok(this)
    }

    #[cfg(esp_idf_version_major = "4")]
    #[allow(clippy::too_many_arguments)]
    pub fn internal_new<I2S: I2s>(config: &i2s_config_t) -> Result<Self, EspError> {
        let port = I2S::port();

        unsafe {
            esp!(i2s_driver_install(port, config, 0, core::ptr::null_mut()))?;
        }

        Ok(Self {
            port: port as u8,
            _p: PhantomData,
            _dir: PhantomData,
        })
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    fn subscribe_channel(&mut self, handle: i2s_chan_handle_t) -> Result<(), EspError> {
        if !handle.is_null() {
            let callbacks = i2s_event_callbacks_t {
                on_recv: Some(dispatch_recv),
                on_recv_q_ovf: Some(dispatch_recv),
                on_sent: Some(dispatch_send),
                on_send_q_ovf: Some(dispatch_send),
                #[cfg(esp_idf_soc_i2s_supports_tx_fifo_sync)]
                on_tx_sync_evt: None,
            };

            // Safety: chan_handle is a valid pointer to an i2s_chan_handle_t and callbacks is initialized.
            esp!(unsafe {
                i2s_channel_register_event_callback(
                    handle,
                    &callbacks,
                    self.port as u32 as *mut core::ffi::c_void,
                )
            })?;
        }

        Ok(())
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    fn unsubscribe_channel(&mut self, handle: i2s_chan_handle_t) -> Result<(), EspError> {
        if !handle.is_null() {
            let callbacks = i2s_event_callbacks_t::default();

            // Safety: chan_handle is a valid pointer to an i2s_chan_handle_t and callbacks is initialized.
            esp!(unsafe {
                i2s_channel_register_event_callback(
                    handle,
                    &callbacks,
                    self.port as u32 as *mut core::ffi::c_void,
                )
            })?;
        }

        Ok(())
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    fn del_channel(&mut self, handle: i2s_chan_handle_t) -> Result<(), EspError> {
        if !handle.is_null() {
            let callbacks = i2s_event_callbacks_t::default();

            // Safety: chan_handle is a valid pointer to an i2s_chan_handle_t and callbacks is initialized.
            esp!(unsafe {
                i2s_channel_register_event_callback(handle, &callbacks, core::ptr::null_mut())
            })?;

            // Safety: chan_handle is a valid, non-null i2s_chan_handle_t.
            esp!(unsafe { i2s_del_channel(handle) })?;
        }

        Ok(())
    }

    fn remap_result(
        result: Result<(), EspError>,
        bytes_processed: usize,
    ) -> Result<usize, EspError> {
        match result {
            Ok(_) => Ok(bytes_processed),
            Err(err) if err.code() == esp_idf_sys::ESP_ERR_TIMEOUT && bytes_processed > 0 => {
                Ok(bytes_processed)
            }
            Err(err) => Err(err),
        }
    }

    /// Borrow the I2S driver by a reference
    pub fn as_ref(&mut self) -> I2sDriverRef<'_, Dir> {
        I2sDriverRef(unsafe { NonNull::new_unchecked(self) })
    }
}

/// Functions for receive channels.
impl<Dir> I2sDriver<'_, Dir>
where
    Dir: I2sRxSupported,
{
    /// Enable the I2S receive channel.
    ///
    /// # Note
    /// This can only be called when the channel is in the `READY` state: initialized but not yet started from a driver
    /// constructor, or disabled from the `RUNNING` state via [`rx_disable()`][I2sDriver::rx_disable]. The channel
    /// will enter the `RUNNING` state if it is enabled successfully.
    ///
    /// Enabling the channel will start I2S communications on the hardware. BCLK and WS signals will be generated if
    /// this is a controller. MCLK will be generated once initialization is finished.
    ///
    /// # Errors
    /// This will return an [`EspError`] with `ESP_ERR_INVALID_STATE` if the channel is not in the `READY` state.
    #[cfg(esp_idf_version_major = "4")]
    pub fn rx_enable(&mut self) -> Result<(), EspError> {
        unsafe { esp!(i2s_start(self.port as _)) }
    }

    /// Enable the I2S receive channel.
    ///
    /// # Note
    /// This can only be called when the channel is in the `READY` state: initialized but not yet started from a driver
    /// constructor, or disabled from the `RUNNING` state via [`rx_enable()`][I2sRxChannel::rx_disable]. The channel
    /// will enter the `RUNNING` state if it is enabled successfully.
    ///
    /// Enabling the channel will start I2S communications on the hardware. BCLK and WS signals will be generated if
    /// this is a controller. MCLK will be generated once initialization is finished.
    ///
    /// # Errors
    /// This will return an [`EspError`] with `ESP_ERR_INVALID_STATE` if the channel is not in the `READY` state.
    #[cfg(not(esp_idf_version_major = "4"))]
    pub fn rx_enable(&mut self) -> Result<(), EspError> {
        unsafe { esp!(i2s_channel_enable(self.rx_handle)) }
    }

    /// Disable the I2S receive channel.
    ///
    /// # Note
    /// This can only be called when the channel is in the `RUNNING` state: the channel has been previously enabled
    /// via a call to [`rx_enable()`][I2sRxChannel::rx_enable]. The channel will enter the `READY` state if it is
    /// disabled successfully.
    ///
    /// Disabling the channel will stop I2S communications on the hardware. BCLK and WS signals will stop being
    /// generated if this is a controller. MCLK will continue to be generated.
    ///
    /// # Errors
    /// This will return an [`EspError`] with `ESP_ERR_INVALID_STATE` if the channel is not in the `RUNNING` state.
    #[cfg(esp_idf_version_major = "4")]
    pub fn rx_disable(&mut self) -> Result<(), EspError> {
        unsafe { esp!(i2s_stop(self.port as _)) }
    }

    /// Disable the I2S receive channel.
    ///
    /// # Note
    /// This can only be called when the channel is in the `RUNNING` state: the channel has been previously enabled
    /// via a call to [`rx_enable()`][I2sRxChannel::rx_enable]. The channel will enter the `READY` state if it is
    /// disabled successfully.
    ///
    /// Disabling the channel will stop I2S communications on the hardware. BCLK and WS signals will stop being
    /// generated if this is a controller. MCLK will continue to be generated.
    ///
    /// # Errors
    /// This will return an [`EspError`] with `ESP_ERR_INVALID_STATE` if the channel is not in the `RUNNING` state.
    #[cfg(not(esp_idf_version_major = "4"))]
    pub fn rx_disable(&mut self) -> Result<(), EspError> {
        unsafe { esp!(i2s_channel_disable(self.rx_handle)) }
    }

    /// Read data from the channel asynchronously.
    ///
    /// This may be called only when the channel is in the `RUNNING` state.
    ///
    /// # Returns
    /// This returns the number of bytes read, or an [`EspError`] if an error occurred.
    #[cfg(not(esp_idf_version_major = "4"))]
    pub async fn read_async(&mut self, buffer: &mut [u8]) -> Result<usize, EspError> {
        loop {
            match self.read(buffer, crate::delay::NON_BLOCK) {
                Err(err) if err.code() == esp_idf_sys::ESP_ERR_TIMEOUT => {
                    RECV_NOTIFIER[self.port as usize].wait().await;
                }
                other => break other,
            }
        }
    }

    /// Read data from the channel.
    ///
    /// This may be called only when the channel is in the `RUNNING` state.
    ///
    /// # Returns
    /// This returns the number of bytes read, or an [`EspError`] if an error occurred.
    #[cfg(esp_idf_version_major = "4")]
    pub fn read(&mut self, buffer: &mut [u8], timeout: TickType_t) -> Result<usize, EspError> {
        if buffer.is_empty() {
            Ok(0)
        } else {
            let mut bytes_read: usize = 0;

            Self::remap_result(
                unsafe {
                    esp!(i2s_read(
                        self.port as _,
                        buffer.as_mut_ptr() as *mut c_void,
                        buffer.len(),
                        &mut bytes_read,
                        crate::delay::TickType(timeout).as_millis_u32(),
                    ))
                },
                bytes_read,
            )
        }
    }

    /// Read data from the channel.
    ///
    /// This may be called only when the channel is in the `RUNNING` state.
    ///
    /// # Returns
    /// This returns the number of bytes read, or an [`EspError`] if an error occurred.
    #[cfg(not(esp_idf_version_major = "4"))]
    pub fn read(&mut self, buffer: &mut [u8], timeout: TickType_t) -> Result<usize, EspError> {
        if buffer.is_empty() {
            Ok(0)
        } else {
            let mut bytes_read: usize = 0;

            Self::remap_result(
                unsafe {
                    esp!(i2s_channel_read(
                        self.rx_handle,
                        buffer.as_mut_ptr() as *mut c_void,
                        buffer.len(),
                        &mut bytes_read,
                        crate::delay::TickType(timeout).as_millis_u32(),
                    ))
                },
                bytes_read,
            )
        }
    }

    /// Read data from the channel into an uninitalized buffer asynchronously.
    ///
    /// This may be called only when the channel is in the `RUNNING` state.
    ///
    /// # Returns
    /// This returns the number of bytes read, or an [EspError] if an error occurred.
    ///
    /// # Safety
    /// Upon a successful return with `Ok(n_read)`, `buffer[..n_read]` will be initialized.
    #[cfg(not(esp_idf_version_major = "4"))]
    pub async fn read_uninit_async(
        &mut self,
        buffer: &mut [MaybeUninit<u8>],
    ) -> Result<usize, EspError> {
        loop {
            match self.read_uninit(buffer, crate::delay::NON_BLOCK) {
                Err(err) if err.code() == esp_idf_sys::ESP_ERR_TIMEOUT => {
                    RECV_NOTIFIER[self.port as usize].wait().await;
                }
                other => break other,
            }
        }
    }

    /// Read data from the channel into an uninitalized buffer.
    ///
    /// This may be called only when the channel is in the `RUNNING` state.
    ///
    /// # Returns
    /// This returns the number of bytes read, or an [`EspError`] if an error occurred.
    ///
    /// # Safety
    /// Upon a successful return with `Ok(n_read)`, `buffer[..n_read]` will be initialized.
    #[cfg(esp_idf_version_major = "4")]
    pub fn read_uninit(
        &mut self,
        buffer: &mut [MaybeUninit<u8>],
        timeout: TickType_t,
    ) -> Result<usize, EspError> {
        if buffer.is_empty() {
            Ok(0)
        } else {
            let mut bytes_read: usize = 0;

            Self::remap_result(
                unsafe {
                    esp!(i2s_read(
                        self.port as _,
                        buffer.as_mut_ptr() as *mut c_void,
                        buffer.len(),
                        &mut bytes_read,
                        crate::delay::TickType(timeout).as_millis_u32(),
                    ))
                },
                bytes_read,
            )
        }
    }

    /// Read data from the channel into an uninitalized buffer.
    ///
    /// This may be called only when the channel is in the `RUNNING` state.
    ///
    /// # Returns
    /// This returns the number of bytes read, or an [`EspError`] if an error occurred.
    ///
    /// # Safety
    /// Upon a successful return with `Ok(n_read)`, `buffer[..n_read]` will be initialized.
    #[cfg(not(esp_idf_version_major = "4"))]
    pub fn read_uninit(
        &mut self,
        buffer: &mut [MaybeUninit<u8>],
        timeout: TickType_t,
    ) -> Result<usize, EspError> {
        if buffer.is_empty() {
            Ok(0)
        } else {
            let mut bytes_read: usize = 0;

            Self::remap_result(
                unsafe {
                    esp!(i2s_channel_read(
                        self.rx_handle,
                        buffer.as_mut_ptr() as *mut c_void,
                        buffer.len(),
                        &mut bytes_read,
                        crate::delay::TickType(timeout).as_millis_u32(),
                    ))
                },
                bytes_read,
            )
        }
    }
}

/// Functions for transmit channels.
impl<Dir> I2sDriver<'_, Dir>
where
    Dir: I2sTxSupported,
{
    /// Enable the I2S transmit channel.
    ///
    /// # Note
    /// This can only be called when the channel is in the `READY` state: initialized but not yet started from a driver
    /// constructor, or disabled from the `RUNNING` state via [`tx_disable()`][I2sTxChannel::tx_disable]. The channel
    /// will enter the `RUNNING` state if it is enabled successfully.
    ///
    /// Enabling the channel will start I2S communications on the hardware. BCLK and WS signals will be generated if
    /// this is a controller. MCLK will be generated once initialization is finished.
    ///
    /// # Errors
    /// This will return an [`EspError`] with `ESP_ERR_INVALID_STATE` if the channel is not in the `READY` state.
    #[cfg(esp_idf_version_major = "4")]
    pub fn tx_enable(&mut self) -> Result<(), EspError> {
        unsafe { esp!(i2s_start(self.port as _)) }
    }

    /// Enable the I2S transmit channel.
    ///
    /// # Note
    /// This can only be called when the channel is in the `READY` state: initialized but not yet started from a driver
    /// constructor, or disabled from the `RUNNING` state via [`tx_disable()`][I2sTxChannel::tx_disable]. The channel
    /// will enter the `RUNNING` state if it is enabled successfully.
    ///
    /// Enabling the channel will start I2S communications on the hardware. BCLK and WS signals will be generated if
    /// this is a controller. MCLK will be generated once initialization is finished.
    ///
    /// # Errors
    /// This will return an [`EspError`] with `ESP_ERR_INVALID_STATE` if the channel is not in the `READY` state.
    #[cfg(not(esp_idf_version_major = "4"))]
    pub fn tx_enable(&mut self) -> Result<(), EspError> {
        unsafe { esp!(i2s_channel_enable(self.tx_handle)) }
    }

    /// Disable the I2S transmit channel.
    ///
    /// # Note
    /// This can only be called when the channel is in the `RUNNING` state: the channel has been previously enabled
    /// via a call to [`tx_enable()`][I2sTxChannel::tx_enable]. The channel will enter the `READY` state if it is
    /// disabled successfully.
    ///
    /// Disabling the channel will stop I2S communications on the hardware. BCLK and WS signals will stop being
    /// generated if this is a controller. MCLK will continue to be generated.
    ///
    /// # Errors
    /// This will return an [`EspError`] with `ESP_ERR_INVALID_STATE` if the channel is not in the `RUNNING` state.
    #[cfg(esp_idf_version_major = "4")]
    pub fn tx_disable(&mut self) -> Result<(), EspError> {
        unsafe { esp!(i2s_stop(self.port())) }
    }

    /// Disable the I2S transmit channel.
    ///
    /// # Note
    /// This can only be called when the channel is in the `RUNNING` state: the channel has been previously enabled
    /// via a call to [`tx_enable()`][I2sTxChannel::tx_enable]. The channel will enter the `READY` state if it is disabled
    /// successfully.
    ///
    /// Disabling the channel will stop I2S communications on the hardware. BCLK and WS signals will stop being
    /// generated if this is a controller. MCLK will continue to be generated.
    ///
    /// # Errors
    /// This will return an [`EspError`] with `ESP_ERR_INVALID_STATE` if the channel is not in the `RUNNING` state.
    #[cfg(not(esp_idf_version_major = "4"))]
    pub fn tx_disable(&mut self) -> Result<(), EspError> {
        unsafe { esp!(i2s_channel_disable(self.tx_handle)) }
    }

    /// Preload data into the transmit channel DMA buffer.
    ///
    /// This may be called only when the channel is in the `READY` state: initialized but not yet started.
    ///
    /// This is used to preload data into the DMA buffer so that valid data can be transmitted immediately after the
    /// channel is enabled via [`tx_enable()`][I2sTxChannel::tx_enable]. If this function is not called before enabling the channel,
    /// empty data will be transmitted.
    ///
    /// This function can be called multiple times before enabling the channel. Additional calls will concatenate the
    /// data to the end of the buffer until the buffer is full.
    ///
    /// # Returns
    /// This returns the number of bytes that have been loaded into the buffer. If this is less than the length of
    /// the data provided, the buffer is full and no more data can be loaded.
    #[cfg(all(
        not(esp_idf_version_major = "4"),
        not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0"))
    ))]
    pub fn preload_data(&mut self, data: &[u8]) -> Result<usize, EspError> {
        let mut bytes_loaded: usize = 0;

        unsafe {
            esp!(esp_idf_sys::i2s_channel_preload_data(
                self.tx_handle,
                data.as_ptr() as *const c_void,
                data.len(),
                &mut bytes_loaded as *mut usize
            ))?;
        }

        Ok(bytes_loaded)
    }

    /// Write data to the channel asynchronously.
    ///
    /// This may be called only when the channel is in the `RUNNING` state.
    ///
    /// # Returns
    /// This returns the number of bytes sent. This may be less than the length of the data provided.
    #[cfg(not(esp_idf_version_major = "4"))]
    pub async fn write_async(&mut self, data: &[u8]) -> Result<usize, EspError> {
        loop {
            match self.write(data, crate::delay::NON_BLOCK) {
                Err(err) if err.code() == esp_idf_sys::ESP_ERR_TIMEOUT => {
                    SEND_NOTIFIER[self.port as usize].wait().await;
                }
                other => break other,
            }
        }
    }

    /// Write all data to the channel asynchronously.
    ///
    /// This may be called only when the channel is in the `RUNNING` state.
    #[cfg(not(esp_idf_version_major = "4"))]
    pub async fn write_all_async(&mut self, data: &[u8]) -> Result<(), EspError> {
        let mut offset = 0;

        while offset < data.len() {
            offset += self.write_async(&data[offset..]).await?;
        }

        Ok(())
    }

    /// Write data to the channel.
    ///
    /// This may be called only when the channel is in the `RUNNING` state.
    ///
    /// # Returns
    /// This returns the number of bytes sent. This may be less than the length of the data provided.
    #[cfg(esp_idf_version_major = "4")]
    pub fn write(&mut self, data: &[u8], timeout: TickType_t) -> Result<usize, EspError> {
        if data.is_empty() {
            Ok(0)
        } else {
            let mut bytes_written: usize = 0;

            Self::remap_result(
                unsafe {
                    esp!(i2s_write(
                        self.port(),
                        data.as_ptr() as *mut c_void,
                        data.len(),
                        &mut bytes_written,
                        crate::delay::TickType(timeout).as_millis_u32(),
                    ))
                },
                bytes_written,
            )
        }
    }

    /// Write data to the channel.
    ///
    /// This may be called only when the channel is in the `RUNNING` state.
    ///
    /// # Returns
    /// This returns the number of bytes sent. This may be less than the length of the data provided.
    #[cfg(not(esp_idf_version_major = "4"))]
    pub fn write(&mut self, data: &[u8], timeout: TickType_t) -> Result<usize, EspError> {
        if data.is_empty() {
            Ok(0)
        } else {
            let mut bytes_written: usize = 0;

            Self::remap_result(
                unsafe {
                    esp!(i2s_channel_write(
                        self.tx_handle,
                        data.as_ptr() as *mut c_void,
                        data.len(),
                        &mut bytes_written,
                        crate::delay::TickType(timeout).as_millis_u32(),
                    ))
                },
                bytes_written,
            )
        }
    }

    /// Write all data to the channel.
    ///
    /// This may be called only when the channel is in the `RUNNING` state.
    pub fn write_all(&mut self, data: &[u8], timeout: TickType_t) -> Result<(), EspError> {
        let mut offset = 0;

        while offset < data.len() {
            offset += self.write(&data[offset..], timeout)?;
        }

        Ok(())
    }
}

impl I2sDriver<'_, I2sBiDir> {
    /// Split the bidirectional I2S driver into two parts (Rx, Tx)
    ///
    /// # Safety
    /// It is safe to use the two parts separately
    /// - esp-idf guarantees thread safety
    /// - esp-idf-hal guarantees asynchronous safety
    pub fn split(&mut self) -> (I2sDriverRef<'_, I2sRx>, I2sDriverRef<'_, I2sTx>) {
        // Safe because self cannot be null
        let this = unsafe { NonNull::new_unchecked(self) };

        (I2sDriverRef(this.cast()), I2sDriverRef(this.cast()))
    }
}

impl<Dir> Drop for I2sDriver<'_, Dir> {
    fn drop(&mut self) {
        #[cfg(esp_idf_version_major = "4")]
        {
            let _ = unsafe { esp!(i2s_stop(self.port as _)) };

            esp!(unsafe { i2s_driver_uninstall(self.port as _) }).unwrap();
        }

        #[cfg(not(esp_idf_version_major = "4"))]
        {
            if !self.rx_handle.is_null() {
                let _ = unsafe { esp!(i2s_channel_disable(self.rx_handle)) };
            }

            if !self.tx_handle.is_null() {
                let _ = unsafe { esp!(i2s_channel_disable(self.tx_handle)) };
            }

            self.unsubscribe_channel(self.rx_handle).unwrap();
            self.unsubscribe_channel(self.tx_handle).unwrap();

            if !self.rx_handle.is_null() {
                self.del_channel(self.rx_handle).unwrap();
            }

            if !self.tx_handle.is_null() {
                self.del_channel(self.tx_handle).unwrap();
            }

            SEND_NOTIFIER[self.port as usize].reset();
            RECV_NOTIFIER[self.port as usize].reset();
        }
    }
}

unsafe impl<Dir> Send for I2sDriver<'_, Dir> {}

impl<Dir> I2sPort for I2sDriver<'_, Dir> {
    fn port(&self) -> i2s_port_t {
        self.port as _
    }
}

impl<Dir> embedded_io::ErrorType for I2sDriver<'_, Dir> {
    type Error = EspIOError;
}

impl<Dir> embedded_io::Read for I2sDriver<'_, Dir>
where
    Dir: I2sRxSupported,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.read(buf, delay::BLOCK).map_err(EspIOError)
    }
}

impl<Dir> embedded_io::Write for I2sDriver<'_, Dir>
where
    Dir: I2sTxSupported,
{
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.write(buf, delay::BLOCK).map_err(EspIOError)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(not(esp_idf_version_major = "4"))]
impl<Dir> embedded_io_async::Read for I2sDriver<'_, Dir>
where
    Dir: I2sRxSupported,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.read_async(buf).await.map_err(EspIOError)
    }
}

#[cfg(not(esp_idf_version_major = "4"))]
impl<Dir> embedded_io_async::Write for I2sDriver<'_, Dir>
where
    Dir: I2sTxSupported,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.write_async(buf).await.map_err(EspIOError)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// C-facing ISR dispatcher for on_send_* callbacks.
#[cfg(not(esp_idf_version_major = "4"))]
unsafe extern "C" fn dispatch_send(
    _handle: i2s_chan_handle_t,
    _raw_event: *mut i2s_event_data_t,
    user_ctx: *mut c_void,
) -> bool {
    let port = user_ctx as u32 as i2s_port_t;

    SEND_NOTIFIER[port as usize].notify_lsb()
}

/// C-facing ISR dispatcher for on_recv_* callbacks.
#[cfg(not(esp_idf_version_major = "4"))]
unsafe extern "C" fn dispatch_recv(
    _handle: i2s_chan_handle_t,
    _raw_event: *mut i2s_event_data_t,
    user_ctx: *mut c_void,
) -> bool {
    let port = user_ctx as u32 as i2s_port_t;

    RECV_NOTIFIER[port as usize].notify_lsb()
}

macro_rules! impl_i2s {
    ($i2s:ident: $port:expr) => {
        crate::impl_peripheral!($i2s);

        impl I2s for $i2s<'_> {
            #[inline(always)]
            fn port() -> i2s_port_t {
                $port
            }
        }
    };
}

impl_i2s!(I2S0: 0);
#[cfg(any(esp32, esp32s3, esp32p4, esp32s31))]
impl_i2s!(I2S1: 1);
#[cfg(esp32p4)]
impl_i2s!(I2S2: 2);

#[cfg(not(esp_idf_version_major = "4"))]
#[cfg(not(any(esp32, esp32s3, esp32p4, esp32s31)))]
static SEND_NOTIFIER: [HalIsrNotification; 1] = [HalIsrNotification::new()];
#[cfg(not(esp_idf_version_major = "4"))]
#[cfg(not(any(esp32, esp32s3, esp32p4, esp32s31)))]
static RECV_NOTIFIER: [HalIsrNotification; 1] = [HalIsrNotification::new()];

#[cfg(not(esp_idf_version_major = "4"))]
#[cfg(any(esp32, esp32s3, esp32s31))]
static SEND_NOTIFIER: [HalIsrNotification; 2] =
    [HalIsrNotification::new(), HalIsrNotification::new()];
#[cfg(not(esp_idf_version_major = "4"))]
#[cfg(any(esp32, esp32s3, esp32s31))]
static RECV_NOTIFIER: [HalIsrNotification; 2] =
    [HalIsrNotification::new(), HalIsrNotification::new()];

#[cfg(not(esp_idf_version_major = "4"))]
#[cfg(esp32p4)]
static SEND_NOTIFIER: [HalIsrNotification; 3] = [
    HalIsrNotification::new(),
    HalIsrNotification::new(),
    HalIsrNotification::new(),
];
#[cfg(not(esp_idf_version_major = "4"))]
#[cfg(esp32p4)]
static RECV_NOTIFIER: [HalIsrNotification; 3] = [
    HalIsrNotification::new(),
    HalIsrNotification::new(),
    HalIsrNotification::new(),
];
