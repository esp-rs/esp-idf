//! Pulse density modulation (PDM) driver for the ESP32 I2S peripheral.
//!
//! # Microcontroller support for PDM mode
//!
//! | Microcontroller    | PDM Rx           | PDM Tx                   |
//! |--------------------|------------------|--------------------------|
//! | ESP32              | I2S0             | I2S0, hardware version 1 |
//! | ESP32-S2           | _not supported_  | _not supported_          |
//! | ESP32-S3           | I2S0             | I2S0, hardware version 2 |
//! | ESP32-C2 (ESP8684) | _not supported_  | _not supported_          |
//! | ESP32-C3           | _not supported_* | I2S0, hardware version 2 |
//! | ESP32-C6           | _not supported_* | I2S0, hardware version 2 |
//! | ESP32-H2           | _not supported_* | I2S0, hardware version 2 |
//! | ESP32-P4           | I2S0             | I2S0, hardware version 2 | ????
//!
//! \* These microcontrollers have PDM Rx capabilities but lack a PDM-to-PCM decoder required by the ESP-IDF SDK.
//!
//! ## Hardware versions
//!
//! Hardware version 1 (ESP32) provides only a single output line, requiring external hardware to demultiplex stereo
//! signals in a time-critical manner; it is unlikely you will see accurate results here.
//!
//! Harware version 2 (all others with PDM Tx support) provide two output lines, allowing for separate left/right
//! channels.
//!
//! See the [`PdmTxSlotConfig documentation`][PdmTxSlotConfig] for more details.

use super::*;
use crate::gpio::*;

// Note on cfg settings:
// esp_idf_soc_i2s_hw_version_1 and esp_idf_soc_i2s_hw_version_2 are defined *only* for ESP-IDF v5.0+.
// When v4.4 support is needed, actual microcontroller names must be used: esp32 for esp_idf_soc_i2s_hw_version_1,
// any(esp32s3,esp32c3,esp32c6,esp32h2) for esp_idf_soc_i2s_hw_version_2.

#[cfg(esp_idf_version_major = "4")]
use esp_idf_sys::*;

pub(super) mod config {
    #[allow(unused)]
    use crate::{gpio::*, i2s::config::*};
    use esp_idf_sys::*;

    /// I2S pulse density modulation (PDM) downsampling mode.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub enum PdmDownsample {
        /// Downsample 8 samples.
        #[default]
        Samples8,

        /// Downsample 16 samples.
        Samples16,

