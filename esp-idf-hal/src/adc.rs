//! Analog to Digital Converter peripheral control.

use core::marker::PhantomData;

use esp_idf_sys::*;

#[cfg(all(
    not(esp_idf_version_major = "4"),
    not(esp32c2),
    esp_idf_comp_esp_adc_enabled
))]
#[allow(deprecated)]
pub use continuous::{
    config as cont_config, config::Config as AdcContConfig, AdcChannels, AdcChannelsArray,
    AdcDriver as AdcContDriver, AdcMeasurement, Atten11dB, Atten12dB, Atten2p5dB, Atten6dB,
    AttenNone, Attenuated, ChainedAdcChannels, EmptyAdcChannels,
};

#[cfg(any(
    all(feature = "adc-oneshot-legacy", esp_idf_version_major = "5"),
    esp_idf_version_major = "4"
))]
pub use oneshot_legacy::*;

/// A trait designating the ADC Unit
///
/// An associated type provided by ADC peripherals
pub trait AdcUnit: 'static {
    /// Return the ESP-IDF ADC unit identifier.
    fn unit() -> adc_unit_t;
}

macro_rules! impl_adcu {
    ($adcu:ident: $unit:expr) => {
        pub struct $adcu;

        impl AdcUnit for $adcu {
            fn unit() -> adc_unit_t {
                $unit
            }
        }
    };
}

impl_adcu!(ADCU1: adc_unit_t_ADC_UNIT_1);
#[cfg(any(esp32, esp32s2, esp32s3, esp32c3, esp32p4))]
impl_adcu!(ADCU2: adc_unit_t_ADC_UNIT_2);

/// A trait designating the ADC channel
///
/// An associated type provided by ADC pins
pub trait AdcChannel: 'static {
    /// The ADC unit this channel is associated with.
    type AdcUnit: AdcUnit;

    /// Return the ESP-IDF ADC unit identifier.
    fn unit() -> adc_unit_t {
        Self::AdcUnit::unit()
    }

    /// Return the ESP-IDF ADC channel identifier.
    fn channel() -> adc_channel_t;
}

macro_rules! impl_adcch {
    ($adcch:ident: $unit:expr) => {
        pub struct $adcch<U: AdcUnit>(PhantomData<U>);

        impl<U: AdcUnit> AdcChannel for $adcch<U> {
            type AdcUnit = U;

            fn channel() -> adc_channel_t {
                $unit
            }
        }
    };
}

impl_adcch!(ADCCH0: adc_channel_t_ADC_CHANNEL_0);
impl_adcch!(ADCCH1: adc_channel_t_ADC_CHANNEL_1);
impl_adcch!(ADCCH2: adc_channel_t_ADC_CHANNEL_2);
impl_adcch!(ADCCH3: adc_channel_t_ADC_CHANNEL_3);
impl_adcch!(ADCCH4: adc_channel_t_ADC_CHANNEL_4);
impl_adcch!(ADCCH5: adc_channel_t_ADC_CHANNEL_5);
impl_adcch!(ADCCH6: adc_channel_t_ADC_CHANNEL_6);
impl_adcch!(ADCCH7: adc_channel_t_ADC_CHANNEL_7);
impl_adcch!(ADCCH8: adc_channel_t_ADC_CHANNEL_8);
impl_adcch!(ADCCH9: adc_channel_t_ADC_CHANNEL_9);

/// A trait for ADC peripherals
pub trait Adc: Send {
    /// The ADC unit this peripheral is associated with.
    type AdcUnit: AdcUnit;

    /// Return the ESP-IDF ADC unit identifier.
    fn unit() -> adc_unit_t {
        Self::AdcUnit::unit()
    }
}

// NOTE: Will be changed to an enum once C-style enums are usable as const generics
pub mod attenuation {
    pub use esp_idf_sys::{
        adc_atten_t, adc_atten_t_ADC_ATTEN_DB_0, adc_atten_t_ADC_ATTEN_DB_2_5,
        adc_atten_t_ADC_ATTEN_DB_6,
    };

    #[cfg(esp_idf_version_at_least_6_0_0)]
    pub use esp_idf_sys::adc_atten_t_ADC_ATTEN_DB_12;
    #[cfg(esp_idf_version_at_least_6_0_0)]
    #[allow(non_upper_case_globals)]
    pub const adc_atten_t_ADC_ATTEN_DB_11: adc_atten_t = adc_atten_t_ADC_ATTEN_DB_12;

    #[cfg(not(esp_idf_version_at_least_6_0_0))]
    pub use esp_idf_sys::adc_atten_t_ADC_ATTEN_DB_11;
    #[cfg(not(esp_idf_version_at_least_6_0_0))]
    #[allow(non_upper_case_globals)]
    pub const adc_atten_t_ADC_ATTEN_DB_12: adc_atten_t = adc_atten_t_ADC_ATTEN_DB_11;

    pub const NONE: adc_atten_t = adc_atten_t_ADC_ATTEN_DB_0;
    pub const DB_2_5: adc_atten_t = adc_atten_t_ADC_ATTEN_DB_2_5;
    pub const DB_6: adc_atten_t = adc_atten_t_ADC_ATTEN_DB_6;
    #[deprecated(since = "0.45.3", note = "Use `DB_12` instead")]
    pub const DB_11: adc_atten_t = adc_atten_t_ADC_ATTEN_DB_11;
    pub const DB_12: adc_atten_t = adc_atten_t_ADC_ATTEN_DB_12;
}

/// The sampling/readout resolution of the ADC
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Resolution {
    #[cfg(esp32)]
    Resolution9Bit,
    #[cfg(esp32)]
    Resolution10Bit,
    #[cfg(esp32)]
    Resolution11Bit,
    #[cfg(any(
        esp32, esp32c3, esp32s3, esp32c2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6, esp32c61,
        esp32p4
    ))]
    Resolution12Bit,
    #[cfg(esp32s2)]
    Resolution13Bit,
}

impl Resolution {
    #[cfg(not(esp32s2))]
    pub const fn new() -> Self {
        Self::Resolution12Bit
    }

    #[cfg(esp32s2)]
    pub const fn new() -> Self {
        Self::Resolution13Bit
    }
}

impl Default for Resolution {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(esp_idf_version_at_least_6_0_0)]
impl From<Resolution> for adc_bitwidth_t {
    fn from(resolution: Resolution) -> Self {
        match resolution {
            #[cfg(esp32)]
            Resolution::Resolution9Bit => adc_bitwidth_t_ADC_BITWIDTH_9,
            #[cfg(esp32)]
            Resolution::Resolution10Bit => adc_bitwidth_t_ADC_BITWIDTH_10,
            #[cfg(esp32)]
            Resolution::Resolution11Bit => adc_bitwidth_t_ADC_BITWIDTH_11,
            #[cfg(any(
                esp32, esp32s3, esp32c3, esp32c2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6,
                esp32c61, esp32p4
            ))]
            Resolution::Resolution12Bit => adc_bitwidth_t_ADC_BITWIDTH_12,
            #[cfg(esp32s2)]
            Resolution::Resolution13Bit => adc_bitwidth_t_ADC_BITWIDTH_13,
        }
    }
}

