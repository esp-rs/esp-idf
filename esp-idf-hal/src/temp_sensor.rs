use esp_idf_sys::{
    esp, soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_DEFAULT,
    temperature_sensor_clk_src_t, temperature_sensor_config_t, temperature_sensor_disable,
    temperature_sensor_enable, temperature_sensor_get_celsius, temperature_sensor_handle_t,
    temperature_sensor_install, temperature_sensor_uninstall, EspError,
};

#[cfg(any(esp32p4, esp32s31))]
use esp_idf_sys::soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_LP_PERI;
#[cfg(any(
    esp32c2, esp32c3, esp32c5, esp32c6, esp32c61, esp32h2, esp32s2, esp32s3
))]
use esp_idf_sys::soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_RC_FAST;
#[cfg(any(esp32c2, esp32c3, esp32c5, esp32c6, esp32c61, esp32h2))]
use esp_idf_sys::soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_XTAL;

use core::marker::PhantomData;

// -- TempSensorClockSource --

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
/// Rust translation of `temperature_sensor_clk_src_t`
pub enum TempSensorClockSource {
    Default,
    #[cfg(any(
        esp32c2, esp32c3, esp32c5, esp32c6, esp32c61, esp32h2, esp32s2, esp32s3
    ))]
    RcFast,
    #[cfg(any(esp32c2, esp32c3, esp32c5, esp32c6, esp32c61, esp32h2))]
    XTAL,
    #[cfg(any(esp32p4, esp32s31))]
    LpPeri,
}

impl From<TempSensorClockSource> for temperature_sensor_clk_src_t {
    fn from(value: TempSensorClockSource) -> Self {
        match value {
            #[cfg(any(
                esp32c2, esp32c3, esp32c5, esp32c6, esp32c61, esp32h2, esp32s2, esp32s3
            ))]
            TempSensorClockSource::RcFast => {
                soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_RC_FAST
            }
            #[cfg(any(esp32c2, esp32c3, esp32c5, esp32c6, esp32c61, esp32h2))]
            TempSensorClockSource::XTAL => {
                soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_XTAL
            }
            #[cfg(any(esp32p4, esp32s31))]
            TempSensorClockSource::LpPeri => {
                soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_LP_PERI
            }
            TempSensorClockSource::Default => {
                soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_DEFAULT
            }
        }
    }
}

impl From<temperature_sensor_clk_src_t> for TempSensorClockSource {
    fn from(value: temperature_sensor_clk_src_t) -> Self {
        match value {
            #[cfg(any(
                esp32c2, esp32c3, esp32c5, esp32c6, esp32c61, esp32h2, esp32s2, esp32s3
            ))]
            #[allow(non_upper_case_globals)]
            soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_RC_FAST => {
                Self::RcFast
            }
            #[cfg(any(esp32c2, esp32c3, esp32c5, esp32c6, esp32c61, esp32h2))]
            #[allow(non_upper_case_globals)]
            soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_XTAL => Self::XTAL,
            #[cfg(any(esp32p4, esp32s31))]
            #[allow(non_upper_case_globals)]
            soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_LP_PERI => {
                Self::LpPeri
            }
            // TODO: Perhaps the default value should be mapped explicitly
            // and all other (u32) values should cause a failure
            _ => Self::Default,
        }
    }
}

// -- TempSensorConfig --
pub type TempSensorConfig = config::Config;
pub mod config {
    use super::*;
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    /// Rust wrapper for `temperature_sensor_config_t`
    pub struct Config {
        // TODO: check int size
        pub range_min: i32,
        pub range_max: i32,
        pub clk_src: TempSensorClockSource,
    }

    impl From<temperature_sensor_config_t> for Config {
        fn from(value: temperature_sensor_config_t) -> Self {
            Config {
                range_min: value.range_min,
                range_max: value.range_max,
                clk_src: value.clk_src.into(),
            }
        }
    }

    #[allow(clippy::needless_update)]
    impl From<&Config> for temperature_sensor_config_t {
        fn from(value: &Config) -> Self {
            temperature_sensor_config_t {
                clk_src: value.clk_src.into(),
                range_max: value.range_max,
                range_min: value.range_min,
                ..Default::default()
            }
        }
    }

    impl Default for Config {
        fn default() -> Self {
            Config::new()
        }
    }

    impl Config {
        pub const fn new() -> Self {
            Self {
                range_min: -10,
                range_max: 80,
                clk_src: TempSensorClockSource::Default,
            }
        }
    }
}

// -- TempSensorDriver --

pub struct TempSensorDriver<'d> {
    ptr: temperature_sensor_handle_t,
    _p: PhantomData<&'d mut ()>,
}

impl<'d> TempSensorDriver<'d> {
    pub fn new(config: &TempSensorConfig, _sensor: TempSensor<'d>) -> Result<Self, EspError> {
        let mut sensor = core::ptr::null_mut();
        esp!(unsafe { temperature_sensor_install(&config.into(), &mut sensor) })?;
        Ok(TempSensorDriver {
            ptr: sensor,
            _p: PhantomData,
        })
    }

    pub fn enable(&mut self) -> Result<(), EspError> {
        esp!(unsafe { temperature_sensor_enable(self.ptr) })
    }

    pub fn disable(&mut self) -> Result<(), EspError> {
        esp!(unsafe { temperature_sensor_disable(self.ptr) })
    }

    pub fn get_celsius(&self) -> Result<f32, EspError> {
        let mut val = 0.0;
        esp!(unsafe { temperature_sensor_get_celsius(self.ptr, &mut val) })?;
        Ok(val)
    }

    pub fn get_fahrenheit(&self) -> Result<f32, EspError> {
        let celsius = self.get_celsius()?;
        Ok((celsius * 1.8) + 32.0)
    }

    pub fn get_kelvin(&self) -> Result<f32, EspError> {
        let celsius = self.get_celsius()?;
        Ok(celsius + 273.15)
    }
}

impl Drop for TempSensorDriver<'_> {
    fn drop(&mut self) {
        let _ = self.disable();
        esp!(unsafe { temperature_sensor_uninstall(self.ptr) }).unwrap();
    }
}

unsafe impl Send for TempSensorDriver<'_> {}

crate::impl_peripheral!(TempSensor);