        /// Maximum downsample rate.
        Max,
    }

    #[cfg(any(esp_idf_soc_i2s_supports_pdm_rx, esp32, esp32s3))]
    impl PdmDownsample {
        /// Convert to the ESP-IDF SDK `i2s_pdm_downsample_t` representation.
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_pdm_dsr_t {
            match self {
                Self::Samples8 => 0,
                Self::Samples16 => 1,
                Self::Max => 2,
            }
        }
    }

    /// Pulse density modulation (PDM) mode receive clock configuration for the I2S peripheral.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct PdmRxClkConfig {
        /// The sample rate in Hz.
        pub(super) sample_rate_hz: u32,

        /// The clock source.
        clk_src: ClockSource,

        /// The multiple of the MCLK signal to the sample rate.
        mclk_multiple: MclkMultiple,

        /// Downsampling rate mode.
        pub(super) downsample_mode: PdmDownsample,
    }

    impl PdmRxClkConfig {
        /// Create a PDM clock configuration with the specified sample rate in Hz. This will set the clock source to
        /// PLL_F160M, the MCLK multiple to 256 times the sample rate, and the downsampling mode to 8 samples.
        #[inline(always)]
        pub fn from_sample_rate_hz(rate: u32) -> Self {
            Self {
                sample_rate_hz: rate,
                clk_src: ClockSource::default(),
                mclk_multiple: MclkMultiple::M256,
                downsample_mode: PdmDownsample::Samples8,
            }
        }

        /// Set the clock source on this PDM receive clock configuration.
        #[inline(always)]
        pub fn clk_src(mut self, clk_src: ClockSource) -> Self {
            self.clk_src = clk_src;
            self
        }

        /// Set the MCLK multiple on this PDM receive clock configuration.
        #[inline(always)]
        pub fn mclk_multiple(mut self, mclk_multiple: MclkMultiple) -> Self {
            self.mclk_multiple = mclk_multiple;
            self
        }

        /// Set the downsampling mode on this PDM receive clock configuration.
        #[inline(always)]
        pub fn downsample_mode(mut self, downsample_mode: PdmDownsample) -> Self {
            self.downsample_mode = downsample_mode;
            self
        }

        /// Convert to the ESP-IDF SDK `i2s_pdm_rx_clk_config_t` representation.
        #[cfg(all(
            any(esp_idf_soc_i2s_supports_pdm_rx, esp32, esp32s3),
            not(esp_idf_version_major = "4")
        ))]
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_pdm_rx_clk_config_t {
            #[allow(clippy::needless_update)]
            i2s_pdm_rx_clk_config_t {
                sample_rate_hz: self.sample_rate_hz,
                clk_src: self.clk_src.as_sdk(),
                mclk_multiple: self.mclk_multiple.as_sdk(),
                dn_sample_mode: self.downsample_mode.as_sdk(),
                ..Default::default()
            }
        }
    }

    /// Pulse density modulation (PDM) mode receive configuration for the I2S peripheral.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct PdmRxConfig {
        /// The base channel configuration.
        pub(super) channel_cfg: Config,

        /// PDM mode channel receive clock configuration.
        pub(super) clk_cfg: PdmRxClkConfig,

        /// PDM mode channel slot configuration.
        pub(super) slot_cfg: PdmRxSlotConfig,

        /// PDM mode channel GPIO configuration.
        #[cfg(not(esp_idf_version_major = "4"))]
        pub(super) gpio_cfg: PdmRxGpioConfig,
    }

    impl PdmRxConfig {
        /// Create a new PDM mode receive configuration from the specified clock, slot, and GPIO configurations.
        pub fn new(
            channel_cfg: Config,
            clk_cfg: PdmRxClkConfig,
            slot_cfg: PdmRxSlotConfig,
            #[cfg(not(esp_idf_version_major = "4"))] gpio_cfg: PdmRxGpioConfig,
        ) -> Self {
            Self {
                channel_cfg,
                clk_cfg,
                slot_cfg,
                #[cfg(not(esp_idf_version_major = "4"))]
                gpio_cfg,
            }
        }

        /// Convert just the clock config to the SDK representation. Used by
        /// the runtime `reconfigure_pdm` paths that don't touch GPIO.
        #[cfg(esp_idf_soc_i2s_supports_pdm_rx)]
        #[inline(always)]
        pub(crate) fn clk_cfg_as_sdk(&self) -> i2s_pdm_rx_clk_config_t {
            self.clk_cfg.as_sdk()
        }

        /// Convert just the slot config to the SDK representation. Used by
        /// the runtime `reconfigure_pdm` paths that don't touch GPIO.
        #[cfg(esp_idf_soc_i2s_supports_pdm_rx)]
        #[inline(always)]
        pub(crate) fn slot_cfg_as_sdk(&self) -> i2s_pdm_rx_slot_config_t {
            self.slot_cfg.as_sdk()
        }

        /// Convert this PDM mode receive configuration into the ESP-IDF SDK `i2s_pdm_rx_config_t` representation.
        #[cfg(esp_idf_soc_i2s_supports_pdm_rx)]
        #[inline(always)]
        pub(super) fn as_sdk<'d>(
            &self,
            clk: impl OutputPin + 'd,
            din: impl InputPin + 'd,
        ) -> i2s_pdm_rx_config_t {
            i2s_pdm_rx_config_t {
                clk_cfg: self.clk_cfg.as_sdk(),
                slot_cfg: self.slot_cfg.as_sdk(),
                gpio_cfg: self.gpio_cfg.as_sdk(clk, din),
            }
        }

        /// Convert this PDM mode receive configuration into the ESP-IDF SDK `i2s_pdm_rx_config_t` representation.
        ///
        /// Supported on ESP-IDF 5.1+.
        #[cfg(all(
            esp_idf_soc_i2s_supports_pdm_rx, // Implicitly selects 5.0+
            not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0"))
        ))]
        #[inline(always)]
        pub(super) fn as_sdk_multi<'d, DIN: InputPin + 'd>(
            &self,
            clk: impl OutputPin + 'd,
            dins: &[DIN],
        ) -> i2s_pdm_rx_config_t {
            i2s_pdm_rx_config_t {
                clk_cfg: self.clk_cfg.as_sdk(),
                slot_cfg: self.slot_cfg.as_sdk(),
                gpio_cfg: self.gpio_cfg.as_sdk_multi(clk, dins),
            }
        }

        /// Convert this PDM mode receive configuration into the ESP-IDF SDK `i2s_driver_config_t` representation.
        #[cfg(all(any(esp32, esp32s3), esp_idf_version_major = "4"))]
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_driver_config_t {
            let chan_fmt = match self.slot_cfg.slot_mode {
                SlotMode::Stereo => i2s_channel_fmt_t_I2S_CHANNEL_FMT_RIGHT_LEFT,
                SlotMode::Mono => match self.slot_cfg.slot_mask {
                    PdmSlotMask::Both => i2s_channel_fmt_t_I2S_CHANNEL_FMT_RIGHT_LEFT,
                    PdmSlotMask::Left => i2s_channel_fmt_t_I2S_CHANNEL_FMT_ONLY_LEFT,
                    PdmSlotMask::Right => i2s_channel_fmt_t_I2S_CHANNEL_FMT_ONLY_RIGHT,
                },
            };

            i2s_driver_config_t {
                mode: self.channel_cfg.role.as_sdk()
                    | i2s_mode_t_I2S_MODE_RX
                    | i2s_mode_t_I2S_MODE_PDM,
                sample_rate: self.clk_cfg.sample_rate_hz,
                bits_per_sample: 16, // fixed for PDM,
                channel_format: chan_fmt,
                communication_format: 0,  // ?
                intr_alloc_flags: 1 << 1, // ESP_INTR_FLAG_LEVEL1
                dma_buf_count: self.channel_cfg.dma_buffer_count as i32,
                dma_buf_len: self.channel_cfg.frames_per_buffer as i32,
                #[cfg(any(esp32, esp32s2))]
                use_apll: matches!(self.clk_cfg.clk_src, ClockSource::Apll),
                #[cfg(not(any(esp32, esp32s2)))]
                use_apll: false,
                tx_desc_auto_clear: self.channel_cfg.auto_clear,
                fixed_mclk: 0,
                mclk_multiple: self.clk_cfg.mclk_multiple.as_sdk(),
                bits_per_chan: 16, // fixed for PDM

                // The following are TDM-only fields and are not present on chips that don't support TDM mode.
                // There's no cfg option for this (it's a constant in esp-idf-sys).
                #[cfg(not(any(esp32, esp32s2)))]
                chan_mask: 0,
                #[cfg(not(any(esp32, esp32s2)))]
                total_chan: 0,
                #[cfg(not(any(esp32, esp32s2)))]
                left_align: false,
                #[cfg(not(any(esp32, esp32s2)))]
                big_edin: false,
                #[cfg(not(any(esp32, esp32s2)))]
                bit_order_msb: false,
                #[cfg(not(any(esp32, esp32s2)))]
                skip_msk: true,
            }
        }
    }

    /// PDM mode GPIO (general purpose input/output) receive configuration.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct PdmRxGpioConfig {
        /// Whether the clock output is inverted.
        pub(super) clk_inv: bool,
    }

    /// The maximum number of data input pins that can be used in PDM mode.
    ///
    /// This is 1 on the ESP32 and 4 on the ESP32-S3.
    #[cfg(esp32)]
    pub const SOC_I2S_PDM_MAX_RX_LINES: usize = 1;

    /// The maximum number of data input pins that can be used in PDM mode.
    ///
    /// This is 1 on the ESP32 and 4 on the ESP32-S3.
    #[cfg(esp32s3)]
    pub const SOC_I2S_PDM_MAX_RX_LINES: usize = 4;

    impl PdmRxGpioConfig {
        /// Create a new PDM mode GPIO receive configuration with the specified inversion flag for the clock output.
        #[inline(always)]
        pub fn new(clk_inv: bool) -> Self {
            Self { clk_inv }
        }

        /// Set the clock inversion flag on this PDM GPIO configuration.
        #[inline(always)]
        pub fn clk_invert(mut self, clk_inv: bool) -> Self {
            self.clk_inv = clk_inv;
            self
        }

        /// Convert to the ESP-IDF SDK `i2s_pdm_rx_gpio_config_t` representation.
        ///
        /// Note: The bitfields are renamed in ESP-IDF 5.1+.
        #[cfg(all(
            esp_idf_soc_i2s_supports_pdm_rx,
            esp_idf_version_major = "5",
            esp_idf_version_minor = "0"
        ))]
        pub(crate) fn as_sdk<'d>(
            &self,
            clk: impl OutputPin + 'd,
            din: impl InputPin + 'd,
        ) -> i2s_pdm_rx_gpio_config_t {
            let invert_flags = i2s_pdm_rx_gpio_config_t__bindgen_ty_1 {
                _bitfield_1: i2s_pdm_rx_gpio_config_t__bindgen_ty_1::new_bitfield_1(
                    self.clk_inv as u32,
                ),
                ..Default::default()
            };

            i2s_pdm_rx_gpio_config_t {
                clk: clk.pin(),
                din: din.pin(),
                invert_flags,
            }
        }

        /// Convert to the ESP-IDF SDK `i2s_pdm_rx_gpio_config_t` representation.
        #[cfg(all(
            esp_idf_soc_i2s_supports_pdm_rx,
            not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0"))
        ))]
        pub(crate) fn as_sdk<'d>(
            &self,
            clk: impl OutputPin + 'd,
            din: impl InputPin + 'd,
        ) -> i2s_pdm_rx_gpio_config_t {
            #[allow(clippy::unnecessary_cast)]
            let mut dins: [gpio_num_t; SOC_I2S_PDM_MAX_RX_LINES as usize] =
                [-1; SOC_I2S_PDM_MAX_RX_LINES as usize];
            dins[0] = din.pin() as _;

            let pins = i2s_pdm_rx_gpio_config_t__bindgen_ty_1 { dins };

            let invert_flags = i2s_pdm_rx_gpio_config_t__bindgen_ty_2 {
                _bitfield_1: i2s_pdm_rx_gpio_config_t__bindgen_ty_2::new_bitfield_1(
                    self.clk_inv as u32,
                ),
                ..Default::default()
            };

            i2s_pdm_rx_gpio_config_t {
                clk: clk.pin() as _,
                __bindgen_anon_1: pins,
                invert_flags,
            }
        }

        /// Convert to the ESP-IDF SDK `i2s_pdm_rx_gpio_config_t` representation.
        ///
        /// This will ignore any din pins beyond [`SOC_I2S_PDM_MAX_RX_LINES`].
        ///
        /// Supported on ESP-IDF 5.1+ only.
        #[cfg(all(
            esp_idf_soc_i2s_supports_pdm_rx,
            not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0"))
        ))]
        pub(crate) fn as_sdk_multi<'d, DIN: InputPin + 'd>(
            &self,
            clk: impl OutputPin + 'd,
            dins: &[DIN],
        ) -> i2s_pdm_rx_gpio_config_t {
            #[allow(clippy::unnecessary_cast)]
            let mut din_pins: [gpio_num_t; SOC_I2S_PDM_MAX_RX_LINES as usize] =
                [-1; SOC_I2S_PDM_MAX_RX_LINES as usize];

            #[allow(clippy::unnecessary_cast)]
            for (i, din) in dins.iter().enumerate() {
                if i >= SOC_I2S_PDM_MAX_RX_LINES as usize {
                    break;
                }

                din_pins[i] = din.pin() as _;
            }

            let pins = i2s_pdm_rx_gpio_config_t__bindgen_ty_1 { dins: din_pins };

            let invert_flags = i2s_pdm_rx_gpio_config_t__bindgen_ty_2 {
                _bitfield_1: i2s_pdm_rx_gpio_config_t__bindgen_ty_2::new_bitfield_1(
                    self.clk_inv as u32,
                ),
                ..Default::default()
            };

            i2s_pdm_rx_gpio_config_t {
                clk: clk.pin() as _,
                __bindgen_anon_1: pins,
                invert_flags,
            }
        }
    }

    /// PDM mode channel receive slot configuration.
    ///
    /// # Note
    /// The `slot_mode` and `slot_mask` cause data to be interpreted in different ways, as noted below.
    /// WS is the "word select" signal, sometimes called LRCLK (left/right clock).
    ///
    /// Assuming the received data contains the following samples (when converted from PDM to PCM), where a sample may be 8, 16, 24, or 32 bits, depending on `data_bit_width`:
    ///
    /// | **WS Low**  | **WS High** | **WS Low**  | **WS High** | **WS Low**  | **WS High** | **WS Low**  | **WS High** |     |
    /// |-------------|-------------|-------------|-------------|-------------|-------------|-------------|-------------|-----|
    /// | 11          | 12          | 13          | 14          | 15          | 16          | 17          | 18          | ... |
    ///
    /// The actual data in the buffer will be (1-4 bytes, depending on `data_bit_width`):
    ///
    /// <table>
    ///   <thead>
    ///     <tr><th><code>slot_mode</code></th><th><code>slot_mask</code></th><th colspan=8>Buffer Contents</th></tr>
    ///     <tr><th></th><th></th><th><code>d[0]</code></th><th><code>d[1]</code></th><th><code>d[2]</code></th><th><code>d[3]</code></th><th><code>d[4]</code></th><th><code>d[5]</code></th><th><code>d[6]</code></th><th><code>d[7]</code></th></tr>
    ///   </thead>
    ///   <tbody>
    ///     <tr><td rowspan=3><code>Mono</code></td>   <td><code>Left</code></td> <td>11</td><td>13</td><td>15</td><td>17</td><td>19</td><td>21</td><td>23</td><td>25</td></tr>
    ///     <tr>                                       <td><code>Right</code></td><td>12</td><td>14</td><td>16</td><td>18</td><td>20</td><td>22</td><td>24</td><td>26</td></tr>
    ///     <tr>                                       <td><code>Both</code></td> <td colspan=8><i>Unspecified behavior</i></td></tr>
    ///     <tr><td><code>Stereo (ESP32)</code></td>   <td><i>Any</i></td>        <td>11</td><td>12</td><td>13</td><td>14</td><td>15</td><td>16</td><td>17</td><td>18</td></tr>
    ///     <tr><td><code>Stereo (ESP32-S3)</code></td><td><i>Any</i></td>        <td>12</td><td>11</td><td>14</td><td>13</td><td>16</td><td>15</td><td>18</td><td>17</td></tr>
    ///   </tbody>
    /// </table>
    ///
    /// Note that, on the ESP32-S3, the right channel is received first. This can be switched by setting
    /// [`PdmRxGpioConfig::clk_invert`] to `true` in the merged [`PdmRxConfig`].
    ///
    /// For details, refer to the
    /// _ESP-IDF Programming Guide_ PDM Rx Usage details for your specific microcontroller:
    /// * [ESP32](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/peripherals/i2s.html#pdm-rx-usage)
    /// * [ESP32-S3](https://docs.espressif.com/projects/esp-idf/en/latest/esp32s3/api-reference/peripherals/i2s.html#pdm-rx-usage)
    ///
    /// Other microcontrollers do not support PDM receive mode, or do not have a PDM-to-PCM peripheral that allows for decoding
    /// the PDM data as required by ESP-IDF.
    #[derive(Clone, Copy, Debug)]
    pub struct PdmRxSlotConfig {
        /// I2S sample data bit width (valid data bits per sample).
        #[allow(dead_code)]
        pub(super) data_bit_width: DataBitWidth,

        /// I2s slot bit width (total bits per slot).
        #[allow(dead_code)]
        pub(super) slot_bit_width: SlotBitWidth,

        /// Mono or stereo mode operation.
        #[allow(dead_code)]
        pub(super) slot_mode: SlotMode,

        /// Are we using the left, right, or both data slots?
        #[allow(dead_code)]
        pub(super) slot_mask: PdmSlotMask,

        /// High pass filter
        #[cfg(esp_idf_soc_i2s_supports_pdm_rx_hp_filter)]
        pub(super) high_pass: Option<HighPassFilter>,
    }

    impl PartialEq for PdmRxSlotConfig {
        #[cfg(not(esp_idf_soc_i2s_supports_pdm_rx_hp_filter))]
        fn eq(&self, other: &Self) -> bool {
            self.data_bit_width == other.data_bit_width
                && self.slot_bit_width == other.slot_bit_width
                && self.slot_mode == other.slot_mode
                && self.slot_mask == other.slot_mask
        }

        /// Note: Ignoring the high_pass filter that contains a f32 for eq compairson and only checking if its set or not.
        #[cfg(esp_idf_soc_i2s_supports_pdm_rx_hp_filter)]
        fn eq(&self, other: &Self) -> bool {
            self.data_bit_width == other.data_bit_width
                && self.slot_bit_width == other.slot_bit_width
                && self.slot_mode == other.slot_mode
                && self.slot_mask == other.slot_mask
                && self.high_pass.is_some() == other.high_pass.is_some()
        }
    }

    impl Eq for PdmRxSlotConfig {}

    impl PdmRxSlotConfig {
        /// Configure the PDM mode channel receive slot configuration for the specified bits per sample and slot mode
        /// in 2 slots.
        pub fn from_bits_per_sample_and_slot_mode(
            bits_per_sample: DataBitWidth,
            slot_mode: SlotMode,
        ) -> Self {
            let slot_mask = if slot_mode == SlotMode::Mono {
                PdmSlotMask::Left
            } else {
                PdmSlotMask::Both
            };

            Self {
                data_bit_width: bits_per_sample,
                slot_bit_width: SlotBitWidth::Auto,
                slot_mode,
                slot_mask,
                #[cfg(esp_idf_soc_i2s_supports_pdm_rx_hp_filter)]
                high_pass: None,
            }
        }

        /// Update the data bit width on this PDM receive slot configuration.
        #[inline(always)]
        pub fn data_bit_width(mut self, data_bit_width: DataBitWidth) -> Self {
            self.data_bit_width = data_bit_width;
            self
        }

        /// Update the slot bit width on this PDM receive slot configuration.
        #[inline(always)]
        pub fn slot_bit_width(mut self, slot_bit_width: SlotBitWidth) -> Self {
            self.slot_bit_width = slot_bit_width;
            self
        }

        /// Update the slot mode and mask on this PDM receive slot configuration.
        #[inline(always)]
        pub fn slot_mode_mask(mut self, slot_mode: SlotMode, slot_mask: PdmSlotMask) -> Self {
            self.slot_mode = slot_mode;
            self.slot_mask = slot_mask;
            self
        }

        #[cfg(esp_idf_soc_i2s_supports_pdm_rx_hp_filter)]
        /// Set the PDM high pass filter
        pub fn high_pass_filter(mut self, filter: Option<HighPassFilter>) -> Self {
            self.high_pass = filter;
            self
        }

        /// Convert this PDM mode channel receive slot configuration into the ESP-IDF SDK `i2s_pdm_rx_slot_config_t`
        /// representation.
        #[cfg(esp_idf_soc_i2s_supports_pdm_rx)]
        #[inline(always)]
        #[allow(clippy::needless_update)]
        pub(super) fn as_sdk(&self) -> i2s_pdm_rx_slot_config_t {
            i2s_pdm_rx_slot_config_t {
                data_bit_width: self.data_bit_width.as_sdk(),
                slot_bit_width: self.slot_bit_width.as_sdk(),
                slot_mode: self.slot_mode.as_sdk(),
                slot_mask: self.slot_mask.as_sdk(),
                #[cfg(esp_idf_soc_i2s_supports_pdm_rx_hp_filter)]
                hp_en: self.high_pass.is_some(),
                #[cfg(esp_idf_soc_i2s_supports_pdm_rx_hp_filter)]
                hp_cut_off_freq_hz: if let Some(filter) = self.high_pass {
                    filter.cut_off_freq
                } else {
                    185.0
                },
                #[cfg(esp_idf_soc_i2s_supports_pdm_rx_hp_filter)]
                amplify_num: if let Some(filter) = self.high_pass {
                    filter.amplify_num
                } else {
                    1
                },
                ..Default::default()
            }
        }
    }

    /// Pulse density modulation (PDM) transmit signal scaling mode.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub enum PdmSignalScale {
        /// Divide the PDM signal by 2.
        Div2,

        /// No scaling.
        #[default]
        None,

        /// Multiply the PDM signal by 2.
        Mul2,

        /// Multiply the PDM signal by 4.
        Mul4,
    }

    impl PdmSignalScale {
        /// Convert to the ESP-IDF SDK `i2s_pdm_signal_scale_t` representation.
        #[cfg_attr(esp_idf_version_major = "4", allow(unused))]
        #[inline(always)]
        pub(crate) fn as_sdk(&self) -> i2s_pdm_sig_scale_t {
            match self {
                Self::Div2 => 0,
                Self::None => 1,
                Self::Mul2 => 2,
                Self::Mul4 => 3,
            }
        }
    }

    /// I2S slot selection in PDM mode.
    ///
    /// The default is `PdmSlotMask::Both`.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum PdmSlotMask {
        /// I2S transmits or receives the left slot.
        Left,

        /// I2S transmits or receives the right slot.
        Right,

        /// I2S transmits or receives both slots.
        Both,
    }

    impl Default for PdmSlotMask {
        #[inline(always)]
        fn default() -> Self {
            Self::Both
        }
    }

    impl PdmSlotMask {
        /// Convert to the ESP-IDF SDK `i2s_pdm_slot_mask_t` representation.
        #[cfg(not(esp_idf_version_major = "4"))]
        #[inline(always)]
        #[allow(unused)]
        pub(crate) fn as_sdk(&self) -> i2s_pdm_slot_mask_t {
            match self {
                Self::Left => 1 << 0,
                Self::Right => 1 << 1,
                Self::Both => (1 << 0) | (1 << 1),
            }
        }
    }

    /// PDM RX High Pass Filter
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct HighPassFilter {
        /// High pass filter cut-off frequency, range 23.3Hz ~ 185Hz
        pub(super) cut_off_freq: f32,

        /// The amplification number of the final conversion result
        ///
        /// The data that have converted from PDM to PCM module, will time `amplify_num` additionally to amplify the final result.
        /// Note that it's only a multiplier of the digital PCM data, not the gain of the analog signal.
        /// range 1~15, default 1
        pub(super) amplify_num: u32,
    }

    impl HighPassFilter {
        /// Set the Filter cut off Frequency.
        ///
        /// Note: Range between 23.3Hz ~ 185Hz
        pub fn cut_off_freq(cut_off_freq: f32) -> Self {
            Self {
                cut_off_freq,
                amplify_num: 1,
            }
        }

        /// Set the amplification number of the final conversion result
        ///
        /// Range: 1-15
        pub fn amplify_number(mut self, amplify_num: u32) -> Self {
            self.amplify_num = amplify_num;
            self
        }
    }

    /// The I2s pulse density modulation (PDM) mode transmit clock configuration.
    ///
    /// # Note
    /// The PDM transmit clock can only be set to the following two upsampling rate configurations:
    /// * `upsampling_fp = 960`, `upsampling_fs = sample_rate_hz / 100`. In this case, `Fpdm = 128 * 48000 = 6.144 MHz`.
    /// * `upsampling_fp = 960`, `upsampling_fs = 480`. In this case, `Fpdm = 128 * sample_rate_hz`.
    ///
    /// If the PDM receiver does not use the PDM serial clock, the first configuration should be used. Otherwise,
    /// use the second configuration.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct PdmTxClkConfig {
        /// I2S sample rate in Hz.
        pub(super) sample_rate_hz: u32,

        /// The clock source.
        pub(super) clk_src: ClockSource,

        /// The multiple of MCLK to the sample rate.
        pub(super) mclk_multiple: MclkMultiple,

        /// Upsampling `fp` parameter.
        upsample_fp: u32,

        /// Upsampling `fs` parameter.
        upsample_fs: u32,
    }

    impl PdmTxClkConfig {
        /// Create a new PDM mode transmit clock configuration from the specified sample rate in Hz. This will set the
        /// clock source to PLL_F160M, the MCLK multiple to 256 times the sample rate, `upsample_fp` to 960, and
        /// `upsample_fs` to 480.
        #[inline(always)]
        pub fn from_sample_rate_hz(sample_rate_hz: u32) -> Self {
            Self {
                sample_rate_hz,
                clk_src: ClockSource::default(),
                mclk_multiple: MclkMultiple::M256,
                upsample_fp: 960,
                upsample_fs: 480,
            }
        }

        /// Set the sample rate on this PDM mode transmit clock configuration.
        #[inline(always)]
        pub fn sample_rate_hz(mut self, sample_rate_hz: u32) -> Self {
            self.sample_rate_hz = sample_rate_hz;
            self
        }

        /// Set the clock source on this PDM mode transmit clock configuration.
        #[inline(always)]
        pub fn clk_src(mut self, clk_src: ClockSource) -> Self {
            self.clk_src = clk_src;
            self
        }

        /// Set the MCLK multiple on this PDM mode transmit clock configuration.
        #[inline(always)]
        pub fn mclk_multiple(mut self, mclk_multiple: MclkMultiple) -> Self {
            self.mclk_multiple = mclk_multiple;
            self
        }

        /// Set the upsampling parameters on this PDM mode transmit clock configuration.
        #[inline(always)]
        pub fn upsample(mut self, upsample_fp: u32, upsample_fs: u32) -> Self {
            self.upsample_fp = upsample_fp;
            self.upsample_fs = upsample_fs;
            self
        }

        /// Convert to the ESP-IDF SDK `i2s_pdm_tx_clk_config_t` representation.
        #[allow(clippy::needless_update)]
        #[cfg(not(esp_idf_version_major = "4"))]
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_pdm_tx_clk_config_t {
            i2s_pdm_tx_clk_config_t {
                sample_rate_hz: self.sample_rate_hz,
                clk_src: self.clk_src.as_sdk(),
                mclk_multiple: self.mclk_multiple.as_sdk(),
                up_sample_fp: self.upsample_fp,
                up_sample_fs: self.upsample_fs,
                ..Default::default() // bclk_div in ESP IDF > 5.1
            }
        }

        /// Convert to the ESP-IDF SDK `i2s_pdm_tx_upsample_cfg_t` representation.
        #[cfg(esp_idf_version_major = "4")]
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_pdm_tx_upsample_cfg_t {
            i2s_pdm_tx_upsample_cfg_t {
                sample_rate: self.sample_rate_hz as i32,
                fp: self.upsample_fp as i32,
                fs: self.upsample_fs as i32,
            }
        }
    }

    /// The I2S pulse density modulation (PDM) mode transmit configuration for the I2S peripheral.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct PdmTxConfig {
        /// The base channel configuration.
        pub(super) channel_cfg: Config,

        /// PDM mode channel transmit clock configuration.
        pub(super) clk_cfg: PdmTxClkConfig,

        /// PDM mode channel transmit slot configuration.
        pub(super) slot_cfg: PdmTxSlotConfig,

        /// PDM mode channel transmit GPIO configuration.
        #[cfg(not(esp_idf_version_major = "4"))]
        pub(super) gpio_cfg: PdmTxGpioConfig,
    }

    impl PdmTxConfig {
        /// Create a new PDM mode transmit configuration from the specified clock, slot, and GPIO configurations.
        pub fn new(
            channel_cfg: Config,
            clk_cfg: PdmTxClkConfig,
            slot_cfg: PdmTxSlotConfig,
            #[cfg(not(esp_idf_version_major = "4"))] gpio_cfg: PdmTxGpioConfig,
        ) -> Self {
            Self {
                channel_cfg,
                clk_cfg,
                slot_cfg,
                #[cfg(not(esp_idf_version_major = "4"))]
                gpio_cfg,
            }
        }

        /// Convert just the clock config to the SDK representation. Used by
        /// the runtime `reconfigure_pdm` paths that don't touch GPIO.
        #[cfg(esp_idf_soc_i2s_supports_pdm_tx)]
        #[inline(always)]
        pub(crate) fn clk_cfg_as_sdk(&self) -> i2s_pdm_tx_clk_config_t {
            self.clk_cfg.as_sdk()
        }

        /// Convert just the slot config to the SDK representation. Used by
        /// the runtime `reconfigure_pdm` paths that don't touch GPIO.
        #[cfg(esp_idf_soc_i2s_supports_pdm_tx)]
        #[inline(always)]
        pub(crate) fn slot_cfg_as_sdk(&self) -> i2s_pdm_tx_slot_config_t {
            self.slot_cfg.as_sdk()
        }

        /// Convert to the ESP-IDF `i2s_pdm_tx_config_t` representation.
        #[cfg(all(not(esp_idf_version_major = "4"), not(esp_idf_soc_i2s_hw_version_2)))]
        #[inline(always)]
        pub(crate) fn as_sdk<'d>(
            &self,
            clk: impl OutputPin + 'd,
            dout: impl OutputPin + 'd,
        ) -> i2s_pdm_tx_config_t {
            i2s_pdm_tx_config_t {
                clk_cfg: self.clk_cfg.as_sdk(),
                slot_cfg: self.slot_cfg.as_sdk(),
                gpio_cfg: self.gpio_cfg.as_sdk(clk, dout),
            }
        }

        /// Convert to the ESP-IDF `i2s_pdm_tx_config_t` representation.
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        #[inline(always)]
        pub(crate) fn as_sdk<'d>(
            &self,
            clk: impl OutputPin + 'd,
            dout: impl OutputPin + 'd,
            dout2: Option<impl OutputPin + 'd>,
        ) -> i2s_pdm_tx_config_t {
            i2s_pdm_tx_config_t {
                clk_cfg: self.clk_cfg.as_sdk(),
                slot_cfg: self.slot_cfg.as_sdk(),
                gpio_cfg: self.gpio_cfg.as_sdk(clk, dout, dout2),
            }
        }

        /// Convert to the ESP-IDF `i2s_driver_config_t` representation.
        #[cfg(esp_idf_version_major = "4")]
        pub(crate) fn as_sdk(&self) -> i2s_driver_config_t {
            let chan_fmt = match self.slot_cfg.slot_mode {
                SlotMode::Stereo => i2s_channel_fmt_t_I2S_CHANNEL_FMT_RIGHT_LEFT,
                SlotMode::Mono => match self.slot_cfg.slot_mask {
                    PdmSlotMask::Both => i2s_channel_fmt_t_I2S_CHANNEL_FMT_RIGHT_LEFT,
                    PdmSlotMask::Left => i2s_channel_fmt_t_I2S_CHANNEL_FMT_ONLY_LEFT,
                    PdmSlotMask::Right => i2s_channel_fmt_t_I2S_CHANNEL_FMT_ONLY_RIGHT,
                },
            };

            i2s_driver_config_t {
                mode: self.channel_cfg.role.as_sdk()
                    | i2s_mode_t_I2S_MODE_TX
                    | i2s_mode_t_I2S_MODE_PDM,
                sample_rate: self.clk_cfg.sample_rate_hz,
                bits_per_sample: 16, // fixed for PDM,
                channel_format: chan_fmt,
                communication_format: 0,  // ?
                intr_alloc_flags: 1 << 1, // ESP_INTR_FLAG_LEVEL1
                dma_buf_count: self.channel_cfg.dma_buffer_count as i32,
                dma_buf_len: self.channel_cfg.frames_per_buffer as i32,
                #[cfg(any(esp32, esp32s2))]
                use_apll: matches!(self.clk_cfg.clk_src, ClockSource::Apll),
                #[cfg(not(any(esp32, esp32s2)))]
                use_apll: false,
                tx_desc_auto_clear: self.channel_cfg.auto_clear,
                fixed_mclk: 0,
                mclk_multiple: self.clk_cfg.mclk_multiple.as_sdk(),
                bits_per_chan: 16, // fixed for PDM

                // The following are TDM-only fields and are not present on chips that don't support TDM mode.
                // There's no cfg option for this (it's a constant in esp-idf-sys).
                #[cfg(not(any(esp32, esp32s2)))]
                chan_mask: 0,
                #[cfg(not(any(esp32, esp32s2)))]
                total_chan: 0,
                #[cfg(not(any(esp32, esp32s2)))]
                left_align: false,
                #[cfg(not(any(esp32, esp32s2)))]
                big_edin: false,
                #[cfg(not(any(esp32, esp32s2)))]
                bit_order_msb: false,
                #[cfg(not(any(esp32, esp32s2)))]
                skip_msk: true,
            }
        }
    }

    /// PDM mode GPIO (general purpose input/output) transmit configuration.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct PdmTxGpioConfig {
        /// Whether the clock output is inverted.
        pub(super) clk_inv: bool,
    }

    /// The maximum number of data output pins that can be used in PDM mode.
    ///
    /// This is 1 on the ESP32, and 2 on the ESP32-S3, ESP32-C3, ESP32-C6, and ESP32-H2.
    #[cfg(esp32)]
    pub const SOC_I2S_PDM_MAX_TX_LINES: usize = 1;

    /// The maximum number of data input pins that can be used in PDM mode.
    ///
    /// This is 1 on the ESP32, and 2 on the ESP32-S3, ESP32-C3, ESP32-C6, and ESP32-H2.
    #[cfg(any(esp32s3, esp32c3, esp32c6, esp32h2, esp32h21))]
    pub const SOC_I2S_PDM_MAX_TX_LINES: usize = 2;

    impl PdmTxGpioConfig {
        /// Create a new PDM mode GPIO transmit configuration with the specified inversion flag for the clock output.
        #[inline(always)]
        pub fn new(clk_inv: bool) -> Self {
            Self { clk_inv }
        }

        /// Set the clock inversion flag on this PDM GPIO transmit configuration.
        #[inline(always)]
        pub fn clk_invert(mut self, clk_inv: bool) -> Self {
            self.clk_inv = clk_inv;
            self
        }

        /// Convert to the ESP-IDF SDK `i2s_pdm_tx_gpio_config_t` representation.
        #[cfg(esp_idf_soc_i2s_hw_version_1)]
        pub(crate) fn as_sdk<'d>(
            &self,
            clk: impl OutputPin + 'd,
            dout: impl OutputPin + 'd,
        ) -> i2s_pdm_tx_gpio_config_t {
            let invert_flags = i2s_pdm_tx_gpio_config_t__bindgen_ty_1 {
                _bitfield_1: i2s_pdm_tx_gpio_config_t__bindgen_ty_1::new_bitfield_1(
                    self.clk_inv as u32,
                ),
                ..Default::default()
            };
            i2s_pdm_tx_gpio_config_t {
                clk: clk.pin() as _,
                dout: dout.pin() as _,
                invert_flags,
            }
        }

        /// Convert to the ESP-IDF SDK `i2s_pdm_tx_gpio_config_t` representation.
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        pub(crate) fn as_sdk<'d>(
            &self,
            clk: impl OutputPin + 'd,
            dout: impl OutputPin + 'd,
            dout2: Option<impl OutputPin + 'd>,
        ) -> i2s_pdm_tx_gpio_config_t {
            let invert_flags = i2s_pdm_tx_gpio_config_t__bindgen_ty_1 {
                _bitfield_1: i2s_pdm_tx_gpio_config_t__bindgen_ty_1::new_bitfield_1(
                    self.clk_inv as u32,
                ),
                ..Default::default()
            };
            let dout2 = if let Some(dout2) = dout2 {
                dout2.pin() as _
            } else {
                -1
            };

            i2s_pdm_tx_gpio_config_t {
                clk: clk.pin() as _,
                dout: dout.pin() as _,
                dout2,
                invert_flags,
            }
        }
    }

    /// I2S pulse density modulation (PDM) transmit line mode
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum PdmTxLineMode {
        /// Standard PDM format output: left and right slot data on a single line.
        #[default]
        OneLineCodec,

        /// PDM DAC format output: left or right slot data on a single line.
        OneLineDac,

        /// PDM DAC format output: left and right slot data on separate lines.
        TwoLineDac,
    }

    impl PdmTxLineMode {
        /// Convert this to the ESP-IDF SDK `i2s_pdm_tx_line_mode_t` representation.
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        #[inline(always)]
        pub(super) fn as_sdk(&self) -> i2s_pdm_tx_line_mode_t {
            match self {
                Self::OneLineCodec => i2s_pdm_tx_line_mode_t_I2S_PDM_TX_ONE_LINE_CODEC,
                Self::OneLineDac => i2s_pdm_tx_line_mode_t_I2S_PDM_TX_ONE_LINE_DAC,
                Self::TwoLineDac => i2s_pdm_tx_line_mode_t_I2S_PDM_TX_TWO_LINE_DAC,
            }
        }
    }

    /// PDM mode channel transmit slot configuration.
    ///
    /// # Note
    /// The `slot_mode` and `line_mode` (microcontrollers new than ESP32) or `slot_mask` (ESP32) cause data to be
    /// interpreted in different ways, as noted below.
    ///
    /// Assuming the buffered data contains the following samples (where a sample may be 1, 2, 3, or 4 bytes, depending
    /// on `data_bit_width`):
    ///
    /// | **`d[0]`** | **`d[1]`** | **`d[2]`** | **`d[3]`** | **`d[4]`** | **`d[5]`** | **`d[6]`** | **`d[7]`** |
    /// |------------|------------|------------|------------|------------|------------|------------|------------|
    /// |  11        | 12         | 13         | 14         | 15         | 16         | 17         | 18         |
    ///
    /// The actual data on the line will be:
    ///
    /// ## All microcontrollers except ESP32
    /// <table>
    ///   <thead>
    ///     <tr><th><code>line_mode</code></th><th><code>slot_mode</code></th><th>Line</th><th colspan=8>Transmitted Data</th></tr>
    ///     <tr><th></th><th></th><th></th><th>WS Low</th><th>WS High</th><th>WS Low</th><th>WS High</th><th>WS Low</th><th>WS High</th><th>WS Low</th><th>WS High</th></tr>
    ///   </thead>
    ///   <tbody>
    ///     <tr><td rowspan=2><code>OneLineCodec</code></td><td><code>Mono</code></td>  <td>dout</td><td>11</td><td><font color="red">0</font></td><td>12</td><td><font color="red">0</font></td><td>13</td><td><font color="red">0</font></td><td>14</td><td><font color="red">0</font></td></tr>
    ///     <tr>                                            <td><code>Stereo</code></td><td>dout</td><td>11</td><td>12</td><td>13</td><td>14</td><td>15</td><td>16</td><td>17</td><td>18</td></tr>
    ///     <tr><td><code>OneLineDac</code></td>            <td><code>Mono</code></td>  <td>dout</td><td>11</td><td>11</td><td>12</td><td>12</td><td>13</td><td>13</td><td>14</td><td>14</td></tr>
    ///     <tr><td rowspan=4><code>TwoLineDac</code></td>        <td rowspan=2><code>Mono</code></td><td>dout</td><td>12</td><td>12</td><td>14</td><td>14</td><td>16</td><td>16</td><td>18</td><td>18</td></tr>
    ///     <tr><td>dout2</td><td><font color="red">0</font></td><td><font color="red">0</font></td><td><font color="red">0</font></td><td><font color="red">0</font></td><td><font color="red">0</font></td><td><font color="red">0</font></td><td><font color="red">0</font></td><td><font color="red">0</font></td></tr>
    ///     <tr><td rowspan=2><code>Stereo</code></td><td>dout</td><td>12</td><td>12</td><td>14</td><td>14</td><td>16</td><td>16</td><td>18</td><td>18</td></tr>
    ///     <tr><td>dout2</td><td>11</td><td>11</td><td>13</td><td>13</td><td>15</td><td>15</td><td>17</td><td>17</td></tr>
    ///  </tbody>
    /// </table>
    ///
    /// ## ESP32
    /// <table>
    ///   <thead>
    ///     <tr><th><code>slot_mode</code></th><th><code>slot_mask</code></th><th colspan=8>Transmitted Data</th></tr>
    ///     <tr><th></th><th></th><th>WS Low</th><th>WS High</th><th>WS Low</th><th>WS High</th><th>WS Low</th><th>WS High</th><th>WS Low</th><th>WS High</th></tr>
    ///   </thead>
    ///   <tbody>
    ///     <tr><td rowspan=3><code>Mono</code></td>  <td><code>Left</code></td> <td>11</td><td><font color="red">0</font></td><td>12</td><td><font color="red">0</font></td><td>13</td><td><font color="red">0</font></td><td>14</td><td><font color="red">0</font></td></tr>
    ///     <tr>                                      <td><code>Right</code></td><td><font color="red">0</font></td><td>11</td><td><font color="red">0</font></td><td>12</td><td><font color="red">0</font></td><td>13</td><td><font color="red">0</font></td><td>14</td></tr>
    ///     <tr>                                      <td><code>Both</code></td><td>11</td><td>12</td><td>13</td><td>14</td><td>15</td><td>16</td><td>17</td><td>18</td></tr>
    ///     <tr><td rowspan=3><code>Mono</code></td>  <td><code>Left</code></td><td>11</td><td>11</td><td>13</td><td>13</td><td>15</td><td>15</td><td>17</td><td>17</td></tr>
    ///     <tr>                                      <td><code>Right</code></td><td>12</td><td>12</td><td>14</td><td>14</td><td>16</td><td>16</td><td>18</td><td>18</td></tr>
    ///     <tr>                                      <td><code>Both</code></td> <td>11</td><td>12</td><td>13</td><td>14</td><td>15</td><td>16</td><td>17</td><td>18</td></tr>
    ///  </tbody>
    /// </table>
    ///
    /// Modes combinations other than [`SlotMode::Mono`]/[`PdmSlotMask::Both`],
    /// [`SlotMode::Stereo`]/[`PdmSlotMask::Left`], and [`SlotMode::Stereo`]/[`PdmSlotMask::Right`] are unlikely to be
    /// useful since it requires precise demutiplexing on the bit stream based on the WS clock.
    ///
    /// For details, refer to the
    /// _ESP-IDF Programming Guide_ PDM Tx Usage details for your specific microcontroller:
    /// * [ESP32](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/peripherals/i2s.html#pdm-tx-usage)
    /// * [ESP32-S3](https://docs.espressif.com/projects/esp-idf/en/latest/esp32s3/api-reference/peripherals/i2s.html#pdm-tx-usage)
    /// * [ESP32-C3](https://docs.espressif.com/projects/esp-idf/en/latest/esp32c3/api-reference/peripherals/i2s.html#pdm-tx-usage)
    /// * [ESP32-C6](https://docs.espressif.com/projects/esp-idf/en/latest/esp32c6/api-reference/peripherals/i2s.html#pdm-tx-usage)
    /// * [ESP32-H2](https://docs.espressif.com/projects/esp-idf/en/latest/esp32c6/api-reference/peripherals/i2s.html#pdm-tx-usage)
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct PdmTxSlotConfig {
        // data_bit_width and slot_bit_width are omitted; they are always 16 bits.
        /// Mono or stereo mode operation.
        pub(super) slot_mode: SlotMode,

        /// Slot mask to choose the left or right slot.
        #[cfg(not(esp_idf_soc_i2s_hw_version_2))]
        pub(super) slot_mask: PdmSlotMask,

        /// Sigma-delta filter prescale.
        sd_prescale: u32,

        /// Sigma-delta filter saling value.
        sd_scale: PdmSignalScale,

        /// High-pass filter scaling value.
        hp_scale: PdmSignalScale,

        /// Low-pass filter scaling value
        lp_scale: PdmSignalScale,

        /// Sinc-filter scaling value.
        sinc_scale: PdmSignalScale,

        /// PDM transmit line mode.
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        line_mode: PdmTxLineMode,

        /// High-pass filter enable
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        hp_enable: bool,

        /// High-pass filter cutoff frequence.
        /// The range of this is 23.3Hz to 185Hz.
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        hp_cutoff_freq: f32,

        /// Sigma-delta filter dither.
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        sd_dither: u32,

        /// Sigma-delta filter dither 2.
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        sd_dither2: u32,
    }

    impl Default for PdmTxSlotConfig {
        #[inline(always)]
        fn default() -> Self {
            Self::from_slot_mode(SlotMode::Stereo)
        }
    }

    // We don't care about NaN in hp_cutoff_freq; go ahead and force it to be Eq.
    impl Eq for PdmTxSlotConfig {}

    impl PdmTxSlotConfig {
        /// Configure the PDM mode channel transmit slot configuration for the specified slot mode in 2 slots.
        ///
        /// This sets the sigma-delta, low-pass, and sinc scaling to None.
        ///
        /// For hardware version 1, the high-pass filter scaling is set to None.
        ///
        /// For hardware version 2, the high-pass filter is enabled, scaled to dividing by 2 and set to 35.5 Hz.
        #[inline(always)]
        pub fn from_slot_mode(slot_mode: SlotMode) -> Self {
            Self {
                slot_mode,
                #[cfg(not(esp_idf_soc_i2s_hw_version_2))]
                slot_mask: PdmSlotMask::Both,
                sd_prescale: 0,
                sd_scale: PdmSignalScale::None,
                #[cfg(not(esp_idf_soc_i2s_hw_version_2))]
                hp_scale: PdmSignalScale::None,
                #[cfg(esp_idf_soc_i2s_hw_version_2)]
                hp_scale: PdmSignalScale::Div2,
                lp_scale: PdmSignalScale::None,
                sinc_scale: PdmSignalScale::None,
                #[cfg(esp_idf_soc_i2s_hw_version_2)]
                line_mode: PdmTxLineMode::OneLineCodec,
                #[cfg(esp_idf_soc_i2s_hw_version_2)]
                hp_enable: true,
                #[cfg(esp_idf_soc_i2s_hw_version_2)]
                hp_cutoff_freq: 32.5,
                #[cfg(esp_idf_soc_i2s_hw_version_2)]
                sd_dither: 0,
                #[cfg(esp_idf_soc_i2s_hw_version_2)]
                sd_dither2: 1,
            }
        }

        /// Sets the slot mode on this PDM transmit slot configuration.
        #[inline(always)]
        pub fn slot_mode(mut self, slot_mode: SlotMode) -> Self {
            self.slot_mode = slot_mode;
            self
        }

        /// Sets the slot mask on this PDM transmit slot configuration.
        #[cfg(esp_idf_soc_i2s_hw_version_1)]
        #[cfg_attr(
            feature = "nightly",
            doc(cfg(all(esp32, not(esp_idf_version_major = "4"))))
        )]
        #[inline(always)]
        pub fn slot_mask(mut self, slot_mask: PdmSlotMask) -> Self {
            self.slot_mask = slot_mask;
            self
        }

        /// Sets the sigma-delta filter prescale on this PDM transmit slot configuration.
        #[inline(always)]
        pub fn sd_prescale(mut self, sd_prescale: u32) -> Self {
            self.sd_prescale = sd_prescale;
            self
        }

        /// Sets the sigma-delta filter scaling on this PDM transmit slot configuration.
        #[inline(always)]
        pub fn sd_scale(mut self, sd_scale: PdmSignalScale) -> Self {
            self.sd_scale = sd_scale;
            self
        }

        /// Sets the high-pass filter scaling on this PDM transmit slot configuration.
        #[inline(always)]
        pub fn hp_scale(mut self, hp_scale: PdmSignalScale) -> Self {
            self.hp_scale = hp_scale;
            self
        }

        /// Sets the low-pass filter scaling on this PDM transmit slot configuration.
        #[inline(always)]
        pub fn lp_scale(mut self, lp_scale: PdmSignalScale) -> Self {
            self.lp_scale = lp_scale;
            self
        }

        /// Sets the sinc filter scaling on this PDM transmit slot configuration.
        #[inline(always)]
        pub fn sinc_scale(mut self, sinc_scale: PdmSignalScale) -> Self {
            self.sinc_scale = sinc_scale;
            self
        }

        /// Sets the PDM transmit line mode on this PDM transmit slot configuration.
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        #[cfg_attr(
            feature = "nightly",
            doc(cfg(all(
                any(esp32s3, esp32c3, esp32c6, esp32h2, esp32h21),
                not(esp_idf_version_major = "4")
            )))
        )]
        #[inline(always)]
        pub fn line_mode(mut self, line_mode: PdmTxLineMode) -> Self {
            self.line_mode = line_mode;
            self
        }

        /// Sets the high-pass filter enable on this PDM transmit slot configuration.
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        #[cfg_attr(
            feature = "nightly",
            doc(cfg(all(
                any(esp32s3, esp32c3, esp32c6, esp32h2, esp32h21),
                not(esp_idf_version_major = "4")
            )))
        )]
        #[inline(always)]
        pub fn hp_enable(mut self, hp_enable: bool) -> Self {
            self.hp_enable = hp_enable;
            self
        }

        /// Sets the high-pass filter cutoff frequency on this PDM transmit slot configuration.
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        #[cfg_attr(
            feature = "nightly",
            doc(cfg(all(
                any(esp32s3, esp32c3, esp32c6, esp32h2, esp32h21),
                not(esp_idf_version_major = "4")
            )))
        )]
        #[inline(always)]
        pub fn hp_cutoff_freq(mut self, hp_cutoff_freq: f32) -> Self {
            self.hp_cutoff_freq = hp_cutoff_freq;
            self
        }

        /// Sets the sigma-delta filter dither on this PDM transmit slot configuration.
        #[cfg(esp_idf_soc_i2s_hw_version_2)]
        #[cfg_attr(
            feature = "nightly",
            doc(cfg(all(
                any(esp32s3, esp32c3, esp32c6, esp32h2, esp32h21),
                not(esp_idf_version_major = "4")
            )))
        )]
        #[inline(always)]
        pub fn sd_dither(mut self, sd_dither: u32, sd_dither2: u32) -> Self {
            self.sd_dither = sd_dither;
            self.sd_dither2 = sd_dither2;
            self
        }

        /// Convert this to the ESP-IDF SDK `i2s_pdm_tx_slot_config_t` type.
        #[cfg(not(esp_idf_version_major = "4"))]
        #[inline(always)]
        #[allow(clippy::needless_update)]
        pub(super) fn as_sdk(&self) -> i2s_pdm_tx_slot_config_t {
            i2s_pdm_tx_slot_config_t {
                data_bit_width: DataBitWidth::Bits16.as_sdk(),
                slot_bit_width: SlotBitWidth::Bits16.as_sdk(),
                slot_mode: self.slot_mode.as_sdk(),
                #[cfg(esp_idf_soc_i2s_hw_version_1)]
                slot_mask: self.slot_mask.as_sdk(),
                sd_prescale: self.sd_prescale,
                sd_scale: self.sd_scale.as_sdk(),
                hp_scale: self.hp_scale.as_sdk(),
                lp_scale: self.lp_scale.as_sdk(),
                sinc_scale: self.sinc_scale.as_sdk(),
                #[cfg(esp_idf_soc_i2s_hw_version_2)]
                line_mode: self.line_mode.as_sdk(),
                #[cfg(esp_idf_soc_i2s_hw_version_2)]
                hp_en: self.hp_enable,
                #[cfg(esp_idf_soc_i2s_hw_version_2)]
                hp_cut_off_freq_hz: self.hp_cutoff_freq,
                #[cfg(esp_idf_soc_i2s_hw_version_2)]
                sd_dither: self.sd_dither,
                #[cfg(esp_idf_soc_i2s_hw_version_2)]
                sd_dither2: self.sd_dither2,
                // i2s_pdm_data_fmt_t::I2S_PDM_DATA_FMT_PCM
                ..Default::default()
            }
        }
    }
}