#[cfg(not(esp_idf_version_at_least_6_0_0))]
impl From<Resolution> for adc_bits_width_t {
    fn from(resolution: Resolution) -> Self {
        match resolution {
            #[cfg(esp32)]
            Resolution::Resolution9Bit => adc_bits_width_t_ADC_WIDTH_BIT_9,
            #[cfg(esp32)]
            Resolution::Resolution10Bit => adc_bits_width_t_ADC_WIDTH_BIT_10,
            #[cfg(esp32)]
            Resolution::Resolution11Bit => adc_bits_width_t_ADC_WIDTH_BIT_11,
            #[cfg(any(
                esp32, esp32s3, esp32c3, esp32c2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6,
                esp32c61, esp32p4
            ))]
            Resolution::Resolution12Bit => adc_bits_width_t_ADC_WIDTH_BIT_12,
            #[cfg(esp32s2)]
            Resolution::Resolution13Bit => adc_bits_width_t_ADC_WIDTH_BIT_13,
        }
    }
}

#[cfg(any(
    all(feature = "adc-oneshot-legacy", esp_idf_version_major = "5"),
    esp_idf_version_major = "4"
))]
mod oneshot_legacy {
    use core::marker::PhantomData;

    use esp_idf_sys::*;

    use crate::{
        adc::{AdcChannel, AdcUnit},
        gpio::ADCPin,
    };

    use super::{to_nb_err, Adc, DirectConverter};

    pub type AdcConfig = config::Config;

    /// ADC configuration
    pub mod config {
        pub use crate::adc::Resolution;

        #[derive(Debug, Copy, Clone, Default)]
        pub struct Config {
            pub resolution: Resolution,
            #[cfg(any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled))]
            pub calibration: bool,
        }

        impl Config {
            pub fn new() -> Self {
                Default::default()
            }

            #[must_use]
            pub fn resolution(mut self, resolution: Resolution) -> Self {
                self.resolution = resolution;
                self
            }

            #[cfg(any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled))]
            #[must_use]
            pub fn calibration(mut self, calibration: bool) -> Self {
                self.calibration = calibration;
                self
            }
        }
    }

    pub struct AdcChannelDriver<'d, const A: adc_atten_t, C: AdcChannel> {
        _channel: PhantomData<C>,
        _t: PhantomData<&'d mut ()>,
    }

    impl<'d, const A: adc_atten_t, C: AdcChannel> AdcChannelDriver<'d, A, C>
    where
        C: AdcChannel,
    {
        pub fn new(pin: impl ADCPin<AdcChannel = C> + 'd) -> Result<Self, EspError> {
            unsafe {
                crate::gpio::rtc_reset_pin(pin.pin() as _)?;
            }

            if C::unit() == adc_unit_t_ADC_UNIT_1 {
                esp!(unsafe { adc1_config_channel_atten(C::channel(), A) })?;
            } else {
                #[cfg(not(any(
                    esp32c2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6, esp32c61, esp32p4
                )))]
                esp!(unsafe { adc2_config_channel_atten(C::channel(), A) })?;

                #[cfg(any(
                    esp32c2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6, esp32c61, esp32p4
                ))]
                unreachable!();
            }

            Ok(Self {
                _channel: PhantomData,
                _t: PhantomData,
            })
        }
    }

    impl<const A: adc_atten_t, C: AdcChannel> embedded_hal_0_2::adc::Channel<C::AdcUnit>
        for AdcChannelDriver<'_, A, C>
    {
        type ID = (adc_channel_t, adc_atten_t);

        fn channel() -> Self::ID {
            (C::channel(), A)
        }
    }

    pub struct AdcDriver<'d, ADCU: AdcUnit> {
        #[allow(dead_code)]
        resolution: config::Resolution,
        #[cfg(all(
            any(esp32, esp32s2, esp32s3, esp32c3),
            any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled)
        ))]
        cal_characteristics: Option<
            [Option<esp_adc_cal_characteristics_t>; adc_atten_t_ADC_ATTEN_DB_11 as usize + 1],
        >,
        _unit: PhantomData<ADCU>,
        _t: PhantomData<&'d mut ()>,
    }

    unsafe impl<ADCU: AdcUnit> Send for AdcDriver<'_, ADCU> {}

    impl<'d, ADCU: AdcUnit> AdcDriver<'d, ADCU> {
        #[cfg(all(
            esp32,
            any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled)
        ))]
        const CALIBRATION_SCHEME: esp_adc_cal_value_t =
            esp_adc_cal_value_t_ESP_ADC_CAL_VAL_EFUSE_VREF;

        #[cfg(all(
            any(esp32c3, esp32s2),
            any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled)
        ))]
        const CALIBRATION_SCHEME: esp_adc_cal_value_t =
            esp_adc_cal_value_t_ESP_ADC_CAL_VAL_EFUSE_TP;

        #[cfg(all(
            esp32s3,
            any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled)
        ))]
        const CALIBRATION_SCHEME: esp_adc_cal_value_t =
            esp_adc_cal_value_t_ESP_ADC_CAL_VAL_EFUSE_TP_FIT;

        pub fn new<ADC: Adc<AdcUnit = ADCU> + 'd>(
            _adc: ADC,
            config: &config::Config,
        ) -> Result<Self, EspError> {
            #[cfg(all(
                any(esp32, esp32s2, esp32s3, esp32c3),
                any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled)
            ))]
            if config.calibration {
                esp!(unsafe { esp_adc_cal_check_efuse(Self::CALIBRATION_SCHEME) })?;
            }

            if ADC::unit() == adc_unit_t_ADC_UNIT_1 {
                esp!(unsafe { adc1_config_width(config.resolution.into()) })?;
            }

            Ok(Self {
                resolution: config.resolution,
                #[cfg(all(
                    any(esp32, esp32s2, esp32s3, esp32c3),
                    any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled)
                ))]
                cal_characteristics: if config.calibration {
                    Some(Default::default())
                } else {
                    None
                },
                _unit: PhantomData,
                _t: PhantomData,
            })
        }

        #[inline(always)]
        pub fn read<const A: adc_atten_t, C: AdcChannel<AdcUnit = ADCU>>(
            &mut self,
            _pin: &mut AdcChannelDriver<'_, A, C>,
        ) -> Result<u16, EspError> {
            self.read_internal(C::unit(), C::channel(), A)
        }

        #[inline(always)]
        pub fn read_raw<const A: adc_atten_t, C: AdcChannel<AdcUnit = ADCU>>(
            &mut self,
            _pin: &mut AdcChannelDriver<'_, A, C>,
        ) -> Result<u16, EspError> {
            self.read_internal_raw(C::unit(), C::channel())
        }

        #[inline(always)]
        fn read_internal(
            &mut self,
            unit: adc_unit_t,
            channel: adc_channel_t,
            atten: adc_atten_t,
        ) -> Result<u16, EspError> {
            let measurement = self.read_internal_raw(unit, channel)?;
            self.raw_to_voltage(measurement, atten)
        }

        #[inline(always)]
        fn read_internal_raw(
            &mut self,
            unit: adc_unit_t,
            channel: adc_channel_t,
        ) -> Result<u16, EspError> {
            if unit == adc_unit_t_ADC_UNIT_1 {
                Ok(unsafe { adc1_get_raw(channel) } as _)
            } else {
                #[cfg(not(any(
                    esp32c2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6, esp32c61, esp32p4
                )))]
                {
                    let mut measurement = 0;
                    esp!(unsafe {
                        adc2_get_raw(channel, self.resolution.into(), &mut measurement)
                    })?;

                    Ok(measurement as _)
                }

                #[cfg(any(
                    esp32c2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6, esp32c61, esp32p4
                ))]
                unreachable!();
            }
        }

        #[inline(always)]
        fn raw_to_voltage(
            &mut self,
            measurement: u16,
            attenuation: adc_atten_t,
        ) -> Result<u16, EspError> {
            #[cfg(all(
                any(esp32, esp32s2, esp32s3, esp32c3),
                any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled)
            ))]
            let mv = if let Some(cal) = self.get_cal_characteristics(attenuation)? {
                unsafe { esp_adc_cal_raw_to_voltage(measurement as u32, &cal) as u16 }
            } else {
                DirectConverter(attenuation).raw_to_mv(measurement)
            };

            #[cfg(not(all(
                any(esp32, esp32s2, esp32s3, esp32c3),
                any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled)
            )))]
            let mv = DirectConverter(attenuation).raw_to_mv(measurement);

            Ok(mv)
        }

        #[cfg(all(
            any(esp32, esp32s2, esp32s3, esp32c3),
            any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled)
        ))]
        fn get_cal_characteristics(
            &mut self,
            attenuation: adc_atten_t,
        ) -> Result<Option<esp_adc_cal_characteristics_t>, EspError> {
            if let Some(characteristics) = &mut self.cal_characteristics {
                if let Some(cal) = characteristics[attenuation as usize] {
                    Ok(Some(cal))
                } else {
                    esp!(unsafe { esp_adc_cal_check_efuse(Self::CALIBRATION_SCHEME) })?;

                    let mut cal: esp_adc_cal_characteristics_t = Default::default();
                    unsafe {
                        esp_adc_cal_characterize(
                            ADCU::unit(),
                            attenuation,
                            self.resolution.into(),
                            0,
                            &mut cal,
                        )
                    };

                    characteristics[attenuation as usize] = Some(cal);

                    Ok(Some(cal))
                }
            } else {
                Ok(None)
            }
        }
    }

    impl<'c, const A: adc_atten_t, C>
        embedded_hal_0_2::adc::OneShot<C::AdcUnit, u16, AdcChannelDriver<'c, A, C>>
        for AdcDriver<'_, C::AdcUnit>
    where
        C: AdcChannel,
    {
        type Error = EspError;

        fn read(&mut self, _pin: &mut AdcChannelDriver<'c, A, C>) -> nb::Result<u16, Self::Error> {
            self.read_internal(C::unit(), C::channel(), A)
                .map_err(to_nb_err)
        }
    }
}

