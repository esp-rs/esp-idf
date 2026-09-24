#[cfg(feature = "alloc")]
pub use sdcard::*;

#[cfg(esp_idf_soc_sdmmc_host_supported)]
pub mod mmc;
pub mod spi;

#[cfg(feature = "alloc")]
mod sdcard {
    #[cfg(esp_idf_soc_sdmmc_host_supported)]
    use super::mmc::SdMmcHostDriver;
    use super::spi::SdSpiHostDriver;

    use core::borrow::Borrow;

    use crate::spi::SpiDriver;
    use crate::sys::*;

    extern crate alloc;

    const _SDMMC_HOST_FLAG_SPI: u32 = 1 << 3;
    const _SDMMC_HOST_FLAG_DDR: u32 = 1 << 4;
    const _SDMMC_HOST_FLAG_DEINIT_ARG: u32 = 1 << 5;

    pub type SdCardConfiguration = config::Configuration;

    pub mod config {
        #[cfg(not(any(
            esp_idf_version_major = "4",
            all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
            all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
        )))] // For ESP-IDF v5.2 and later
        use crate::sys::*;

        /// (SD-MMC only): Input delay phase
        #[cfg(not(any(
            esp_idf_version_major = "4",
            all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
            all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
        )))] // For ESP-IDF v5.2 and later
        #[non_exhaustive]
        #[derive(Debug, Copy, Clone, Eq, PartialEq)]
        pub enum DelayPhase {
            Phase0 = sdmmc_delay_phase_t_SDMMC_DELAY_PHASE_0 as isize,
            Phase1 = sdmmc_delay_phase_t_SDMMC_DELAY_PHASE_1 as isize,
            Phase2 = sdmmc_delay_phase_t_SDMMC_DELAY_PHASE_2 as isize,
            Phase3 = sdmmc_delay_phase_t_SDMMC_DELAY_PHASE_3 as isize,
        }

        // #[cfg(not(any(
        //     esp_idf_version_major = "4",
        //     all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
        //     all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
        // )))] // For ESP-IDF v5.2 and later
        // impl From<sdmmc_delay_phase_t> for DelayPhase {
        //     fn from(phase: sdmmc_delay_phase_t) -> Self {
        //         #[allow(non_upper_case_globals)]
        //         match phase {
        //             sdmmc_delay_phase_t_SDMMC_DELAY_PHASE_0 => Self::Phase0,
        //             sdmmc_delay_phase_t_SDMMC_DELAY_PHASE_1 => Self::Phase1,
        //             sdmmc_delay_phase_t_SDMMC_DELAY_PHASE_2 => Self::Phase2,
        //             sdmmc_delay_phase_t_SDMMC_DELAY_PHASE_3 => Self::Phase3,
        //             _ => panic!("Invalid delay phase"),
        //         }
        //     }
        // }

        /// SD-Card voltage
        #[non_exhaustive]
        #[derive(Default, Debug, Copy, Clone, Eq, PartialEq)]
        pub enum Voltage {
            /// (ESP32P4 only)
            /// Use 1.8V IO voltage for UHS-I speed
            /// This means the user has to provide an external LDO power supply
            /// or to enable and configure an internal LDO via the `sdkconfig` setting
            /// "SD/MMC Example Configuration -> SD power supply comes from internal LDO IO"
            V1P8,
            /// Use 3.3V IO voltage for speeds below UHS-I
            /// The only supported conf for MCUs other than ESP32P4
            #[default]
            V3P3,
        }

        impl Voltage {
            pub(crate) const fn as_native(&self) -> f32 {
                match self {
                    Self::V1P8 => 1.8,
                    Self::V3P3 => 3.3,
                }
            }
        }

        /// (SD-MMC only): Driver Strength
        #[cfg(not(any(
            esp_idf_version_major = "4",
            all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
            all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
            all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
            all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
        )))] // For ESP-IDF v5.4 and later
        #[non_exhaustive]
        #[derive(Debug, Copy, Clone, Eq, PartialEq)]
        pub enum DriverStrength {
            /// Type B
            StrengthB = sdmmc_driver_strength_t_SDMMC_DRIVER_STRENGTH_B as isize,
            /// Type A
            StrengthA = sdmmc_driver_strength_t_SDMMC_DRIVER_STRENGTH_A as isize,
            /// Type C
            StrengthC = sdmmc_driver_strength_t_SDMMC_DRIVER_STRENGTH_C as isize,
            /// Type D
            StrengthD = sdmmc_driver_strength_t_SDMMC_DRIVER_STRENGTH_D as isize,
        }

        /// (SD-MMC only): Driver Strength
        #[cfg(not(any(
            esp_idf_version_major = "4",
            all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
            all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
            all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
            all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
        )))] // For ESP-IDF v5.4 and later
        #[non_exhaustive]
        #[derive(Debug, Copy, Clone, Eq, PartialEq)]
        pub enum CurrentLimit {
            /// < 200mA
            Limit200mA = sdmmc_current_limit_t_SDMMC_CURRENT_LIMIT_200MA as isize,
            /// < 400mA
            Limit400mA = sdmmc_current_limit_t_SDMMC_CURRENT_LIMIT_400MA as isize,
            /// < 600mA
            Limit600mA = sdmmc_current_limit_t_SDMMC_CURRENT_LIMIT_600MA as isize,
            /// < 800mA
            Limit800mA = sdmmc_current_limit_t_SDMMC_CURRENT_LIMIT_800MA as isize,
        }

        /// Configuration for the SD-Card driver
        #[non_exhaustive]
        pub struct Configuration {
            /// Command timeout in milliseconds. Default is 0 (no timeout)
            pub command_timeout_ms: u32,
            /// SD-Card IO voltage. Default is 3.3V;
            /// 1.8V might only be necessary for ESP32P4 and UHS-I speeds
            pub io_voltage: Voltage,
            /// SD-Card speed in kHz. Default is 20000 kHz (20 MHz)
            /// Maximum speed is usually 40000 kHz (40 MHz)
            /// Speeds lower than 20000 kHz can also be used
            pub speed_khz: u32,
            #[cfg(not(any(
                esp_idf_version_major = "4",
                all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
            )))] // For ESP-IDF v5.2 and later
            pub input_delay_phase: DelayPhase,
            #[cfg(not(any(
                esp_idf_version_major = "4",
                all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
            )))] // For ESP-IDF v5.4 and later
            pub driver_strength: DriverStrength,
            #[cfg(not(any(
                esp_idf_version_major = "4",
                all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
            )))] // For ESP-IDF v5.4 and later
            pub current_limit: CurrentLimit,
        }

        impl Configuration {
            /// Create a new configuration with default values
            pub const fn new() -> Self {
                Self {
                    command_timeout_ms: 0,
                    io_voltage: Voltage::V3P3,
                    speed_khz: 20000,
                    #[cfg(not(any(
                        esp_idf_version_major = "4",
                        all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                        all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    )))] // For ESP-IDF v5.2 and later
                    input_delay_phase: DelayPhase::Phase0,
                    #[cfg(not(any(
                        esp_idf_version_major = "4",
                        all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                        all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                        all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                        all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
                    )))] // For ESP-IDF v5.4 and later
                    driver_strength: DriverStrength::StrengthB,
                    #[cfg(not(any(
                        esp_idf_version_major = "4",
                        all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                        all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                        all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                        all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
                    )))] // For ESP-IDF v5.4 and later
                    current_limit: CurrentLimit::Limit200mA,
                }
            }
        }

        impl Default for Configuration {
            fn default() -> Self {
                Self::new()
            }
        }
    }

    /// A high-level SD-Card driver.
    ///
    /// This driver is used to interface with an SD-Card by wrapping one of the two SD Host drivers:
    /// - SD-SPI Host driver (`SdSpiHostDriver`)
    /// - SD-MMC Host driver (`SdMmcHostDriver`) - on MCUs that do have an SD-MMC peripheral (ESP32, ESP32S3, ESP32P4 and ESP32S31)
    ///
    /// The interface allows reading, writing and erasing sectors, as well as reading and writing arbitrary-length bytes.
    ///
    /// Currently, all interaction with the SD-Card driver is via the native, unsafe `sys::sdmmc_*` functions.
    pub struct SdCardDriver<T> {
        _host: T,
        card: alloc::boxed::Box<sdmmc_card_t>,
    }

    impl<T> SdCardDriver<T> {
        /// Get a reference to the SD-Card native structure.
        pub fn card(&self) -> &sdmmc_card_t {
            &self.card
        }

        // TODO: Implement the SD-Card API here, i.e. read/write/erase sectors, as well as
        // read/write of arbitrary-length bytes.
    }

    impl<'d, T> SdCardDriver<SdSpiHostDriver<'d, T>>
    where
        T: Borrow<SpiDriver<'d>>,
    {
        /// Create a new SD-Card driver using the SD-SPI host driver instantiated with one of the SPI peripherals
        pub fn new_spi(
            host: SdSpiHostDriver<'d, T>,
            configuration: &config::Configuration,
        ) -> Result<Self, EspError> {
            let configuration = sdmmc_host_t {
                flags: _SDMMC_HOST_FLAG_SPI | _SDMMC_HOST_FLAG_DEINIT_ARG,
                slot: host.handle() as _,
                max_freq_khz: configuration.speed_khz as _,
                io_voltage: configuration.io_voltage.as_native(),
                init: Some(sdspi_host_init),
                set_bus_width: None,
                get_bus_width: None,
                set_bus_ddr_mode: None,
                set_card_clk: Some(sdspi_host_set_card_clk),
                set_cclk_always_on: None,
                do_transaction: Some(sdspi_host_do_transaction),
                __bindgen_anon_1: sdmmc_host_t__bindgen_ty_1 {
                    deinit_p: Some(sdspi_host_remove_device),
                },
                io_int_enable: Some(sdspi_host_io_int_enable),
                io_int_wait: Some(sdspi_host_io_int_wait),
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                )))]    // For ESP-IDF v5.1 and later
                get_real_freq: Some(sdspi_host_get_real_freq),
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                )))]    // For ESP-IDF v5.2 and later
                input_delay_phase: configuration.input_delay_phase as _,
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                )))]   // For ESP-IDF v5.2 and later
                set_input_delay: None,
                #[cfg(esp_idf_version_at_least_6_0_0)]
                set_input_delayline: None,
                command_timeout_ms: configuration.command_timeout_ms as _,
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                )))]   // For ESP-IDF v5.3 and later
                dma_aligned_buffer: core::ptr::null_mut(),
                #[cfg(all(esp_idf_version_at_least_5_3_0, not(esp_idf_version_at_least_6_0_0)))] // For ESP-IDF v5.3 until v6.0
                get_dma_info: Some(sdspi_host_get_dma_info),
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "4"),
                )))]   // For ESP-IDF v5.5 and later
                check_buffer_alignment: Some(sdspi_host_check_buffer_alignment),
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                )))]   // For ESP-IDF v5.3 and later
                pwr_ctrl_handle: core::ptr::null_mut() as _,
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
                )))] // For ESP-IDF v5.4 and later
                driver_strength: configuration.driver_strength as _,
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
                )))] // For ESP-IDF v5.4 and later
                current_limit: configuration.current_limit as _,
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
                )))] // For ESP-IDF v5.4 and later
                is_slot_set_to_uhs1: None,
                // Added in ESP-IDF 6.0.1 and backported to ESP-IDF 5.5.5
                #[cfg(any(
                    esp_idf_version_at_least_6_0_1,
                    esp_idf_version_patch_at_least_5_5_5
                ))]
                unaligned_multi_block_rw_max_chunk_size: 0,
            };

            let mut card: alloc::boxed::Box<sdmmc_card_t> = Default::default();

            esp!(unsafe { sdmmc_card_init(&configuration, &mut *card) })?;

            Ok(Self { _host: host, card })
        }
    }

    #[cfg(esp_idf_soc_sdmmc_host_supported)]
    impl<'d> SdCardDriver<SdMmcHostDriver<'d>> {
        /// Create a new SD-Card driver using the SD-MMC Host driver instantiated with one of the two SD-MMC peripheral slots
        pub fn new_mmc(
            host: SdMmcHostDriver<'d>,
            configuration: &config::Configuration,
        ) -> Result<Self, EspError> {
            let configuration = sdmmc_host_t {
                flags: _SDMMC_HOST_FLAG_DEINIT_ARG
                    | _SDMMC_HOST_FLAG_DDR
                    // Bits 0 - 2 are flags for data widths 1, 4 and 8 respectively
                    // Set the bit corresponding to our width and all smaller widths
                    // in case the card does not support our width, but a smaller one only
                    | (1 | (host.width() - 1)) as u32,
                slot: host.slot() as _,
                max_freq_khz: configuration.speed_khz as _,
                io_voltage: configuration.io_voltage.as_native(),
                init: Some(sdmmc_host_init),
                set_bus_width: Some(sdmmc_host_set_bus_width),
                get_bus_width: Some(sdmmc_host_get_slot_width),
                set_bus_ddr_mode: Some(sdmmc_host_set_bus_ddr_mode),
                set_card_clk: Some(sdmmc_host_set_card_clk),
                set_cclk_always_on: Some(sdmmc_host_set_cclk_always_on),
                do_transaction: Some(sdmmc_host_do_transaction),
                __bindgen_anon_1: sdmmc_host_t__bindgen_ty_1 {
                    deinit: Some(sdmmc_host_deinit),
                },
                io_int_enable: Some(sdmmc_host_io_int_enable),
                io_int_wait: Some(sdmmc_host_io_int_wait),
                get_real_freq: Some(sdmmc_host_get_real_freq),
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                )))] // For ESP-IDF v5.2 and later            
                input_delay_phase: configuration.input_delay_phase as _,
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                )))] // For ESP-IDF v5.2 and later            
                set_input_delay: Some(sdmmc_host_set_input_delay),
                #[cfg(esp_idf_version_at_least_6_0_0)]
                set_input_delayline: None,
                command_timeout_ms: configuration.command_timeout_ms as _,
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                )))]   // For ESP-IDF v5.3 and later
                dma_aligned_buffer: core::ptr::null_mut(),
                #[cfg(all(esp_idf_version_at_least_5_3_0, not(esp_idf_version_at_least_6_0_0)))] // For ESP-IDF v5.3 until v6.0
                get_dma_info: Some(sdmmc_host_get_dma_info),
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "4"),
                )))]   // For ESP-IDF v5.5 and later
                check_buffer_alignment: Some(sdmmc_host_check_buffer_alignment),
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                )))]   // For ESP-IDF v5.3 and later
                pwr_ctrl_handle: core::ptr::null_mut() as _,
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
                )))] // For ESP-IDF v5.4 and later
                driver_strength: configuration.driver_strength as _,
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
                )))] // For ESP-IDF v5.4 and later
                current_limit: configuration.current_limit as _,
                #[cfg(not(any(
                    esp_idf_version_major = "4",
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "1"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "2"),
                    all(esp_idf_version_major = "5", esp_idf_version_minor = "3"),
                )))] // For ESP-IDF v5.4 and later
                is_slot_set_to_uhs1: None,
                // Added in ESP-IDF 6.0.1 and backported to ESP-IDF 5.5.5
                #[cfg(any(
                    esp_idf_version_at_least_6_0_1,
                    esp_idf_version_patch_at_least_5_5_5
                ))]
                unaligned_multi_block_rw_max_chunk_size: 0,
            };

            let mut card: alloc::boxed::Box<sdmmc_card_t> = Default::default();

            esp!(unsafe { sdmmc_card_init(&configuration, &mut *card) })?;

            Ok(Self { _host: host, card })
        }
    }
}