#[cfg(esp_idf_soc_i2s_supports_pdm_rx)]
#[cfg_attr(
    feature = "nightly",
    doc(cfg(all(any(esp32, esp32s3), not(esp_idf_version_major = "4"))))
)]
impl<'d> I2sDriver<'d, I2sRx> {
    /// Create a new pulse density modulation (PDM) mode driver for the given I2S peripheral with only the receive
    /// channel open.
    #[allow(clippy::too_many_arguments)]
    pub fn new_pdm_rx<I2S: I2s + 'd>(
        _i2s: I2S,
        rx_cfg: &config::PdmRxConfig,
        clk: impl OutputPin + 'd,
        din: impl InputPin + 'd,
    ) -> Result<Self, EspError> {
        let chan_cfg = rx_cfg.channel_cfg.as_sdk(I2S::port());

        let this = Self::internal_new::<I2S>(&chan_cfg, true, false)?;

        let rx_cfg = rx_cfg.as_sdk(clk, din);

        // Safety: rx.chan_handle is a valid, non-null i2s_chan_handle_t,
        // and &rx_cfg is a valid pointer to an i2s_pdm_rx_config_t.
        unsafe {
            // Open the RX channel.
            esp!(esp_idf_sys::i2s_channel_init_pdm_rx_mode(
                this.rx_handle,
                &rx_cfg
            ))?;
        }

        Ok(this)
    }

    /// Create a new pulse density modulation (PDM) mode driver for the given I2S peripheral with only the receive
    /// channel open using multiple DIN pins to receive data.
    #[cfg(not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0")))]
    #[cfg_attr(
        feature = "nightly",
        doc(cfg(not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0"))))
    )]
    #[allow(clippy::too_many_arguments)]
    pub fn new_pdm_rx_multi<I2S, DIN, const DINC: usize>(
        _i2s: I2S,
        rx_cfg: &config::PdmRxConfig,
        clk: impl OutputPin + 'd,
        dins: [DIN; DINC],
    ) -> Result<Self, EspError>
    where
        I2S: I2s + 'd,
        DIN: InputPin + 'd,
    {
        let chan_cfg = rx_cfg.channel_cfg.as_sdk(I2S::port());

        let this = Self::internal_new::<I2S>(&chan_cfg, true, true)?;

        // Create the channel configuration.
        let rx_cfg = rx_cfg.as_sdk_multi(clk, &dins);

        // Safety: rx.chan_handle is a valid, non-null i2s_chan_handle_t,
        // and &rx_cfg is a valid pointer to an i2s_pdm_rx_config_t.
        unsafe {
            // Open the RX channel.
            esp!(esp_idf_sys::i2s_channel_init_pdm_rx_mode(
                this.rx_handle,
                &rx_cfg
            ))?;
        }

        Ok(this)
    }
}