fn to_nb_err(err: EspError) -> nb::Error<EspError> {
    if err.code() == ESP_ERR_INVALID_STATE {
        nb::Error::WouldBlock
    } else {
        nb::Error::Other(err)
    }
}

macro_rules! impl_adc {
    ($adc:ident: $unit:ident) => {
        crate::impl_peripheral!($adc);

        impl Adc for $adc<'_> {
            type AdcUnit = $unit;
        }
    };
}

impl_adc!(ADC1: ADCU1);
#[cfg(not(any(esp32c2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6, esp32c61)))] // TODO: Check for esp32c5
impl_adc!(ADC2: ADCU2);

/// Converts a raw reading to mV without using calibration
struct DirectConverter(adc_atten_t);

impl DirectConverter {
    #[cfg(not(esp32s2))]
    const MAX_READING: u32 = 4095;

    #[cfg(esp32s2)]
    const MAX_READING: u32 = 8191;

    fn raw_to_mv(&self, raw: u16) -> u16 {
        (raw as u32 * self.get_max_mv() as u32 / Self::MAX_READING) as u16
    }

    #[inline(always)]
    #[allow(non_upper_case_globals)]
    fn get_max_mv(&self) -> u16 {
        let attenuation = self.0;

        #[cfg(esp32)]
        let mv = match attenuation {
            attenuation::NONE => 950,
            attenuation::DB_2_5 => 1250,
            attenuation::DB_6 => 1750,
            attenuation::DB_12 => 2450,
            other => panic!("Unknown attenuation: {other}"),
        };

        #[cfg(any(
            esp32c3, esp32s2, esp32c2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6, esp32c61,
            esp32p4
        ))]
        let mv = match attenuation {
            attenuation::NONE => 750,
            attenuation::DB_2_5 => 1050,
            attenuation::DB_6 => 1300,
            attenuation::DB_12 => 2500,
            other => panic!("Unknown attenuation: {other}"),
        };

        #[cfg(esp32s3)]
        let mv = match attenuation {
            attenuation::NONE => 950,
            attenuation::DB_2_5 => 1250,
            attenuation::DB_6 => 1750,
            attenuation::DB_12 => 3100,
            other => panic!("Unknown attenuation: {other}"),
        };

        mv
    }
}

/// One-shot ADC module
/// Example: reading a value form a pin and printing it on the terminal
/// ```
/// use std::thread;
/// use std::time::Duration;
///
/// fn main() -> anyhow::Result<()> {
///     use esp_idf_hal::adc::attenuation::DB_12;
///     use esp_idf_hal::adc::oneshot::config::AdcChannelConfig;
///     use esp_idf_hal::adc::oneshot::*;
///     use esp_idf_hal::peripherals::Peripherals;
///
///     let peripherals = Peripherals::take()?;
///     let adc = AdcDriver::new(peripherals.adc1)?;
///
///     /// configuring pin to analog read, you can regulate the adc input voltage range depending on your need
///     /// for this example we use the attenuation of 11db which sets the input voltage range to around 0-3.6V
///     let config = AdcChannelConfig {
///         attenuation: DB_12,
///         ..Default::default()
///     };
///     let mut adc_pin = AdcChannelDriver::new(&adc, peripherals.pins.gpio2, &config)?;
///
///     loop {
///         /// you can change the sleep duration depending on how often you want to sample
///         thread::sleep(Duration::from_millis(100));
///         println!("ADC value: {}", adc.read(&mut adc_pin)?);
///     }
/// }
/// ```
#[cfg(all(
    not(all(feature = "adc-oneshot-legacy", esp_idf_version_major = "5")),
    not(esp_idf_version_major = "4"),
    esp_idf_comp_esp_adc_enabled
))]
pub mod oneshot {
    use core::borrow::Borrow;
    use core::marker::PhantomData;