#[cfg(all(any(esp32, esp32s3), esp_idf_version_major = "4"))]
#[cfg_attr(
    feature = "nightly",
    doc(cfg(all(any(esp32, esp32s3), esp_idf_version_major = "4")))
)]
impl<'d> I2sDriver<'d, I2sRx> {
    /// Create a new pulse density modulation (PDM) mode driver for the given I2S peripheral with only the receive
    /// channel open.
    #[allow(clippy::too_many_arguments)]
    pub fn new_pdm_rx<I2S: I2s + 'd>(
        _i2s: I2S,
        rx_cfg: &config::PdmRxConfig,
        clk: impl OutputPin + 'd,
        din: impl InputPin + 'd,
    ) -> Result<Self, EspError> {
        let driver_cfg = rx_cfg.as_sdk();

        let this = Self::internal_new::<I2S>(&driver_cfg)?;

        // Set the rate and downsampling configuration.
        let downsample = rx_cfg.clk_cfg.downsample_mode.as_sdk();
        unsafe {
            esp!(i2s_set_pdm_rx_down_sample(I2S::port(), downsample))?;
        }

        // Set the pin configuration.
        let pin_cfg = i2s_pin_config_t {
            bck_io_num: clk.pin() as _,
            data_in_num: din.pin() as _,
            data_out_num: -1,
            mck_io_num: -1,
            ws_io_num: -1,
        };

        // Safety: &pin_cfg is a valid pointer to an i2s_pin_config_t.
        unsafe {
            esp!(i2s_set_pin(I2S::port(), &pin_cfg))?;
        }

        Ok(this)
    }
}