    use esp_idf_sys::*;

    use crate::adc::to_nb_err;
    use crate::adc::AdcChannel;
    use crate::gpio::ADCPin;

    use super::attenuation::adc_atten_t;
    use super::Adc;
    use super::AdcUnit;
    use super::DirectConverter;

    pub mod config {
        use super::adc_atten_t;

        pub use crate::adc::Resolution;

        #[derive(Debug, Copy, Clone, Default, Eq, PartialEq, Hash)]
        pub enum Calibration {
            #[default]
            None,
            #[cfg(all(
                any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
                any(
                    esp32c3,
                    all(
                        esp32c6,
                        not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0")),
                        not(esp_idf_version_full = "5.1.0")
                    ),
                    esp32s3,
                )
            ))]
            Curve,
            #[cfg(all(
                any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
                any(esp32, esp32c2, esp32s2)
            ))]
            Line,
        }

        #[derive(Debug, Copy, Clone, Default)]
        pub struct AdcChannelConfig {
            pub attenuation: adc_atten_t,
            pub resolution: Resolution,
            pub calibration: Calibration,
        }

        impl AdcChannelConfig {
            pub const fn new() -> Self {
                Self {
                    attenuation: crate::adc::attenuation::NONE,
                    resolution: Resolution::new(),
                    calibration: Calibration::None,
                }
            }
        }
    }

    /// Converts a raw reading to mV with or without calibration
    enum Converter {
        NoCalibration(adc_atten_t),
        #[cfg(all(
            any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
            any(
                esp32c3,
                all(
                    esp32c6,
                    not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0")),
                    not(esp_idf_version_full = "5.1.0")
                ),
                esp32s3,
            )
        ))]
        CurveFittingCalibration(adc_cali_handle_t),
        #[cfg(all(
            any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
            any(esp32, esp32c2, esp32s2)
        ))]
        LineFittingCalibration(adc_cali_handle_t),
    }

    impl Converter {
        #[allow(unused_variables)]
        fn create(
            unit: adc_unit_t,
            chan: adc_channel_t,
            atten: adc_atten_t,
            #[cfg(esp_idf_version_at_least_6_0_0)] bitwidth: adc_bitwidth_t,
            #[cfg(not(esp_idf_version_at_least_6_0_0))] bitwidth: adc_bits_width_t,
            calibration: config::Calibration,
        ) -> Result<Self, EspError> {
            match calibration {
                config::Calibration::None => Ok(Self::NoCalibration(atten)),
                #[cfg(all(
                    any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
                    any(
                        esp32c3,
                        all(
                            esp32c6,
                            not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0")),
                            not(esp_idf_version_full = "5.1.0")
                        ),
                        esp32s3,
                    )
                ))]
                config::Calibration::Curve => {
                    // it would be nice if esp-idf-sys could export some cfg values to replicate these two defines
                    // from esp-idf:
                    // ADC_CALI_SCHEME_CURVE_FITTING_SUPPORTED
                    // ADC_CALI_SCHEME_LINE_FITTING_SUPPORTED
                    // then we wouuld not need the ugliness for the esp32c6
                    let cal_config = adc_cali_curve_fitting_config_t {
                        unit_id: unit,
                        #[cfg(esp_idf_version_at_least_5_1_1)]
                        chan,
                        atten,
                        bitwidth,
                    };
                    let mut cal_handle: adc_cali_handle_t = core::ptr::null_mut();
                    esp!(unsafe {
                        esp_idf_sys::adc_cali_create_scheme_curve_fitting(
                            &cal_config,
                            &mut cal_handle,
                        )
                    })?;

                    Ok(Self::CurveFittingCalibration(cal_handle))
                }
                #[cfg(all(
                    any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
                    any(esp32, esp32c2, esp32s2)
                ))]
                config::Calibration::Line => {
                    // esp32 has an additional field that the exanple defaults
                    // to using fuse values for vref. Maybe we should expose
                    // this as a config option?
                    #[allow(clippy::needless_update)]
                    let cal_config = adc_cali_line_fitting_config_t {
                        unit_id: unit,
                        atten,
                        bitwidth,
                        ..Default::default()
                    };
                    let mut cal_handle: adc_cali_handle_t = core::ptr::null_mut();
                    esp!(unsafe {
                        esp_idf_sys::adc_cali_create_scheme_line_fitting(
                            &cal_config,
                            &mut cal_handle,
                        )
                    })?;

                    Ok(Self::LineFittingCalibration(cal_handle))
                }
            }
        }

        fn raw_to_mv(&self, raw: u16) -> Result<u16, EspError> {
            match self {
                Self::NoCalibration(atten) => Ok(DirectConverter(*atten).raw_to_mv(raw)),
                #[cfg(all(
                    any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
                    any(
                        esp32c3,
                        all(
                            esp32c6,
                            not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0")),
                            not(esp_idf_version_full = "5.1.0")
                        ),
                        esp32s3,
                    )
                ))]
                Self::CurveFittingCalibration(handle) => {
                    let mut mv = 0i32;
                    esp!(unsafe { adc_cali_raw_to_voltage(*handle, raw as i32, &mut mv) })?;

                    Ok(mv as u16)
                }
                #[cfg(all(
                    any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
                    any(esp32, esp32c2, esp32s2)
                ))]
                Self::LineFittingCalibration(handle) => {
                    let mut mv = 0i32;
                    esp!(unsafe { adc_cali_raw_to_voltage(*handle, raw as i32, &mut mv) })?;

                    Ok(mv as u16)
                }
            }
        }
    }

    impl Drop for Converter {
        fn drop(&mut self) {
            match self {
                Self::NoCalibration(_) => (),
                #[cfg(all(
                    any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
                    any(
                        esp32c3,
                        all(
                            esp32c6,
                            not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0")),
                            not(esp_idf_version_full = "5.1.0")
                        ),
                        esp32s3,
                    )
                ))]
                Self::CurveFittingCalibration(handle) => {
                    esp!(unsafe { esp_idf_sys::adc_cali_delete_scheme_curve_fitting(*handle) })
                        .unwrap()
                }
                #[cfg(all(
                    any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
                    any(esp32, esp32c2, esp32s2)
                ))]
                Self::LineFittingCalibration(handle) => {
                    esp!(unsafe { esp_idf_sys::adc_cali_delete_scheme_line_fitting(*handle) })
                        .unwrap()
                }
            }
        }
    }

    pub struct AdcChannelDriver<'d, C, M>
    where
        C: AdcChannel,
        M: Borrow<AdcDriver<'d, C::AdcUnit>>,
    {
        adc: M,
        _channel: PhantomData<C>,
        converter: Converter,
        _t: PhantomData<&'d mut ()>,
    }

    impl<'d, C, M> AdcChannelDriver<'d, C, M>
    where
        C: AdcChannel,
        M: Borrow<AdcDriver<'d, C::AdcUnit>>,
    {
        pub fn new(
            adc: M,
            pin: impl ADCPin<AdcChannel = C> + 'd,
            config: &config::AdcChannelConfig,
        ) -> Result<Self, EspError> {
            unsafe {
                crate::gpio::rtc_reset_pin(pin.pin() as _)?;
            }

            let chan_config = adc_oneshot_chan_cfg_t {
                atten: config.attenuation,
                bitwidth: config.resolution.into(),
            };

            let converter = Converter::create(
                C::unit(),
                C::channel(),
                config.attenuation,
                config.resolution.into(),
                config.calibration,
            )?;

            unsafe {
                esp!(adc_oneshot_config_channel(
                    adc.borrow().handle,
                    C::channel(),
                    &chan_config
                ))?
            };

            Ok(Self {
                adc,
                _channel: PhantomData,
                converter,
                _t: PhantomData,
            })
        }

        #[inline(always)]
        pub fn read(&mut self) -> Result<u16, EspError> {
            let raw = self.read_raw()?;
            self.raw_to_mv(raw)
        }

        #[inline(always)]
        pub fn read_raw(&mut self) -> Result<u16, EspError> {
            let channel = C::channel();
            self.adc.borrow().read_raw_internal(channel)
        }

        #[inline(always)]
        pub fn raw_to_mv(&self, raw: u16) -> Result<u16, EspError> {
            self.converter.raw_to_mv(raw)
        }
    }

    impl<'d, C, M> embedded_hal_0_2::adc::Channel<C::AdcUnit> for AdcChannelDriver<'d, C, M>
    where
        C: AdcChannel,
        M: Borrow<AdcDriver<'d, C::AdcUnit>>,
    {
        type ID = adc_channel_t;

        fn channel() -> Self::ID {
            C::channel()
        }
    }

    unsafe impl<'d, C, M> Send for AdcChannelDriver<'d, C, M>
    where
        C: AdcChannel,
        M: Borrow<AdcDriver<'d, C::AdcUnit>>,
    {
    }

    pub struct AdcDriver<'d, U> {
        handle: adc_oneshot_unit_handle_t,
        _unit: PhantomData<U>,
        _t: PhantomData<&'d mut ()>,
    }

    impl<'d, U> AdcDriver<'d, U>
    where
        U: AdcUnit + 'd,
    {
        pub fn new<ADC: Adc<AdcUnit = U> + 'd>(_adc: ADC) -> Result<Self, EspError> {
            let config = adc_oneshot_unit_init_cfg_t {
                unit_id: ADC::unit(),
                ..Default::default()
            };
            let mut handle: adc_oneshot_unit_handle_t = core::ptr::null_mut();
            unsafe { esp!(adc_oneshot_new_unit(&config, &mut handle))? };
            Ok(Self {
                handle,
                _unit: PhantomData,
                _t: PhantomData,
            })
        }

        #[inline(always)]
        pub fn read<C, M>(&self, channel: &mut AdcChannelDriver<'d, C, M>) -> Result<u16, EspError>
        where
            C: AdcChannel<AdcUnit = U>,
            M: Borrow<AdcDriver<'d, U>>,
        {
            let raw = self.read_raw(channel)?;
            self.raw_to_mv(channel, raw)
        }

        #[inline(always)]
        pub fn read_raw<C, M>(
            &self,
            _channel: &mut AdcChannelDriver<'d, C, M>,
        ) -> Result<u16, EspError>
        where
            C: AdcChannel<AdcUnit = U>,
            M: Borrow<AdcDriver<'d, U>>,
        {
            self.read_raw_internal(C::channel())
        }

        #[inline(always)]
        fn read_raw_internal(&self, channel: adc_channel_t) -> Result<u16, EspError> {
            let mut measurement = 0;
            unsafe { esp!(adc_oneshot_read(self.handle, channel, &mut measurement)) }?;
            Ok(measurement as u16)
        }

        #[inline(always)]
        pub fn raw_to_mv<C, M>(
            &self,
            channel: &AdcChannelDriver<'d, C, M>,
            raw: u16,
        ) -> Result<u16, EspError>
        where
            C: AdcChannel<AdcUnit = U>,
            M: Borrow<AdcDriver<'d, U>>,
        {
            channel.converter.raw_to_mv(raw)
        }
    }

    impl<U> Drop for AdcDriver<'_, U> {
        fn drop(&mut self) {
            unsafe { esp!(adc_oneshot_del_unit(self.handle)) }.unwrap();
        }
    }

    impl<'d, C, M> embedded_hal_0_2::adc::OneShot<C::AdcUnit, u16, AdcChannelDriver<'d, C, M>>
        for AdcDriver<'d, C::AdcUnit>
    where
        C: AdcChannel,
        M: Borrow<AdcDriver<'d, C::AdcUnit>>,
    {
        type Error = EspError;

        fn read(&mut self, pin: &mut AdcChannelDriver<'d, C, M>) -> nb::Result<u16, Self::Error> {
            AdcDriver::read(self, pin).map_err(to_nb_err)
        }
    }

    unsafe impl<U: AdcUnit> Send for AdcDriver<'_, U> {}
    unsafe impl<U: AdcUnit> Sync for AdcDriver<'_, U> {}
}

/// Continuous ADC module
/// Example: continuously reading value from a pin
/// ```
/// use ::log::{info, debug};
///
/// fn main() -> anyhow::Result<()> {
///     use esp_idf_svc::hal::adc::{AdcContConfig, AdcContDriver, AdcMeasurement, Attenuated};
///     use esp_idf_svc::hal::modem::Modem;
///     use esp_idf_svc::hal::peripherals::Peripherals;
///     use esp_idf_svc::sys::EspError;
///
///     esp_idf_svc::sys::link_patches();
///     esp_idf_svc::log::EspLogger::initialize_default();
///
///     let peripherals = Peripherals::take()?;
///     let config = AdcContConfig::default();
///
///     let adc_1_channel_0 = Attenuated::db12(peripherals.pins.gpio0);
///     let mut adc = AdcContDriver::new(peripherals.adc1, &config, adc_1_channel_0)?;
///
///     adc.start()?;
///
///     /// Default to just read 100 measurements per each read
///     let mut samples = [AdcMeasurement::default(); 100];
///
///     loop {
///         if let Ok(num_read) = adc.read(&mut samples, 10) {
///             debug!("Read {} measurement.", num_read);
///             for index in 0..num_read {
///                 debug!("{}", samples[index].data());
///             }
///         }
///     }
/// }
/// ```
#[cfg(all(
    not(esp_idf_version_major = "4"),
    not(esp32c2),
    esp_idf_comp_esp_adc_enabled
))]
pub mod continuous {
    use core::ffi::c_void;
    use core::fmt::{self, Debug, Display};
    use core::marker::PhantomData;