#[cfg(esp_idf_soc_i2s_supports_pdm_tx)]
#[cfg_attr(
    feature = "nightly",
    doc(cfg(all(
        any(esp32, esp32s3, esp32c3, esp32c6, esp32h2, esp32h21),
        not(esp_idf_version_major = "4")
    )))
)]
impl<'d> I2sDriver<'d, I2sTx> {
    /// Create a new pulse density modulation (PDM) mode driver for the given I2S peripheral with only the transmit
    /// channel open.
    #[allow(clippy::too_many_arguments)]
    pub fn new_pdm_tx<I2S: I2s + 'd>(
        _i2s: I2S,
        tx_cfg: &config::PdmTxConfig,
        clk: impl OutputPin + 'd,
        dout: impl OutputPin + 'd,
        #[cfg(esp_idf_soc_i2s_hw_version_2)] dout2: Option<impl OutputPin + 'd>,
    ) -> Result<Self, EspError> {
        let chan_cfg = tx_cfg.channel_cfg.as_sdk(I2S::port());

        let this = Self::internal_new::<I2S>(&chan_cfg, false, true)?;

        // Create the channel configuration.
        let tx_cfg = tx_cfg.as_sdk(
            clk,
            dout,
            #[cfg(esp_idf_soc_i2s_hw_version_2)]
            dout2,
        );

        // Safety: tx.chan_handle is a valid, non-null i2s_chan_handle_t,
        // and &tx_cfg is a valid pointer to an i2s_pdm_tx_config_t.
        unsafe {
            // Open the TX channel.
            esp!(esp_idf_sys::i2s_channel_init_pdm_tx_mode(
                this.tx_handle,
                &tx_cfg
            ))?;
        }

        Ok(this)
    }
}