    use esp_idf_sys::*;

    use crate::adc::AdcChannel;
    use crate::delay::{self, TickType};
    use crate::gpio::ADCPin;
    use crate::interrupt::asynch::HalIsrNotification;
    use crate::io::EspIOError;

    use super::{attenuation, Adc, AdcUnit};

    /// Set ADC attenuation level
    /// Example: let pin = Attenuated::db12(peripherals.pins.gpio0);
    pub struct Attenuated<const A: adc_atten_t, T>(T);

    impl<T> Attenuated<{ attenuation::NONE }, T> {
        pub const fn none(t: T) -> Self {
            Self(t)
        }
    }

    impl<T> Attenuated<{ attenuation::DB_2_5 }, T> {
        pub const fn db2_5(t: T) -> Self {
            Self(t)
        }
    }

    impl<T> Attenuated<{ attenuation::DB_6 }, T> {
        pub const fn db6(t: T) -> Self {
            Self(t)
        }
    }

    #[allow(deprecated)]
    impl<T> Attenuated<{ attenuation::DB_11 }, T> {
        #[cfg_attr(
            not(esp_idf_version_major = "4"),
            deprecated(since = "0.45.3", note = "Use `Attenuated::db12` instead")
        )]
        pub const fn db11(t: T) -> Self {
            Self(t)
        }
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    impl<T> Attenuated<{ attenuation::DB_12 }, T> {
        pub const fn db12(t: T) -> Self {
            Self(t)
        }
    }

    impl<const A: adc_atten_t, T> Attenuated<A, T> {
        pub fn atten(channel: (adc_channel_t, adc_atten_t)) -> (adc_channel_t, adc_atten_t) {
            (channel.0, A)
        }
    }

    pub type AttenNone<T> = Attenuated<{ attenuation::NONE }, T>;
    pub type Atten2p5dB<T> = Attenuated<{ attenuation::DB_2_5 }, T>;
    pub type Atten6dB<T> = Attenuated<{ attenuation::DB_6 }, T>;
    #[deprecated(since = "0.45.3", note = "Use `Atten12dB` instead")]
    #[allow(deprecated)]
    pub type Atten11dB<T> = Attenuated<{ attenuation::DB_11 }, T>;
    pub type Atten12dB<T> = Attenuated<{ attenuation::DB_12 }, T>;

    pub trait AdcChannels {
        type AdcUnit: AdcUnit;

        type Iterator<'a>: Iterator<Item = (adc_channel_t, adc_atten_t)>
        where
            Self: 'a;

        fn iter(&self) -> Self::Iterator<'_>;
    }

    impl<P> AdcChannels for P
    where
        P: ADCPin,
    {
        type AdcUnit = <P::AdcChannel as AdcChannel>::AdcUnit;

        type Iterator<'a>
            = core::iter::Once<(adc_channel_t, adc_atten_t)>
        where
            Self: 'a;

        fn iter(&self) -> Self::Iterator<'_> {
            core::iter::once((P::AdcChannel::channel(), attenuation::NONE))
        }
    }

    impl<const A: adc_atten_t, C> AdcChannels for Attenuated<A, C>
    where
        C: AdcChannels,
    {
        type AdcUnit = C::AdcUnit;

        type Iterator<'a>
            = core::iter::Map<
            C::Iterator<'a>,
            fn((adc_channel_t, adc_atten_t)) -> (adc_channel_t, adc_atten_t),
        >
        where
            Self: 'a;

        fn iter(&self) -> Self::Iterator<'_> {
            self.0.iter().map(Attenuated::<A, C>::atten)
        }
    }

    pub struct AdcChannelsArray<C, const N: usize>(pub [C; N]);

    impl<C, const N: usize> AdcChannels for AdcChannelsArray<C, N>
    where
        C: AdcChannels,
    {
        type AdcUnit = C::AdcUnit;

        type Iterator<'a>
            = core::iter::FlatMap<
            core::slice::Iter<'a, C>,
            <C as AdcChannels>::Iterator<'a>,
            fn(&'a C) -> C::Iterator<'a>,
        >
        where
            Self: 'a;

        fn iter(&self) -> Self::Iterator<'_> {
            self.0.iter().flat_map(AdcChannels::iter)
        }
    }

    pub struct EmptyAdcChannels<A>(PhantomData<A>);

    impl<A> EmptyAdcChannels<A> {
        pub fn chain<O>(other: O) -> ChainedAdcChannels<Self, O>
        where
            A: Adc,
            O: AdcChannels<AdcUnit = A>,
        {
            ChainedAdcChannels {
                first: Self(PhantomData),
                second: other,
            }
        }
    }

    impl<A> AdcChannels for EmptyAdcChannels<A>
    where
        A: AdcUnit,
    {
        type AdcUnit = A;

        type Iterator<'a>
            = core::iter::Empty<(adc_channel_t, adc_atten_t)>
        where
            Self: 'a;

        fn iter(&self) -> Self::Iterator<'_> {
            core::iter::empty()
        }
    }

    pub struct ChainedAdcChannels<F, S> {
        first: F,
        second: S,
    }

    impl<F, S> ChainedAdcChannels<F, S> {
        pub fn chain<O>(self, other: O) -> ChainedAdcChannels<Self, O>
        where
            F: AdcChannels,
            S: AdcChannels<AdcUnit = F::AdcUnit>,
            O: AdcChannels<AdcUnit = F::AdcUnit>,
        {
            ChainedAdcChannels {
                first: self,
                second: other,
            }
        }
    }

    impl<F, S> AdcChannels for ChainedAdcChannels<F, S>
    where
        F: AdcChannels,
        S: AdcChannels<AdcUnit = F::AdcUnit>,
    {
        type AdcUnit = F::AdcUnit;

        type Iterator<'a>
            = core::iter::Chain<F::Iterator<'a>, S::Iterator<'a>>
        where
            Self: 'a;

        fn iter(&self) -> Self::Iterator<'_> {
            self.first.iter().chain(self.second.iter())
        }
    }

    #[derive(Copy, Clone)]
    #[repr(transparent)]
    pub struct AdcMeasurement(adc_digi_output_data_t);

    impl Default for AdcMeasurement {
        fn default() -> Self {
            Self::new()
        }
    }

    impl AdcMeasurement {
        pub const INIT: Self = AdcMeasurement(unsafe {
            core::mem::transmute::<
                [u8; core::mem::size_of::<adc_digi_output_data_t>()],
                adc_digi_output_data_t,
            >([0u8; core::mem::size_of::<adc_digi_output_data_t>()])
        });

        pub const fn new() -> Self {
            Self::INIT
        }

        #[cfg(any(esp32, esp32s2))]
        pub fn data(&self) -> u16 {
            unsafe { self.0.__bindgen_anon_1.type1.data() as _ }
        }

        #[cfg(any(esp32, esp32s2))]
        pub fn channel(&self) -> adc_channel_t {
            unsafe { self.0.__bindgen_anon_1.type1.channel() as _ }
        }

        #[cfg(not(any(esp32, esp32s2)))]
        pub fn data(&self) -> u16 {
            unsafe { self.0.__bindgen_anon_1.type2.data() as _ }
        }

        #[cfg(not(any(esp32, esp32s2)))]
        pub fn channel(&self) -> adc_channel_t {
            unsafe { self.0.__bindgen_anon_1.type2.channel() as _ }
        }

        #[cfg(not(any(esp32, esp32s2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6, esp32c61)))]
        pub fn unit(&self) -> adc_unit_t {
            unsafe { self.0.__bindgen_anon_1.type2.unit() as _ }
        }

        #[cfg(any(esp32, esp32s2))]
        pub fn nullify(&mut self) {
            self.0.__bindgen_anon_1.val = self.data();
        }

        #[cfg(not(any(esp32, esp32s2)))]
        pub fn nullify(&mut self) {
            self.0.__bindgen_anon_1.val = self.data() as _;
        }

        #[cfg(any(esp32, esp32s2))]
        pub fn as_pcm16(data: &mut [AdcMeasurement]) -> &mut [u16] {
            for measurement in data.iter_mut() {
                measurement.nullify();
            }

            unsafe { core::slice::from_raw_parts_mut(data.as_mut_ptr() as *mut _, data.len()) }
        }

        #[cfg(not(any(esp32, esp32s2)))]
        pub fn as_pcm32(data: &mut [AdcMeasurement]) -> &mut [u32] {
            for measurement in data.iter_mut() {
                measurement.nullify();
            }

            unsafe { core::slice::from_raw_parts_mut(data.as_mut_ptr() as *mut _, data.len()) }
        }
    }

    impl Display for AdcMeasurement {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "ADC Reading {{channel={}, data={}}}",
                self.channel(),
                self.data()
            )
        }
    }

    impl Debug for AdcMeasurement {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "AdcMeasurement {{channel: {}, data: {}}}",
                self.channel(),
                self.data()
            )
        }
    }

    pub mod config {
        use crate::units::*;

        /// ADC continuous mode driver configurations.
        /// The default configuration is:
        /// * [`sample_freq`][Config::sample_freq]: 2000 Hz
        /// * [`frame_measurements`][Config::frame_measurements]: 100
        /// * [`frames_count`][Config::frames_count]: 10
        #[derive(Debug, Copy, Clone)]
        pub struct Config {
            pub sample_freq: Hertz,
            pub frame_measurements: usize,
            pub frames_count: usize,
        }

        impl Config {
            pub const fn new() -> Self {
                Self {
                    sample_freq: Hertz(20000),
                    frame_measurements: 100,
                    frames_count: 10,
                }
            }

            #[must_use]
            pub fn sample_freq(mut self, sample_freq: Hertz) -> Self {
                self.sample_freq = sample_freq;
                self
            }

            #[must_use]
            pub fn frame_measurements(mut self, frame_measurements: usize) -> Self {
                self.frame_measurements = frame_measurements;
                self
            }

            #[must_use]
            pub fn frames_count(mut self, frames_count: usize) -> Self {
                self.frames_count = frames_count;
                self
            }
        }

        impl Default for Config {
            fn default() -> Self {
                Self::new()
            }
        }
    }

    pub struct AdcDriver<'d> {
        handle: adc_continuous_handle_t,
        adc: u8,
        _ref: PhantomData<&'d ()>,
    }

    impl<'d> AdcDriver<'d> {
        /// Initialize ADC continuous driver with configuration and channels.
        #[cfg(esp32)]
        pub fn new(
            adc: super::ADC1<'d>,
            _i2s: crate::i2s::I2S0<'d>,
            config: &config::Config,
            channels: impl AdcChannels<AdcUnit = super::ADCU1> + 'd,
        ) -> Result<Self, EspError> {
            Self::internal_new(adc, config, channels)
        }

        /// Initialize ADC continuous driver with configuration and channels
        #[cfg(esp32s2)]
        pub fn new(
            adc: super::ADC1<'d>,
            _spi: crate::spi::SPI3<'d>,
            config: &config::Config,
            channels: impl AdcChannels<AdcUnit = super::ADCU1> + 'd,
        ) -> Result<Self, EspError> {
            Self::internal_new(adc, config, channels)
        }

        /// Initialize ADC continuous driver with configuration and channels.
        #[cfg(not(any(esp32, esp32s2)))]
        pub fn new<A: Adc + 'd>(
            adc: A,
            config: &config::Config,
            channels: impl AdcChannels<AdcUnit = A::AdcUnit> + 'd,
        ) -> Result<Self, EspError> {
            Self::internal_new(adc, config, channels)
        }

        fn internal_new<A: Adc + 'd>(
            _adc: A,
            config: &config::Config,
            channels: impl AdcChannels<AdcUnit = A::AdcUnit> + 'd,
        ) -> Result<Self, EspError> {
            let mut patterns = [adc_digi_pattern_config_t::default(); 32];

            for (index, (channel, atten)) in channels.iter().enumerate() {
                if index >= patterns.len() {
                    return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>());
                }

                patterns[index].atten = atten as _;
                patterns[index].channel = channel as _;
                patterns[index].unit = A::unit() as _;
                patterns[index].bit_width = 12; // For now only 12 bits is supported
            }

            let mut handle: adc_continuous_handle_t = core::ptr::null_mut();

            #[allow(clippy::needless_update)]
            esp!(unsafe {
                adc_continuous_new_handle(
                    &adc_continuous_handle_cfg_t {
                        max_store_buf_size: SOC_ADC_DIGI_DATA_BYTES_PER_CONV
                            * (config.frame_measurements as u32)
                            * (config.frames_count as u32),
                        conv_frame_size: SOC_ADC_DIGI_DATA_BYTES_PER_CONV
                            * (config.frame_measurements as u32),
                        ..Default::default()
                    },
                    &mut handle,
                )
            })?;

            let conv_mode = if A::unit() == 0 {
                adc_digi_convert_mode_t_ADC_CONV_SINGLE_UNIT_1
            } else {
                adc_digi_convert_mode_t_ADC_CONV_SINGLE_UNIT_2
            };

            #[cfg(any(esp32, esp32s2))]
            let format = adc_digi_output_format_t_ADC_DIGI_OUTPUT_FORMAT_TYPE1;

            #[cfg(not(any(esp32, esp32s2)))]
            let format = adc_digi_output_format_t_ADC_DIGI_OUTPUT_FORMAT_TYPE2;

            esp!(unsafe {
                adc_continuous_config(
                    handle,
                    &adc_continuous_config_t {
                        pattern_num: channels.iter().count() as _,
                        adc_pattern: &patterns as *const _ as *mut _,
                        sample_freq_hz: config.sample_freq.into(),
                        conv_mode,
                        format,
                    },
                )
            })?;

            #[cfg(not(esp_idf_adc_continuous_isr_iram_safe))]
            {
                esp!(unsafe {
                    adc_continuous_register_event_callbacks(
                        handle,
                        &adc_continuous_evt_cbs_t {
                            on_conv_done: Some(Self::handle_isr),
                            on_pool_ovf: Some(Self::handle_isr),
                        },
                        &NOTIFIER[A::unit() as usize] as *const _ as *mut _,
                    )
                })?;
            }

            Ok(Self {
                handle,
                adc: A::unit() as _,
                _ref: PhantomData,
            })
        }

        /// Get the ADC driver handle.
        pub fn handle(&self) -> adc_continuous_handle_t {
            self.handle
        }

        // Get the ADC unit. ADC1 or ADC2.
        pub fn unit(&self) -> adc_unit_t {
            self.adc as _
        }

        /// Start the ADC under continuous mode and generate results.
        pub fn start(&mut self) -> Result<(), EspError> {
            esp!(unsafe { adc_continuous_start(self.handle) })
        }

        /// Stop the ADC.
        pub fn stop(&mut self) -> Result<(), EspError> {
            esp!(unsafe { adc_continuous_stop(self.handle) })
        }

        /// Read `AdcMeasurements` in continuous mode.
        pub fn read(
            &mut self,
            buf: &mut [AdcMeasurement],
            timeout: TickType_t,
        ) -> Result<usize, EspError> {
            let mut read: u32 = 0;

            esp!(unsafe {
                adc_continuous_read(
                    self.handle,
                    buf.as_mut_ptr() as *mut _,
                    core::mem::size_of_val(buf) as _,
                    &mut read,
                    TickType(timeout).as_millis_u32(),
                )
            })?;

            Ok(read as usize / core::mem::size_of::<AdcMeasurement>())
        }

        /// Read bytes in continuous mode.
        pub fn read_bytes(
            &mut self,
            buf: &mut [u8],
            timeout: TickType_t,
        ) -> Result<usize, EspError> {
            let mut read: u32 = 0;

            esp!(unsafe {
                adc_continuous_read(
                    self.handle,
                    buf.as_mut_ptr() as *mut _,
                    core::mem::size_of_val(buf) as _,
                    &mut read,
                    TickType(timeout).as_millis_u32(),
                )
            })?;

            Ok(read as usize)
        }

        #[cfg(not(esp_idf_adc_continuous_isr_iram_safe))]
        pub async fn read_async(&mut self, buf: &mut [AdcMeasurement]) -> Result<usize, EspError> {
            loop {
                match self.read(buf, delay::NON_BLOCK) {
                    Ok(len) if len > 0 => return Ok(len),
                    Err(e) if e.code() != ESP_ERR_TIMEOUT => return Err(e),
                    _ => {
                        NOTIFIER[self.adc as usize].wait().await;
                    }
                }
            }
        }

        #[cfg(not(esp_idf_adc_continuous_isr_iram_safe))]
        pub async fn read_bytes_async(&mut self, buf: &mut [u8]) -> Result<usize, EspError> {
            loop {
                match self.read_bytes(buf, delay::NON_BLOCK) {
                    Ok(len) if len > 0 => return Ok(len),
                    Err(e) if e.code() != ESP_ERR_TIMEOUT => return Err(e),
                    _ => {
                        NOTIFIER[self.adc as usize].wait().await;
                    }
                }
            }
        }

        #[cfg(not(esp_idf_adc_continuous_isr_iram_safe))]
        extern "C" fn handle_isr(
            _handle: adc_continuous_handle_t,
            _data: *const adc_continuous_evt_data_t,
            user_data: *mut c_void,
        ) -> bool {
            let notifier: &HalIsrNotification =
                unsafe { (user_data as *const HalIsrNotification).as_ref() }.unwrap();

            notifier.notify_lsb()
        }
    }

    impl Drop for AdcDriver<'_> {
        fn drop(&mut self) {
            let _ = self.stop();

            #[cfg(not(esp_idf_adc_continuous_isr_iram_safe))]
            {
                esp!(unsafe {
                    adc_continuous_register_event_callbacks(
                        self.handle,
                        &adc_continuous_evt_cbs_t {
                            on_conv_done: None,
                            on_pool_ovf: None,
                        },
                        core::ptr::null_mut(),
                    )
                })
                .unwrap();
            }

            esp!(unsafe { adc_continuous_deinit(self.handle) }).unwrap();

            #[cfg(not(esp_idf_adc_continuous_isr_iram_safe))]
            NOTIFIER[self.adc as usize].reset();
        }
    }

    unsafe impl Send for AdcDriver<'_> {}

    impl embedded_io::ErrorType for AdcDriver<'_> {
        type Error = EspIOError;
    }

    impl embedded_io::Read for AdcDriver<'_> {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            self.read_bytes(buf, delay::BLOCK).map_err(EspIOError)
        }
    }

    #[cfg(not(esp_idf_adc_continuous_isr_iram_safe))]
    impl embedded_io_async::Read for AdcDriver<'_> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            self.read_bytes_async(buf).await.map_err(EspIOError)
        }
    }

    #[cfg(not(esp_idf_adc_continuous_isr_iram_safe))]
    #[cfg(any(
        esp32c2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6, esp32c61, esp32p4
    ))] // TODO: Check for esp32c5 and esp32p4
    static NOTIFIER: [HalIsrNotification; 1] = [HalIsrNotification::new()];

    #[cfg(not(esp_idf_adc_continuous_isr_iram_safe))]
    #[cfg(not(any(
        esp32c2, esp32h2, esp32h21, esp32h4, esp32c5, esp32c6, esp32c61, esp32p4
    )))] // TODO: Check for esp32c5 and esp32p4
    static NOTIFIER: [HalIsrNotification; 2] =
        [HalIsrNotification::new(), HalIsrNotification::new()];
}