#[cfg(all(
    esp_idf_version_major = "4",
    any(esp32, esp32s3, esp32c3, esp32c6, esp32h2, esp32h21)
))]
#[cfg_attr(
    feature = "nightly",
    doc(cfg(all(
        any(esp32, esp32s3, esp32c3, esp32c6, esp32h2, esp32h21),
        esp_idf_version_major = "4"
    )))
)]
impl<'d> I2sDriver<'d, I2sTx> {
    /// Create a new pulse density modulation (PDM) mode driver for the given I2S peripheral with only the transmit
    /// channel open.
    #[allow(clippy::too_many_arguments)]
    pub fn new_pdm_tx<I2S: I2s + 'd>(
        _i2s: I2S,
        tx_cfg: &config::PdmTxConfig,
        clk: impl OutputPin + 'd,
        dout: impl OutputPin + 'd,
    ) -> Result<Self, EspError> {
        let driver_cfg = tx_cfg.as_sdk();

        let this = Self::internal_new::<I2S>(&driver_cfg)?;

        // Set the upsampling configuration.
        let upsample = tx_cfg.clk_cfg.as_sdk();
        unsafe {
            esp!(esp_idf_sys::i2s_set_pdm_tx_up_sample(
                I2S::port(),
                &upsample
            ))?;
        }

        // Set the pin configuration.
        let pin_cfg = i2s_pin_config_t {
            bck_io_num: clk.pin() as _,
            data_in_num: -1,
            data_out_num: dout.pin() as _,
            mck_io_num: -1,
            ws_io_num: -1,
        };

        // Safety: &pin_cfg is a valid pointer to an i2s_pin_config_t.
        unsafe {
            esp!(i2s_set_pin(I2S::port(), &pin_cfg))?;
        }

        Ok(this)
    }
}

/// PDM-mode runtime reconfiguration.
///
/// Reconfigure the clock + slot config of an already-initialised PDM mode
/// channel without tearing the driver down, the PDM counterpart of
/// [`I2sDriver::rx_reconfigure_std`].
///
/// The channel is briefly disabled while the reconfigure happens and
/// re-enabled on success. GPIO pins are not touched.
#[cfg(esp_idf_soc_i2s_supports_pdm_rx)]
impl<Dir> I2sDriver<'_, Dir>
where
    Dir: I2sRxSupported,
{
    /// Reconfigure the RX channel's clock + slot from a new [`config::PdmRxConfig`].
    ///
    /// Fails if the channel is not currently enabled.
    pub fn rx_reconfigure_pdm(&mut self, config: &config::PdmRxConfig) -> Result<(), EspError> {
        let clk_cfg = config.clk_cfg_as_sdk();
        let slot_cfg = config.slot_cfg_as_sdk();
        unsafe {
            esp!(esp_idf_sys::i2s_channel_disable(self.rx_handle))?;
            esp!(esp_idf_sys::i2s_channel_reconfig_pdm_rx_clock(
                self.rx_handle,
                &clk_cfg
            ))?;
            esp!(esp_idf_sys::i2s_channel_reconfig_pdm_rx_slot(
                self.rx_handle,
                &slot_cfg
            ))?;
            esp!(esp_idf_sys::i2s_channel_enable(self.rx_handle))?;
        }
        Ok(())
    }
}

/// PDM-mode runtime reconfiguration.
///
/// Reconfigure the clock + slot config of an already-initialised PDM mode
/// channel without tearing the driver down, the PDM counterpart of
/// [`I2sDriver::tx_reconfigure_std`].
///
/// The channel is briefly disabled while the reconfigure happens and
/// re-enabled on success. GPIO pins are not touched.
#[cfg(esp_idf_soc_i2s_supports_pdm_tx)]
impl<Dir> I2sDriver<'_, Dir>
where
    Dir: I2sTxSupported,
{
    /// Reconfigure the TX channel's clock + slot from a new [`config::PdmTxConfig`].
    ///
    /// Fails if the channel is not currently enabled.
    pub fn tx_reconfigure_pdm(&mut self, config: &config::PdmTxConfig) -> Result<(), EspError> {
        let clk_cfg = config.clk_cfg_as_sdk();
        let slot_cfg = config.slot_cfg_as_sdk();
        unsafe {
            esp!(esp_idf_sys::i2s_channel_disable(self.tx_handle))?;
            esp!(esp_idf_sys::i2s_channel_reconfig_pdm_tx_clock(
                self.tx_handle,
                &clk_cfg
            ))?;
            esp!(esp_idf_sys::i2s_channel_reconfig_pdm_tx_slot(
                self.tx_handle,
                &slot_cfg
            ))?;
            esp!(esp_idf_sys::i2s_channel_enable(self.tx_handle))?;
        }
        Ok(())
    }
}
