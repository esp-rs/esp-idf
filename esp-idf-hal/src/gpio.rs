//! GPIO and pin configuration

use core::marker::PhantomData;

#[cfg(feature = "alloc")]
extern crate alloc;

use esp_idf_sys::*;

#[cfg(not(esp32s31))]
use crate::adc::AdcChannel;

pub use chip::*;

pub type PinId = u8;

/// A trait implemented by every pin instance
pub trait Pin: Sized + Send {
    /// Return the pin ID
    fn pin(&self) -> PinId;
}

/// A marker trait designating a pin which is capable of
/// operating as an input pin
pub trait InputPin: Pin {}

/// A marker trait designating a pin which is capable of
/// operating as an output pin
pub trait OutputPin: Pin {}

/// A marker trait designating a pin which is capable of
/// operating as an RTC pin
pub trait RTCPin: Pin {
    /// Return the RTC pin ID
    fn rtc_pin(&self) -> PinId;
}

/// A marker trait designating a pin which is capable of
/// operating as an ADC pin
#[cfg(not(esp32s31))]
pub trait ADCPin: Pin {
    /// Return the ADC channel for this pin
    type AdcChannel: AdcChannel;
}

/// A marker trait designating a pin which is capable of
/// operating as a DAC pin
#[cfg(any(esp32, esp32s2))]
pub trait DacChannel: 'static {
    /// Return the DAC channel for this pin
    fn dac_channel(&self) -> dac_channel_t;
}

#[cfg(any(esp32, esp32s2))]
pub struct DACCH<const N: dac_channel_t>;

#[cfg(any(esp32, esp32s2))]
impl<const N: dac_channel_t> DacChannel for DACCH<N> {
    fn dac_channel(&self) -> dac_channel_t {
        N
    }
}

/// A marker trait designating a pin which is capable of
/// operating as a DAC pin
#[cfg(any(esp32, esp32s2))]
pub trait DACPin: Pin {
    /// Return the DAC channel for this pin
    type DacChannel: DacChannel;
}

/// A marker trait designating a pin which is capable of
/// operating as a touch pin
#[cfg(any(esp32, esp32s2, esp32s3))]
pub trait TouchChannel: 'static {
    /// Return the touch channel for this pin
    fn touch_channel(&self) -> touch_pad_t;
}

#[cfg(any(esp32, esp32s2, esp32s3))]
pub struct TOUCHCH<const N: touch_pad_t>;

#[cfg(any(esp32, esp32s2, esp32s3))]
impl<const N: touch_pad_t> TouchChannel for TOUCHCH<N> {
    fn touch_channel(&self) -> touch_pad_t {
        N
    }
}

/// A marker trait designating a pin which is capable of
/// operating as a touch pin
#[cfg(any(esp32, esp32s2, esp32s3))]
pub trait TouchPin: Pin {
    /// Return the touch channel for this pin
    type TouchChannel: TouchChannel;
}

#[allow(unused_macros)]
macro_rules! impl_any {
    ($name:ident) => {
        pub struct $name<'a> {
            pin: PinId,
            _t: ::core::marker::PhantomData<&'a mut ()>,
        }

        impl $name<'_> {
            /// Unsafely create an instance of this peripheral out of thin air.
            ///
            /// # Safety
            ///
            /// You must ensure that you're only using one instance of this type at a time.
            #[inline(always)]
            pub unsafe fn steal(pin: PinId) -> Self {
                Self {
                    pin,
                    _t: ::core::marker::PhantomData,
                }
            }

            /// Creates a new peripheral reference with a shorter lifetime.
            ///
            /// Use this method if you would like to keep working with the peripheral after
            /// you dropped the driver that consumes this.
            ///
            /// # Safety
            ///
            /// You must ensure that you are not using reborrowed peripherals in drivers which are
            /// forgotten via `core::mem::forget`.
            #[inline]
            #[allow(dead_code)]
            pub unsafe fn reborrow(&mut self) -> $name<'_> {
                $name {
                    pin: self.pin,
                    _t: ::core::marker::PhantomData,
                }
            }

            /// Return `Option::None` for an unconfigured instance of that pin
            pub const fn none() -> Option<Self> {
                None
            }
        }

        unsafe impl Send for $name<'_> {}

        impl Pin for $name<'_> {
            fn pin(&self) -> PinId {
                self.pin as _
            }
        }
    };
}

impl_any!(AnyIOPin);

impl InputPin for AnyIOPin<'_> {}
impl OutputPin for AnyIOPin<'_> {}

impl_any!(AnyInputPin);

impl InputPin for AnyInputPin<'_> {}

impl<'a> From<AnyIOPin<'a>> for AnyInputPin<'a> {
    fn from(pin: AnyIOPin<'a>) -> Self {
        unsafe { Self::steal(pin.pin()) }
    }
}

impl_any!(AnyOutputPin);

impl OutputPin for AnyOutputPin<'_> {}

impl<'a> From<AnyIOPin<'a>> for AnyOutputPin<'a> {
    fn from(pin: AnyIOPin<'a>) -> Self {
        unsafe { Self::steal(pin.pin()) }
    }
}

/// Interrupt types
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum InterruptType {
    PosEdge,
    NegEdge,
    AnyEdge,
    LowLevel,
    HighLevel,
}

impl From<InterruptType> for gpio_int_type_t {
    fn from(interrupt_type: InterruptType) -> gpio_int_type_t {
        match interrupt_type {
            InterruptType::PosEdge => gpio_int_type_t_GPIO_INTR_POSEDGE,
            InterruptType::NegEdge => gpio_int_type_t_GPIO_INTR_NEGEDGE,
            InterruptType::AnyEdge => gpio_int_type_t_GPIO_INTR_ANYEDGE,
            InterruptType::LowLevel => gpio_int_type_t_GPIO_INTR_LOW_LEVEL,
            InterruptType::HighLevel => gpio_int_type_t_GPIO_INTR_HIGH_LEVEL,
        }
    }
}

impl From<InterruptType> for u8 {
    fn from(interrupt_type: InterruptType) -> u8 {
        let int_type: gpio_int_type_t = interrupt_type.into();

        int_type as u8
    }
}

/// Drive strength (values are approximates)
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum DriveStrength {
    I5mA = 0,
    I10mA = 1,
    I20mA = 2,
    I40mA = 3,
}

impl From<DriveStrength> for gpio_drive_cap_t {
    fn from(strength: DriveStrength) -> gpio_drive_cap_t {
        match strength {
            DriveStrength::I5mA => gpio_drive_cap_t_GPIO_DRIVE_CAP_0,
            DriveStrength::I10mA => gpio_drive_cap_t_GPIO_DRIVE_CAP_1,
            DriveStrength::I20mA => gpio_drive_cap_t_GPIO_DRIVE_CAP_2,
            DriveStrength::I40mA => gpio_drive_cap_t_GPIO_DRIVE_CAP_3,
        }
    }
}

impl From<gpio_drive_cap_t> for DriveStrength {
    #[allow(non_upper_case_globals)]
    fn from(cap: gpio_drive_cap_t) -> DriveStrength {
        match cap {
            gpio_drive_cap_t_GPIO_DRIVE_CAP_0 => DriveStrength::I5mA,
            gpio_drive_cap_t_GPIO_DRIVE_CAP_1 => DriveStrength::I10mA,
            gpio_drive_cap_t_GPIO_DRIVE_CAP_2 => DriveStrength::I20mA,
            gpio_drive_cap_t_GPIO_DRIVE_CAP_3 => DriveStrength::I40mA,
            other => panic!("Unknown GPIO pin drive capability: {other}"),
        }
    }
}

// Pull setting for an input.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum Pull {
    Floating,
    Up,
    Down,
    UpDown,
}

impl From<Pull> for gpio_pull_mode_t {
    fn from(pull: Pull) -> gpio_pull_mode_t {
        match pull {
            Pull::Floating => gpio_pull_mode_t_GPIO_FLOATING,
            Pull::Up => gpio_pull_mode_t_GPIO_PULLUP_ONLY,
            Pull::Down => gpio_pull_mode_t_GPIO_PULLDOWN_ONLY,
            Pull::UpDown => gpio_pull_mode_t_GPIO_PULLUP_PULLDOWN,
        }
    }
}

/// Digital input or output level.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum Level {
    Low,
    High,
}

impl From<bool> for Level {
    fn from(val: bool) -> Self {
        match val {
            true => Self::High,
            false => Self::Low,
        }
    }
}

impl From<Level> for bool {
    fn from(val: Level) -> bool {
        match val {
            Level::Low => false,
            Level::High => true,
        }
    }
}

impl core::ops::Not for Level {
    type Output = Level;

    fn not(self) -> Self::Output {
        match self {
            Level::Low => Level::High,
            Level::High => Level::Low,
        }
    }
}

impl From<embedded_hal_0_2::digital::v2::PinState> for Level {
    fn from(state: embedded_hal_0_2::digital::v2::PinState) -> Self {
        match state {
            embedded_hal_0_2::digital::v2::PinState::Low => Self::Low,
            embedded_hal_0_2::digital::v2::PinState::High => Self::High,
        }
    }
}

impl From<Level> for embedded_hal_0_2::digital::v2::PinState {
    fn from(level: Level) -> Self {
        match level {
            Level::Low => Self::Low,
            Level::High => Self::High,
        }
    }
}

impl From<embedded_hal::digital::PinState> for Level {
    fn from(state: embedded_hal::digital::PinState) -> Self {
        match state {
            embedded_hal::digital::PinState::Low => Self::Low,
            embedded_hal::digital::PinState::High => Self::High,
        }
    }
}

impl From<Level> for embedded_hal::digital::PinState {
    fn from(level: Level) -> Self {
        match level {
            Level::Low => Self::Low,
            Level::High => Self::High,
        }
    }
}

pub trait GPIOMode {}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
pub trait RTCMode {}

pub trait InputMode {
    const RTC: bool;
}

pub trait OutputMode {
    const RTC: bool;
}

pub struct Disabled;
pub struct Input;
pub struct Output;
pub struct InputOutput;
#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
pub struct RtcDisabled;
#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
pub struct RtcInput;
#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
pub struct RtcOutput;
#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
pub struct RtcInputOutput;

impl GPIOMode for Disabled {}

impl GPIOMode for Input {}

impl InputMode for Input {
    const RTC: bool = false;
}

impl GPIOMode for InputOutput {}

impl InputMode for InputOutput {
    const RTC: bool = false;
}

impl OutputMode for InputOutput {
    const RTC: bool = false;
}

impl GPIOMode for Output {}

impl OutputMode for Output {
    const RTC: bool = false;
}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl RTCMode for RtcDisabled {}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl RTCMode for RtcInput {}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl InputMode for RtcInput {
    const RTC: bool = true;
}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl RTCMode for RtcInputOutput {}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl InputMode for RtcInputOutput {
    const RTC: bool = true;
}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl OutputMode for RtcInputOutput {
    const RTC: bool = true;
}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl RTCMode for RtcOutput {}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl OutputMode for RtcOutput {
    const RTC: bool = true;
}

/// A driver for a GPIO pin.
///
/// The driver can set the pin as a disconnected/disabled one, input, or output pin, or both or analog.
/// On some chips (i.e. esp32 and esp32s*), the driver can also set the pin in RTC IO mode.
/// Depending on the current operating mode, different sets of functions are available.
///
/// The mode-setting depends on the capabilities of the pin as well, i.e. input-only pins cannot be set
/// into output or input-output mode.
pub struct PinDriver<'d, MODE> {
    pin: PinId,
    _mode: PhantomData<MODE>,
    _t: PhantomData<&'d mut ()>,
}

impl<'d, MODE> PinDriver<'d, MODE> {
    /// Try to convert the pin driver into a disabled pin driver.
    ///
    /// Return an error if the pin cannot be disabled.
    #[inline]
    pub fn try_into_disabled(self) -> Result<PinDriver<'d, Disabled>, EspError> {
        PinDriver::new_gpio(self.pin as _, gpio_mode_t_GPIO_MODE_DISABLE)
    }

    /// Try to convert the pin driver into an input pin driver.
    ///
    /// Return an error if the pin cannot be set as input.
    #[inline]
    pub fn try_into_input(self, pull: Pull) -> Result<PinDriver<'d, Input>, EspError> {
        let mut pin = PinDriver::new_gpio(self.pin as _, gpio_mode_t_GPIO_MODE_INPUT)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }

    /// Try to convert the pin driver into an output pin driver.
    ///
    /// Return an error if the pin cannot be set as output.
    #[inline]
    pub fn try_into_output(self) -> Result<PinDriver<'d, Output>, EspError> {
        PinDriver::new_gpio(self.pin as _, gpio_mode_t_GPIO_MODE_OUTPUT)
    }

    /// Try to convert the pin driver into an input-output pin driver.
    ///
    /// Return an error if the pin cannot be set as input-output.
    #[inline]
    pub fn try_into_input_output(self, pull: Pull) -> Result<PinDriver<'d, InputOutput>, EspError> {
        let mut pin = PinDriver::new_gpio(self.pin as _, gpio_mode_t_GPIO_MODE_INPUT_OUTPUT)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }

    /// Try to convert the pin driver into an output open-drain pin driver.
    ///
    /// Return an error if the pin cannot be set as output open-drain.
    #[inline]
    pub fn try_into_output_od(self) -> Result<PinDriver<'d, Output>, EspError> {
        PinDriver::new_gpio(self.pin as _, gpio_mode_t_GPIO_MODE_OUTPUT_OD)
    }

    /// Try to convert the pin driver into an input-output open-drain pin driver.
    ///
    /// Return an error if the pin cannot be set as input-output open-drain.
    #[inline]
    pub fn try_into_input_output_od(
        self,
        pull: Pull,
    ) -> Result<PinDriver<'d, InputOutput>, EspError> {
        let mut pin = PinDriver::new_gpio(self.pin as _, gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }

    /// Try to convert the pin driver into an RTC disabled pin driver.
    ///
    /// Return an error if the pin cannot be disabled.
    #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
    #[inline]
    pub fn try_into_rtc_disabled(self) -> Result<PinDriver<'d, RtcDisabled>, EspError> {
        PinDriver::new_rtc(self.pin as _, rtc_gpio_mode_t_RTC_GPIO_MODE_DISABLED)
    }

    /// Try to convert the pin driver into an RTC input pin driver.
    ///
    /// Return an error if the pin cannot be set as RTC input.
    #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
    #[inline]
    pub fn try_into_rtc_input(self, pull: Pull) -> Result<PinDriver<'d, RtcInput>, EspError> {
        let mut pin = PinDriver::new_rtc(self.pin as _, rtc_gpio_mode_t_RTC_GPIO_MODE_INPUT_ONLY)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }

    /// Try to convert the pin driver into an RTC output pin driver.
    ///
    /// Return an error if the pin cannot be set as RTC output.
    #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
    #[inline]
    pub fn try_into_rtc_output(self) -> Result<PinDriver<'d, RtcOutput>, EspError> {
        PinDriver::new_rtc(self.pin as _, rtc_gpio_mode_t_RTC_GPIO_MODE_OUTPUT_ONLY)
    }

    /// Try to convert the pin driver into an RTC output open-drain pin driver.
    ///
    /// Return an error if the pin cannot be set as RTC output open-drain.
    #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
    #[inline]
    pub fn try_into_rtc_output_od(self) -> Result<PinDriver<'d, RtcOutput>, EspError> {
        PinDriver::new_rtc(self.pin as _, rtc_gpio_mode_t_RTC_GPIO_MODE_OUTPUT_OD)
    }

    /// Try to convert the pin driver into an RTC input-output pin driver.
    ///
    /// Return an error if the pin cannot be set as RTC input-output.
    #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
    #[inline]
    pub fn try_into_rtc_input_output(
        self,
        pull: Pull,
    ) -> Result<PinDriver<'d, RtcInputOutput>, EspError> {
        let mut pin =
            PinDriver::new_rtc(self.pin as _, rtc_gpio_mode_t_RTC_GPIO_MODE_INPUT_OUTPUT)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }

    /// Try to convert the pin driver into an RTC input-output open-drain pin driver.
    ///
    /// Return an error if the pin cannot be set as RTC input-output open-drain.
    #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
    #[inline]
    pub fn try_into_rtc_input_output_od(
        self,
        pull: Pull,
    ) -> Result<PinDriver<'d, RtcInputOutput>, EspError> {
        let mut pin =
            PinDriver::new_rtc(self.pin as _, rtc_gpio_mode_t_RTC_GPIO_MODE_INPUT_OUTPUT_OD)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }
}

impl<'d> PinDriver<'d, Disabled> {
    /// Creates the driver for a pin in disabled state.
    #[inline]
    pub fn disabled<T: Pin + 'd>(pin: T) -> Result<Self, EspError> {
        Self::new_gpio(pin.pin(), gpio_mode_t_GPIO_MODE_DISABLE)
    }
}

impl<'d> PinDriver<'d, Input> {
    /// Creates the driver for a pin in input state.
    #[inline]
    pub fn input<T: InputPin + 'd>(pin: T, pull: Pull) -> Result<Self, EspError> {
        let mut pin = Self::new_gpio(pin.pin(), gpio_mode_t_GPIO_MODE_INPUT)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }
}

impl<'d> PinDriver<'d, InputOutput> {
    /// Creates the driver for a pin in input-output state.
    #[inline]
    pub fn input_output<T: InputPin + OutputPin + 'd>(
        pin: T,
        pull: Pull,
    ) -> Result<Self, EspError> {
        let mut pin = Self::new_gpio(pin.pin(), gpio_mode_t_GPIO_MODE_INPUT_OUTPUT)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }

    /// Creates the driver for a pin in input-output open-drain state.
    #[inline]
    pub fn input_output_od<T: InputPin + OutputPin + 'd>(
        pin: T,
        pull: Pull,
    ) -> Result<Self, EspError> {
        let mut pin = Self::new_gpio(pin.pin(), gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }
}

impl<'d> PinDriver<'d, Output> {
    /// Creates the driver for a pin in output state.
    #[inline]
    pub fn output<T: OutputPin + 'd>(pin: T) -> Result<Self, EspError> {
        Self::new_gpio(pin.pin(), gpio_mode_t_GPIO_MODE_OUTPUT)
    }

    /// Creates the driver for a pin in output open-drain state.
    #[inline]
    pub fn output_od<T: OutputPin + 'd>(pin: T) -> Result<Self, EspError> {
        Self::new_gpio(pin.pin(), gpio_mode_t_GPIO_MODE_OUTPUT_OD)
    }
}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl<'d> PinDriver<'d, RtcDisabled> {
    /// Creates the driver for a pin in disabled state.
    #[inline]
    pub fn rtc_disabled<T: Pin + RTCPin + 'd>(pin: T) -> Result<Self, EspError> {
        Self::new_rtc(pin.pin(), rtc_gpio_mode_t_RTC_GPIO_MODE_DISABLED)
    }
}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl<'d> PinDriver<'d, RtcInput> {
    /// Creates the driver for a pin in RTC input state.
    #[inline]
    pub fn rtc_input<T: InputPin + RTCPin + 'd>(pin: T, pull: Pull) -> Result<Self, EspError> {
        let mut pin = Self::new_rtc(pin.pin(), rtc_gpio_mode_t_RTC_GPIO_MODE_INPUT_ONLY)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }
}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl<'d> PinDriver<'d, RtcInputOutput> {
    /// Creates the driver for a pin in RTC input-output state.
    #[inline]
    pub fn rtc_input_output<T: InputPin + OutputPin + RTCPin + 'd>(
        pin: T,
        pull: Pull,
    ) -> Result<Self, EspError> {
        let mut pin = Self::new_rtc(pin.pin(), rtc_gpio_mode_t_RTC_GPIO_MODE_INPUT_OUTPUT)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }

    /// Creates the driver for a pin in RTC input-output open-drain state.
    #[inline]
    pub fn rtc_input_output_od<T: InputPin + OutputPin + RTCPin + 'd>(
        pin: T,
        pull: Pull,
    ) -> Result<Self, EspError> {
        let mut pin = Self::new_rtc(pin.pin(), rtc_gpio_mode_t_RTC_GPIO_MODE_INPUT_OUTPUT_OD)?;

        pin.set_pull(pull)?;

        Ok(pin)
    }
}

#[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
impl<'d> PinDriver<'d, RtcOutput> {
    /// Creates the driver for a pin in RTC output state.
    #[inline]
    pub fn rtc_output<T: OutputPin + RTCPin + 'd>(pin: T) -> Result<Self, EspError> {
        Self::new_rtc(pin.pin(), rtc_gpio_mode_t_RTC_GPIO_MODE_OUTPUT_ONLY)
    }

    /// Creates the driver for a pin in RTC output open-drain state.
    #[inline]
    pub fn rtc_output_od<T: OutputPin + RTCPin + 'd>(pin: T) -> Result<Self, EspError> {
        Self::new_rtc(pin.pin(), rtc_gpio_mode_t_RTC_GPIO_MODE_OUTPUT_OD)
    }
}

impl<'d, MODE> PinDriver<'d, MODE> {
    /// Returns the pin number.
    pub fn pin(&self) -> PinId {
        self.pin
    }

    #[inline]
    pub fn get_drive_strength(&self) -> Result<DriveStrength, EspError>
    where
        MODE: OutputMode,
    {
        let mut cap: gpio_drive_cap_t = 0;

        if MODE::RTC {
            #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
            esp!(unsafe { rtc_gpio_get_drive_capability(self.pin as _, &mut cap) })?;

            #[cfg(any(esp32c3, esp32c2, esp32h2, esp32h4))]
            unreachable!();
        } else {
            esp!(unsafe { gpio_get_drive_capability(self.pin as _, &mut cap) })?;
        }

        Ok(cap.into())
    }

    #[inline]
    pub fn set_drive_strength(&mut self, strength: DriveStrength) -> Result<(), EspError>
    where
        MODE: OutputMode,
    {
        if MODE::RTC {
            #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
            esp!(unsafe { rtc_gpio_set_drive_capability(self.pin as _, strength.into()) })?;

            #[cfg(any(esp32c3, esp32c2, esp32h2, esp32h4))]
            unreachable!();
        } else {
            esp!(unsafe { gpio_set_drive_capability(self.pin as _, strength.into()) })?;
        }

        Ok(())
    }

    #[inline]
    pub fn is_high(&self) -> bool
    where
        MODE: InputMode,
    {
        self.get_level().into()
    }

    #[inline]
    pub fn is_low(&self) -> bool
    where
        MODE: InputMode,
    {
        !self.is_high()
    }

    #[inline]
    #[allow(clippy::needless_late_init)]
    pub fn get_level(&self) -> Level
    where
        MODE: InputMode,
    {
        let res;

        if MODE::RTC {
            #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
            {
                res = if unsafe { rtc_gpio_get_level(self.pin as _) } != 0 {
                    Level::High
                } else {
                    Level::Low
                };
            }

            #[cfg(any(esp32c3, esp32c2, esp32h2, esp32h4))]
            unreachable!();
        } else if unsafe { gpio_get_level(self.pin as _) } != 0 {
            res = Level::High;
        } else {
            res = Level::Low;
        }

        res
    }

    #[inline]
    pub fn is_set_high(&self) -> bool
    where
        MODE: OutputMode,
    {
        !self.is_set_low()
    }

    /// Is the output pin set as low?
    #[inline]
    pub fn is_set_low(&self) -> bool
    where
        MODE: OutputMode,
    {
        self.get_output_level() == Level::Low
    }

    /// What level output is set to
    #[inline]
    fn get_output_level(&self) -> Level
    where
        MODE: OutputMode,
    {
        // TODO: Implement for RTC mode

        let pin = self.pin as u32;

        #[cfg(any(esp32c3, esp32c2, esp32h2, esp32h4))]
        let is_set_high = unsafe { (*(GPIO_OUT_REG as *const u32) >> pin) & 0x01 != 0 };
        #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
        let is_set_high = if pin <= 31 {
            // GPIO0 - GPIO31
            unsafe { (*(GPIO_OUT_REG as *const u32) >> pin) & 0x01 != 0 }
        } else {
            // GPIO32+
            unsafe { (*(GPIO_OUT1_REG as *const u32) >> (pin - 32)) & 0x01 != 0 }
        };

        if is_set_high {
            Level::High
        } else {
            Level::Low
        }
    }

    #[inline]
    pub fn set_high(&mut self) -> Result<(), EspError>
    where
        MODE: OutputMode,
    {
        self.set_level(Level::High)
    }

    /// Set the output as low.
    #[inline]
    pub fn set_low(&mut self) -> Result<(), EspError>
    where
        MODE: OutputMode,
    {
        self.set_level(Level::Low)
    }

    #[inline]
    pub fn set_level(&mut self, level: Level) -> Result<(), EspError>
    where
        MODE: OutputMode,
    {
        let on = match level {
            Level::Low => 0,
            Level::High => 1,
        };

        if MODE::RTC {
            #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
            esp!(unsafe { rtc_gpio_set_level(self.pin as _, on) })?;

            #[cfg(any(esp32c3, esp32c2, esp32h2, esp32h4))]
            unreachable!();
        } else {
            esp!(unsafe { gpio_set_level(self.pin as _, on) })?;
        }

        Ok(())
    }

    /// Toggle pin output
    #[inline]
    pub fn toggle(&mut self) -> Result<(), EspError>
    where
        MODE: OutputMode,
    {
        if self.is_set_low() {
            self.set_high()
        } else {
            self.set_low()
        }
    }

    fn set_pull(&mut self, pull: Pull) -> Result<(), EspError>
    where
        MODE: InputMode,
    {
        if MODE::RTC {
            #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
            unsafe {
                match pull {
                    Pull::Down => {
                        esp!(rtc_gpio_pulldown_en(self.pin as _))?;
                        esp!(rtc_gpio_pullup_dis(self.pin as _))?;
                    }
                    Pull::Up => {
                        esp!(rtc_gpio_pulldown_dis(self.pin as _))?;
                        esp!(rtc_gpio_pullup_en(self.pin as _))?;
                    }
                    Pull::UpDown => {
                        esp!(rtc_gpio_pulldown_en(self.pin as _))?;
                        esp!(rtc_gpio_pullup_en(self.pin as _))?;
                    }
                    Pull::Floating => {
                        esp!(rtc_gpio_pulldown_dis(self.pin as _))?;
                        esp!(rtc_gpio_pullup_dis(self.pin as _))?;
                    }
                }
            }

            #[cfg(any(esp32c3, esp32c2, esp32h2, esp32h4))]
            unreachable!();
        } else {
            esp!(unsafe { gpio_set_pull_mode(self.pin as _, pull.into()) })?;
        }

        Ok(())
    }

    /// Subscribes the provided callback for ISR notifications.
    /// As a side effect, interrupts will be disabled, so to receive a notification, one has
    /// to also call `PinDriver::enable_interrupt` after calling this method.
    ///
    /// Note that `PinDriver::enable_interrupt` should also be called after
    /// each received notification **from non-ISR context**, because the driver will automatically
    /// disable ISR interrupts on each received ISR notification (so as to avoid IWDT triggers).
    ///
    /// # Safety
    ///
    /// Care should be taken not to call STD, libc or FreeRTOS APIs (except for a few allowed ones)
    /// in the callback passed to this function, as it is executed in an ISR context.
    #[cfg(feature = "alloc")]
    pub unsafe fn subscribe<F: FnMut() + Send + 'static>(
        &mut self,
        callback: F,
    ) -> Result<(), EspError>
    where
        MODE: InputMode,
    {
        self.internal_subscribe(callback)
    }

    /// Subscribes the provided callback for ISR notifications.
    /// As a side effect, interrupts will be disabled, so to receive a notification, one has
    /// to also call `PinDriver::enable_interrupt` after calling this method.
    ///
    /// Note that `PinDriver::enable_interrupt` should also be called after
    /// each received notification **from non-ISR context**, because the driver will automatically
    /// disable ISR interrupts on each received ISR notification (so as to avoid IWDT triggers).
    ///
    /// # Safety
    ///
    /// Care should be taken not to call STD, libc or FreeRTOS APIs (except for a few allowed ones)
    /// in the callback passed to this function, as it is executed in an ISR context.
    ///
    /// Additionally, this method - in contrast to method `subscribe` - allows
    /// the passed-in callback/closure to be non-`'static`. This enables users to borrow
    /// - in the closure - variables that live on the stack - or more generally - in the same
    ///   scope where the driver is created.
    ///
    /// HOWEVER: care should be taken NOT to call `core::mem::forget()` on the driver,
    /// as that would immediately lead to an UB (crash).
    /// Also note that forgetting the driver might happen with `Rc` and `Arc`
    /// when circular references are introduced: https://github.com/rust-lang/rust/issues/24456
    ///
    /// The reason is that the closure is actually sent and owned by an ISR routine,
    /// which means that if the driver is forgotten, Rust is free to e.g. unwind the stack
    /// and the ISR routine will end up with references to variables that no longer exist.
    ///
    /// The destructor of the driver takes care - prior to the driver being dropped and e.g.
    /// the stack being unwind - to unsubscribe the ISR routine.
    /// Unfortunately, when the driver is forgotten, the un-subscription does not happen
    /// and invalid references are left dangling.
    ///
    /// This "local borrowing" will only be possible to express in a safe way once/if `!Leak` types
    /// are introduced to Rust (i.e. the impossibility to "forget" a type and thus not call its destructor).
    #[cfg(feature = "alloc")]
    pub unsafe fn subscribe_nonstatic<F: FnMut() + Send + 'd>(
        &mut self,
        callback: F,
    ) -> Result<(), EspError>
    where
        MODE: InputMode,
    {
        self.internal_subscribe(callback)
    }

    #[cfg(feature = "alloc")]
    fn internal_subscribe(&mut self, callback: impl FnMut() + Send + 'd) -> Result<(), EspError>
    where
        MODE: InputMode,
    {
        extern crate alloc;

        self.disable_interrupt()?;

        let callback: alloc::boxed::Box<dyn FnMut() + Send + 'd> = alloc::boxed::Box::new(callback);
        unsafe {
            chip::PIN_ISR_HANDLER[self.pin as usize] = Some(core::mem::transmute::<
                alloc::boxed::Box<dyn FnMut() + Send>,
                alloc::boxed::Box<dyn FnMut() + Send>,
            >(callback));
        }

        Ok(())
    }

    pub fn unsubscribe(&mut self) -> Result<(), EspError>
    where
        MODE: InputMode,
    {
        unsafe {
            unsubscribe_pin(self.pin as _)?;
        }

        Ok(())
    }

    /// Enables or re-enables the interrupt
    ///
    /// Note that the interrupt is automatically disabled each time an interrupt is triggered
    /// (or else we risk entering a constant interrupt processing loop while the pin is in low/high state
    /// and the interrupt type is set to non-edge)
    ///
    /// Therefore - to continue receiving ISR interrupts - user needs to call `enable_interrupt`
    /// - **from a non-ISR context** - after each successful interrupt triggering.
    pub fn enable_interrupt(&mut self) -> Result<(), EspError>
    where
        MODE: InputMode,
    {
        enable_isr_service()?;

        unsafe {
            esp!(gpio_isr_handler_add(
                self.pin as _,
                Some(Self::handle_isr),
                self.pin as u32 as *mut core::ffi::c_void,
            ))
        }
    }

    pub fn disable_interrupt(&mut self) -> Result<(), EspError>
    where
        MODE: InputMode,
    {
        use core::sync::atomic::Ordering;

        if ISR_SERVICE_ENABLED.load(Ordering::SeqCst) {
            esp!(unsafe { gpio_isr_handler_remove(self.pin as _) })?;
        }

        Ok(())
    }

    pub fn set_interrupt_type(&mut self, interrupt_type: InterruptType) -> Result<(), EspError>
    where
        MODE: InputMode,
    {
        esp!(unsafe { gpio_set_intr_type(self.pin as _, interrupt_type.into()) })?;

        Ok(())
    }

    #[inline]
    fn new_gpio(pin: PinId, mode: gpio_mode_t) -> Result<Self, EspError>
    where
        MODE: GPIOMode,
    {
        if mode != gpio_mode_t_GPIO_MODE_DISABLE {
            // `gpio_set_direction` wires up the GPIO matrix and the output
            // enable, but it never touches `IO_MUX.MCU_SEL`. A pad that boots
            // on an alternate function - JTAG, SPI flash, UART - therefore
            // stays on that function and never sees what the driver writes.
            // Route it to the GPIO function first, which is what `gpio_config`
            // does for every pin it is handed.
            unsafe { esp_rom_gpio_pad_select_gpio(pin as _) };

            esp!(unsafe { gpio_set_direction(pin as _, mode) })?;
        }

        Ok(Self {
            pin,
            _mode: PhantomData,
            _t: PhantomData,
        })
    }

    #[inline]
    #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
    fn new_rtc(pin: PinId, mode: rtc_gpio_mode_t) -> Result<Self, EspError>
    where
        MODE: RTCMode,
    {
        esp!(unsafe { rtc_gpio_init(pin as _) })?;
        esp!(unsafe { rtc_gpio_set_direction(pin as _, mode) })?;

        Ok(PinDriver {
            pin,
            _mode: PhantomData,
            _t: PhantomData,
        })
    }

    unsafe extern "C" fn handle_isr(user_ctx: *mut core::ffi::c_void) {
        let pin = user_ctx as u32;

        // IMPORTANT: MUST be done or else the ESP IDF GPIO driver will continue calling us in a loop
        // - particularly when the interrupt type is set to non-edge triggering (pin high or low) -
        // which will eventually cause the Interrupt WatchDog to kick in
        gpio_intr_disable(pin as _);

        PIN_NOTIF[pin as usize].notify_lsb();

        #[cfg(feature = "alloc")]
        {
            if let Some(unsafe_callback) = unsafe { &mut PIN_ISR_HANDLER[pin as usize] } {
                (unsafe_callback)();
            }
        }
    }
}

impl<MODE: InputMode> PinDriver<'_, MODE> {
    pub async fn wait_for(&mut self, interrupt_type: InterruptType) -> Result<(), EspError> {
        self.disable_interrupt()?;

        let notif = &chip::PIN_NOTIF[self.pin as usize];

        notif.reset();

        match interrupt_type {
            InterruptType::LowLevel if self.is_low() => return Ok(()),
            InterruptType::HighLevel if self.is_high() => return Ok(()),
            _ => (),
        }

        self.set_interrupt_type(interrupt_type)?;
        self.enable_interrupt()?;

        notif.wait().await;

        Ok(())
    }

    pub async fn wait_for_high(&mut self) -> Result<(), EspError> {
        self.wait_for(InterruptType::HighLevel).await
    }

    pub async fn wait_for_low(&mut self) -> Result<(), EspError> {
        self.wait_for(InterruptType::LowLevel).await
    }

    pub async fn wait_for_rising_edge(&mut self) -> Result<(), EspError> {
        self.wait_for(InterruptType::PosEdge).await
    }

    pub async fn wait_for_falling_edge(&mut self) -> Result<(), EspError> {
        self.wait_for(InterruptType::NegEdge).await
    }

    pub async fn wait_for_any_edge(&mut self) -> Result<(), EspError> {
        self.wait_for(InterruptType::AnyEdge).await
    }
}

impl<MODE> Drop for PinDriver<'_, MODE> {
    fn drop(&mut self) {
        gpio_reset_without_pull(self.pin as _).unwrap();
    }
}

unsafe impl<MODE> Send for PinDriver<'_, MODE> {}

impl<MODE> embedded_hal_0_2::digital::v2::InputPin for PinDriver<'_, MODE>
where
    MODE: InputMode,
{
    type Error = EspError;

    fn is_high(&self) -> Result<bool, Self::Error> {
        Ok(PinDriver::is_high(self))
    }

    fn is_low(&self) -> Result<bool, Self::Error> {
        Ok(PinDriver::is_low(self))
    }
}

use crate::embedded_hal_error;
embedded_hal_error!(
    GpioError,
    embedded_hal::digital::Error,
    embedded_hal::digital::ErrorKind
);

fn to_gpio_err(err: EspError) -> GpioError {
    GpioError::other(err)
}

impl<MODE> embedded_hal::digital::ErrorType for PinDriver<'_, MODE> {
    type Error = GpioError;
}

impl<MODE> embedded_hal::digital::InputPin for PinDriver<'_, MODE>
where
    MODE: InputMode,
{
    fn is_high(&mut self) -> Result<bool, Self::Error> {
        Ok(PinDriver::is_high(self))
    }

    fn is_low(&mut self) -> Result<bool, Self::Error> {
        Ok(PinDriver::is_low(self))
    }
}

impl<MODE> embedded_hal::digital::InputPin for &PinDriver<'_, MODE>
where
    MODE: InputMode,
{
    fn is_high(&mut self) -> Result<bool, Self::Error> {
        Ok(PinDriver::is_high(self))
    }

    fn is_low(&mut self) -> Result<bool, Self::Error> {
        Ok(PinDriver::is_low(self))
    }
}

impl<MODE> embedded_hal_0_2::digital::v2::OutputPin for PinDriver<'_, MODE>
where
    MODE: OutputMode,
{
    type Error = EspError;

    fn set_high(&mut self) -> Result<(), Self::Error> {
        self.set_level(Level::High)
    }

    fn set_low(&mut self) -> Result<(), Self::Error> {
        self.set_level(Level::Low)
    }
}

impl<MODE> embedded_hal::digital::OutputPin for PinDriver<'_, MODE>
where
    MODE: OutputMode,
{
    fn set_high(&mut self) -> Result<(), Self::Error> {
        self.set_level(Level::High).map_err(to_gpio_err)
    }

    fn set_low(&mut self) -> Result<(), Self::Error> {
        self.set_level(Level::Low).map_err(to_gpio_err)
    }
}

impl<MODE> embedded_hal::digital::StatefulOutputPin for PinDriver<'_, MODE>
where
    MODE: OutputMode,
{
    fn is_set_high(&mut self) -> Result<bool, Self::Error> {
        Ok(self.get_output_level().into())
    }

    fn is_set_low(&mut self) -> Result<bool, Self::Error> {
        Ok(!bool::from(self.get_output_level()))
    }
}

// TODO: Will become possible once the `PinDriver::setXXX`` methods become non-`&mut`, which they really are, internally
// impl<'d, T: Pin, MODE> embedded_hal::digital::StatefulOutputPin for &PinDriver<'d, T, MODE>
// where
//     MODE: OutputMode,
// {
//     fn is_set_high(&mut self) -> Result<bool, Self::Error> {
//         Ok(self.get_output_level().into())
//     }

//     fn is_set_low(&mut self) -> Result<bool, Self::Error> {
//         Ok(!bool::from(self.get_output_level()))
//     }
// }

impl<MODE> embedded_hal_0_2::digital::v2::StatefulOutputPin for PinDriver<'_, MODE>
where
    MODE: OutputMode,
{
    fn is_set_high(&self) -> Result<bool, Self::Error> {
        Ok(self.get_output_level().into())
    }

    fn is_set_low(&self) -> Result<bool, Self::Error> {
        Ok(!bool::from(self.get_output_level()))
    }
}

impl<MODE> embedded_hal_0_2::digital::v2::ToggleableOutputPin for PinDriver<'_, MODE>
where
    MODE: OutputMode,
{
    type Error = EspError;

    fn toggle(&mut self) -> Result<(), Self::Error> {
        self.set_level(Level::from(!bool::from(self.get_output_level())))
    }
}

impl<MODE: InputMode> embedded_hal_async::digital::Wait for PinDriver<'_, MODE> {
    async fn wait_for_high(&mut self) -> Result<(), GpioError> {
        self.wait_for_high().await?;

        Ok(())
    }

    async fn wait_for_low(&mut self) -> Result<(), GpioError> {
        self.wait_for_low().await?;

        Ok(())
    }

    async fn wait_for_rising_edge(&mut self) -> Result<(), GpioError> {
        self.wait_for_rising_edge().await?;

        Ok(())
    }

    async fn wait_for_falling_edge(&mut self) -> Result<(), GpioError> {
        self.wait_for_falling_edge().await?;

        Ok(())
    }

    async fn wait_for_any_edge(&mut self) -> Result<(), GpioError> {
        self.wait_for_any_edge().await?;

        Ok(())
    }
}

static ISR_ALLOC_FLAGS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

static ISR_SERVICE_ENABLED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

static ISR_SERVICE_ENABLED_CS: crate::task::CriticalSection = crate::task::CriticalSection::new();

pub fn init_isr_alloc_flags(flags: enumset::EnumSet<crate::interrupt::InterruptType>) {
    ISR_ALLOC_FLAGS.store(
        crate::interrupt::InterruptType::to_native(flags),
        core::sync::atomic::Ordering::SeqCst,
    );
}

pub fn enable_isr_service() -> Result<(), EspError> {
    use core::sync::atomic::Ordering;

    if !ISR_SERVICE_ENABLED.load(Ordering::SeqCst) {
        let _guard = ISR_SERVICE_ENABLED_CS.enter();

        if !ISR_SERVICE_ENABLED.load(Ordering::SeqCst) {
            esp!(unsafe { gpio_install_isr_service(ISR_ALLOC_FLAGS.load(Ordering::SeqCst) as _) })?;

            ISR_SERVICE_ENABLED.store(true, Ordering::SeqCst);
        }
    }

    Ok(())
}

/// Notifies this GPIO driver that the GPIO ISR service has already been initialized.
/// This prevents an error as this driver would otherwise attempt to initialize it again.
///
/// # Safety
///
/// The caller must ensure that `gpio_install_isr_service` has already
/// been invoked elsewhere. This should only be ever needed when
/// interfacing external code that touches the GPIO ISR service.
pub unsafe fn set_isr_service_flag_unchecked() {
    ISR_SERVICE_ENABLED.store(true, core::sync::atomic::Ordering::SeqCst);
}

#[cfg(not(esp32s31))]
pub(crate) unsafe fn rtc_reset_pin(pin: i32) -> Result<(), EspError> {
    gpio_reset_without_pull(pin)?;

    #[cfg(not(any(esp32c3, esp32c2, esp32h2, esp32h4)))]
    if rtc_gpio_is_valid_gpio(pin) {
        esp!(rtc_gpio_init(pin))?;
    }

    Ok(())
}

// The default esp-idf gpio_reset function sets a pull-up. If that behaviour is
// not desired this function can be used instead.
#[inline]
fn gpio_reset_without_pull(pin: gpio_num_t) -> Result<(), EspError> {
    let cfg = gpio_config_t {
        pin_bit_mask: (1u64 << pin),
        mode: esp_idf_sys::gpio_mode_t_GPIO_MODE_DISABLE,
        pull_up_en: esp_idf_sys::gpio_pullup_t_GPIO_PULLUP_DISABLE,
        pull_down_en: esp_idf_sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
        intr_type: esp_idf_sys::gpio_int_type_t_GPIO_INTR_DISABLE,
        #[cfg(all(
            esp_idf_soc_gpio_support_pin_hys_filter,
            not(esp_idf_version_major = "4")
        ))]
        hys_ctrl_mode: esp_idf_sys::gpio_hys_ctrl_mode_t_GPIO_HYS_SOFT_DISABLE,
    };

    unsafe {
        unsubscribe_pin(pin)?;
        esp!(gpio_config(&cfg))?;
    }
    Ok(())
}

unsafe fn unsubscribe_pin(pin: gpio_num_t) -> Result<(), EspError> {
    use core::sync::atomic::Ordering;

    if ISR_SERVICE_ENABLED.load(Ordering::SeqCst) {
        esp!(gpio_isr_handler_remove(pin as _))?;

        chip::PIN_NOTIF[pin as usize].reset();

        #[cfg(feature = "alloc")]
        {
            chip::PIN_ISR_HANDLER[pin as usize] = None;
        }
    }

    Ok(())
}

#[cfg(feature = "alloc")]
#[allow(clippy::declare_interior_mutable_const)] // OK because this is only used as an array initializer
const PIN_ISR_INIT: Option<alloc::boxed::Box<dyn FnMut() + Send + 'static>> = None;

#[allow(clippy::declare_interior_mutable_const)] // OK because this is only used as an array initializer
const PIN_NOTIF_INIT: crate::interrupt::asynch::HalIsrNotification =
    crate::interrupt::asynch::HalIsrNotification::new();

macro_rules! impl_input {
    ($pxi:ident: $pin:expr) => {
        $crate::impl_peripheral!($pxi);

        impl $pxi<'static> {
            /// Return `Option::None` for an unconfigured instance of that pin
            pub const fn none() -> Option<Self> {
                None
            }
        }

        impl Pin for $pxi<'_> {
            fn pin(&self) -> PinId {
                $pin as _
            }
        }

        impl InputPin for $pxi<'_> {}

        impl<'a> From<$pxi<'a>> for AnyInputPin<'a> {
            fn from(pin: $pxi) -> Self {
                unsafe { Self::steal(pin.pin()) }
            }
        }

        impl<'a> $pxi<'a> {
            pub fn degrade_input(self) -> AnyInputPin<'a> {
                self.into()
            }
        }
    };
}

macro_rules! impl_input_output {
    ($pxi:ident: $pin:expr) => {
        impl_input!($pxi: $pin);

        impl<'a> $pxi<'a> {
            pub fn degrade_output(self) -> AnyOutputPin<'a> {
                self.into()
            }

            pub fn degrade_input_output(self) -> AnyIOPin<'a> {
                self.into()
            }
        }

        impl OutputPin for $pxi<'_> {}

        impl<'a> From<$pxi<'a>> for AnyOutputPin<'a> {
            fn from(pin: $pxi) -> Self {
                unsafe { Self::steal(pin.pin()) }
            }
        }

        impl<'a> From<$pxi<'a>> for AnyIOPin<'a> {
            fn from(pin: $pxi) -> Self {
                unsafe { Self::steal(pin.pin()) }
            }
        }
    };
}

macro_rules! impl_rtc {
    ($pxi:ident: $pin:expr, RTC: $rtc:expr) => {
        impl RTCPin for $pxi<'_> {
            fn rtc_pin(&self) -> PinId {
                $rtc
            }
        }
    };

    ($pxi:ident: $pin:expr, NORTC: $rtc:expr) => {};
}

macro_rules! impl_adc {
    ($pxi:ident: $pin:expr, ADC1: $channel:ident) => {
        impl ADCPin for $pxi<'_> {
            type AdcChannel = $crate::adc::$channel<$crate::adc::ADCU1>;
        }
    };

    ($pxi:ident: $pin:expr, ADC2: $channel:ident) => {
        impl ADCPin for $pxi<'_> {
            type AdcChannel = $crate::adc::$channel<$crate::adc::ADCU2>;
        }
    };

    ($pxi:ident: $pin:expr, NOADC: $channel:ident) => {};
}

macro_rules! impl_dac {
    ($pxi:ident: $pin:expr, DAC: $dac:expr) => {
        #[cfg(any(esp32, esp32s2))]
        impl DACPin for $pxi<'_> {
            type DacChannel = DACCH<$dac>;
        }
    };

    ($pxi:ident: $pin:expr, NODAC: $dac:expr) => {};
}

macro_rules! impl_touch {
    ($pxi:ident: $pin:expr, TOUCH: $touch:expr) => {
        #[cfg(any(esp32, esp32s2, esp32s3))]
        impl TouchPin for $pxi<'_> {
            type TouchChannel = TOUCHCH<$touch>;
        }
    };

    ($pxi:ident: $pin:expr, NOTOUCH: $touch:expr) => {};
}

macro_rules! pin {
    ($pxi:ident: $pin:expr, Input, $rtc:ident: $rtcno:expr, $adc:ident: $adcno:ident, $dac:ident: $dacno:expr, $touch:ident: $touchno:expr) => {
        impl_input!($pxi: $pin);
        impl_rtc!($pxi: $pin, $rtc: $rtcno);
        impl_adc!($pxi: $pin, $adc: $adcno);
        impl_dac!($pxi: $pin, $dac: $dacno);
        impl_touch!($pxi: $pin, $touch: $touchno);
    };

    ($pxi:ident: $pin:expr, IO, $rtc:ident: $rtcno:expr, $adc:ident: $adcno:ident, $dac:ident: $dacno:expr, $touch:ident: $touchno:expr) => {
        impl_input_output!($pxi: $pin);
        impl_rtc!($pxi: $pin, $rtc: $rtcno);
        impl_adc!($pxi: $pin, $adc: $adcno);
        impl_dac!($pxi: $pin, $dac: $dacno);
        impl_touch!($pxi: $pin, $touch: $touchno);
    };
}

#[cfg(esp32)]
mod chip {
    #[cfg(feature = "alloc")]
    extern crate alloc;

    #[cfg(feature = "alloc")]
    use alloc::boxed::Box;

    use crate::interrupt::asynch::HalIsrNotification;

    use super::*;

    #[allow(clippy::type_complexity)]
    #[cfg(feature = "alloc")]
    pub(crate) static mut PIN_ISR_HANDLER: [Option<Box<dyn FnMut() + Send + 'static>>; 40] =
        [PIN_ISR_INIT; 40];

    #[allow(clippy::type_complexity)]
    pub(crate) static PIN_NOTIF: [HalIsrNotification; 40] = [PIN_NOTIF_INIT; 40];

    // NOTE: Gpio26 - Gpio32 are used by SPI0/SPI1 for external PSRAM/SPI Flash and
    //       are not recommended for other uses
    pin!(Gpio0:0, IO, RTC:11, ADC2:ADCCH1, NODAC:0, TOUCH:1);
    pin!(Gpio1:1, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio2:2, IO, RTC:12, ADC2:ADCCH2, NODAC:0, TOUCH:2);
    pin!(Gpio3:3, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio4:4, IO, RTC:10, ADC2:ADCCH0, NODAC:0, TOUCH:0);
    pin!(Gpio5:5, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio6:6, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio7:7, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio8:8, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio9:9, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio10:10, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio11:11, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio12:12, IO, RTC:15, ADC2:ADCCH5, NODAC:0, TOUCH:5);
    pin!(Gpio13:13, IO, RTC:14, ADC2:ADCCH4, NODAC:0, TOUCH:4);
    pin!(Gpio14:14, IO, RTC:16, ADC2:ADCCH6, NODAC:0, TOUCH:6);
    pin!(Gpio15:15, IO, RTC:13, ADC2:ADCCH3, NODAC:0, TOUCH:3);
    pin!(Gpio16:16, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio17:17, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio18:18, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio19:19, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio20:20, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio21:21, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio22:22, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio23:23, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio25:25, IO, RTC:6, ADC2:ADCCH8, DAC:1, NOTOUCH:0);
    pin!(Gpio26:26, IO, RTC:7, ADC2:ADCCH9, DAC:2, NOTOUCH:0);
    pin!(Gpio27:27, IO, RTC:17, ADC2:ADCCH7, NODAC:0, TOUCH:7);
    pin!(Gpio32:32, IO, RTC:9, ADC1:ADCCH4, NODAC:0, TOUCH:9);
    pin!(Gpio33:33, IO, RTC:8, ADC1:ADCCH5, NODAC:0, TOUCH:8);
    pin!(Gpio34:34, Input, RTC:4, ADC1:ADCCH6, NODAC:0, NOTOUCH:0);
    pin!(Gpio35:35, Input, RTC:5, ADC1:ADCCH7, NODAC:0, NOTOUCH:0);
    pin!(Gpio36:36, Input, RTC:0, ADC1:ADCCH0, NODAC:0, NOTOUCH:0);
    pin!(Gpio37:37, Input, RTC:1, ADC1:ADCCH1, NODAC:0, NOTOUCH:0);
    pin!(Gpio38:38, Input, RTC:2, ADC1:ADCCH2, NODAC:0, NOTOUCH:0);
    pin!(Gpio39:39, Input, RTC:3, ADC1:ADCCH3, NODAC:0, NOTOUCH:0);

    pub struct Pins {
        pub gpio0: Gpio0<'static>,
        pub gpio1: Gpio1<'static>,
        pub gpio2: Gpio2<'static>,
        pub gpio3: Gpio3<'static>,
        pub gpio4: Gpio4<'static>,
        pub gpio5: Gpio5<'static>,
        pub gpio6: Gpio6<'static>,
        pub gpio7: Gpio7<'static>,
        pub gpio8: Gpio8<'static>,
        pub gpio9: Gpio9<'static>,
        pub gpio10: Gpio10<'static>,
        pub gpio11: Gpio11<'static>,
        pub gpio12: Gpio12<'static>,
        pub gpio13: Gpio13<'static>,
        pub gpio14: Gpio14<'static>,
        pub gpio15: Gpio15<'static>,
        pub gpio16: Gpio16<'static>,
        pub gpio17: Gpio17<'static>,
        pub gpio18: Gpio18<'static>,
        pub gpio19: Gpio19<'static>,
        pub gpio20: Gpio20<'static>,
        pub gpio21: Gpio21<'static>,
        pub gpio22: Gpio22<'static>,
        pub gpio23: Gpio23<'static>,
        pub gpio25: Gpio25<'static>,
        pub gpio26: Gpio26<'static>,
        pub gpio27: Gpio27<'static>,
        pub gpio32: Gpio32<'static>,
        pub gpio33: Gpio33<'static>,
        pub gpio34: Gpio34<'static>,
        pub gpio35: Gpio35<'static>,
        pub gpio36: Gpio36<'static>,
        pub gpio37: Gpio37<'static>,
        pub gpio38: Gpio38<'static>,
        pub gpio39: Gpio39<'static>,
    }

    impl Pins {
        /// # Safety
        ///
        /// Care should be taken not to instantiate the Pins structure, if it is
        /// already instantiated and used elsewhere
        pub unsafe fn new() -> Self {
            Self {
                gpio0: Gpio0::steal(),
                gpio1: Gpio1::steal(),
                gpio2: Gpio2::steal(),
                gpio3: Gpio3::steal(),
                gpio4: Gpio4::steal(),
                gpio5: Gpio5::steal(),
                gpio6: Gpio6::steal(),
                gpio7: Gpio7::steal(),
                gpio8: Gpio8::steal(),
                gpio9: Gpio9::steal(),
                gpio10: Gpio10::steal(),
                gpio11: Gpio11::steal(),
                gpio12: Gpio12::steal(),
                gpio13: Gpio13::steal(),
                gpio14: Gpio14::steal(),
                gpio15: Gpio15::steal(),
                gpio16: Gpio16::steal(),
                gpio17: Gpio17::steal(),
                gpio18: Gpio18::steal(),
                gpio19: Gpio19::steal(),
                gpio20: Gpio20::steal(),
                gpio21: Gpio21::steal(),
                gpio22: Gpio22::steal(),
                gpio23: Gpio23::steal(),
                gpio25: Gpio25::steal(),
                gpio26: Gpio26::steal(),
                gpio27: Gpio27::steal(),
                gpio32: Gpio32::steal(),
                gpio33: Gpio33::steal(),
                gpio34: Gpio34::steal(),
                gpio35: Gpio35::steal(),
                gpio36: Gpio36::steal(),
                gpio37: Gpio37::steal(),
                gpio38: Gpio38::steal(),
                gpio39: Gpio39::steal(),
            }
        }
    }
}

#[cfg(any(esp32s2, esp32s3))]
mod chip {
    #[cfg(feature = "alloc")]
    extern crate alloc;

    #[cfg(feature = "alloc")]
    use alloc::boxed::Box;

    use crate::interrupt::asynch::HalIsrNotification;

    use super::*;

    #[allow(clippy::type_complexity)]
    #[cfg(feature = "alloc")]
    pub(crate) static mut PIN_ISR_HANDLER: [Option<Box<dyn FnMut() + Send + 'static>>; 49] =
        [PIN_ISR_INIT; 49];

    #[allow(clippy::type_complexity)]
    pub(crate) static PIN_NOTIF: [HalIsrNotification; 49] = [PIN_NOTIF_INIT; 49];

    // NOTE: Gpio26 - Gpio32 (and Gpio33 - Gpio37 if using Octal RAM/Flash) are used
    //       by SPI0/SPI1 for external PSRAM/SPI Flash and are not recommended for
    //       other uses
    pin!(Gpio0:0, IO, RTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio1:1, IO, RTC:1, ADC1:ADCCH0, NODAC:0, TOUCH:1);
    pin!(Gpio2:2, IO, RTC:2, ADC1:ADCCH1, NODAC:0, TOUCH:2);
    pin!(Gpio3:3, IO, RTC:3, ADC1:ADCCH2, NODAC:0, TOUCH:3);
    pin!(Gpio4:4, IO, RTC:4, ADC1:ADCCH3, NODAC:0, TOUCH:4);
    pin!(Gpio5:5, IO, RTC:5, ADC1:ADCCH4, NODAC:0, TOUCH:5);
    pin!(Gpio6:6, IO, RTC:6, ADC1:ADCCH5, NODAC:0, TOUCH:6);
    pin!(Gpio7:7, IO, RTC:7, ADC1:ADCCH6, NODAC:0, TOUCH:7);
    pin!(Gpio8:8, IO, RTC:8, ADC1:ADCCH7, NODAC:0, TOUCH:8);
    pin!(Gpio9:9, IO, RTC:9, ADC1:ADCCH8, NODAC:0, TOUCH:9);
    pin!(Gpio10:10, IO, RTC:10, ADC1:ADCCH9, NODAC:0, TOUCH:10);
    pin!(Gpio11:11, IO, RTC:11, ADC2:ADCCH0, NODAC:0, TOUCH:11);
    pin!(Gpio12:12, IO, RTC:12, ADC2:ADCCH1, NODAC:0, TOUCH:12);
    pin!(Gpio13:13, IO, RTC:13, ADC2:ADCCH2, NODAC:0, TOUCH:13);
    pin!(Gpio14:14, IO, RTC:14, ADC2:ADCCH3, NODAC:0, TOUCH:14);
    pin!(Gpio15:15, IO, RTC:15, ADC2:ADCCH4, NODAC:0, NOTOUCH:0);
    pin!(Gpio16:16, IO, RTC:16, ADC2:ADCCH5, NODAC:0, NOTOUCH:0);
    #[cfg(esp32s2)]
    pin!(Gpio17:17, IO, RTC:17, ADC2:ADCCH6, DAC:1, NOTOUCH:0);
    #[cfg(esp32s3)]
    pin!(Gpio17:17, IO, RTC:17, ADC2:ADCCH6, NODAC:0, NOTOUCH:0);
    #[cfg(esp32s2)]
    pin!(Gpio18:18, IO, RTC:18, ADC2:ADCCH7, DAC:2, NOTOUCH:0);
    #[cfg(esp32s3)]
    pin!(Gpio18:18, IO, RTC:18, ADC2:ADCCH7, NODAC:0, NOTOUCH:0);
    pin!(Gpio19:19, IO, RTC:19, ADC2:ADCCH8, NODAC:0, NOTOUCH:0);
    pin!(Gpio20:20, IO, RTC:20, ADC2:ADCCH9, NODAC:0, NOTOUCH:0);
    pin!(Gpio21:21, IO, RTC:21, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio26:26, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio27:27, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio28:28, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio29:29, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio30:30, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio31:31, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio32:32, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio33:33, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio34:34, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio35:35, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio36:36, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio37:37, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio38:38, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio39:39, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio40:40, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio41:41, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio42:42, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio43:43, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio44:44, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio45:45, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    #[cfg(esp32s2)]
    pin!(Gpio46:46, Input, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    #[cfg(esp32s3)]
    pin!(Gpio46:46, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    #[cfg(esp32s3)]
    pin!(Gpio47:47, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    #[cfg(esp32s3)]
    pin!(Gpio48:48, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pub struct Pins {
        pub gpio0: Gpio0<'static>,
        pub gpio1: Gpio1<'static>,
        pub gpio2: Gpio2<'static>,
        pub gpio3: Gpio3<'static>,
        pub gpio4: Gpio4<'static>,
        pub gpio5: Gpio5<'static>,
        pub gpio6: Gpio6<'static>,
        pub gpio7: Gpio7<'static>,
        pub gpio8: Gpio8<'static>,
        pub gpio9: Gpio9<'static>,
        pub gpio10: Gpio10<'static>,
        pub gpio11: Gpio11<'static>,
        pub gpio12: Gpio12<'static>,
        pub gpio13: Gpio13<'static>,
        pub gpio14: Gpio14<'static>,
        pub gpio15: Gpio15<'static>,
        pub gpio16: Gpio16<'static>,
        pub gpio17: Gpio17<'static>,
        pub gpio18: Gpio18<'static>,
        pub gpio19: Gpio19<'static>,
        pub gpio20: Gpio20<'static>,
        pub gpio21: Gpio21<'static>,
        pub gpio26: Gpio26<'static>,
        pub gpio27: Gpio27<'static>,
        pub gpio28: Gpio28<'static>,
        pub gpio29: Gpio29<'static>,
        pub gpio30: Gpio30<'static>,
        pub gpio31: Gpio31<'static>,
        pub gpio32: Gpio32<'static>,
        pub gpio33: Gpio33<'static>,
        pub gpio34: Gpio34<'static>,
        pub gpio35: Gpio35<'static>,
        pub gpio36: Gpio36<'static>,
        pub gpio37: Gpio37<'static>,
        pub gpio38: Gpio38<'static>,
        pub gpio39: Gpio39<'static>,
        pub gpio40: Gpio40<'static>,
        pub gpio41: Gpio41<'static>,
        pub gpio42: Gpio42<'static>,
        pub gpio43: Gpio43<'static>,
        pub gpio44: Gpio44<'static>,
        pub gpio45: Gpio45<'static>,
        pub gpio46: Gpio46<'static>,
        #[cfg(esp32s3)]
        pub gpio47: Gpio47<'static>,
        #[cfg(esp32s3)]
        pub gpio48: Gpio48<'static>,
    }

    impl Pins {
        /// # Safety
        ///
        /// Care should be taken not to instantiate the Pins structure, if it is
        /// already instantiated and used elsewhere
        pub unsafe fn new() -> Self {
            Self {
                gpio0: Gpio0::steal(),
                gpio1: Gpio1::steal(),
                gpio2: Gpio2::steal(),
                gpio3: Gpio3::steal(),
                gpio4: Gpio4::steal(),
                gpio5: Gpio5::steal(),
                gpio6: Gpio6::steal(),
                gpio7: Gpio7::steal(),
                gpio8: Gpio8::steal(),
                gpio9: Gpio9::steal(),
                gpio10: Gpio10::steal(),
                gpio11: Gpio11::steal(),
                gpio12: Gpio12::steal(),
                gpio13: Gpio13::steal(),
                gpio14: Gpio14::steal(),
                gpio15: Gpio15::steal(),
                gpio16: Gpio16::steal(),
                gpio17: Gpio17::steal(),
                gpio18: Gpio18::steal(),
                gpio19: Gpio19::steal(),
                gpio20: Gpio20::steal(),
                gpio21: Gpio21::steal(),
                gpio26: Gpio26::steal(),
                gpio27: Gpio27::steal(),
                gpio28: Gpio28::steal(),
                gpio29: Gpio29::steal(),
                gpio30: Gpio30::steal(),
                gpio31: Gpio31::steal(),
                gpio32: Gpio32::steal(),
                gpio33: Gpio33::steal(),
                gpio34: Gpio34::steal(),
                gpio35: Gpio35::steal(),
                gpio36: Gpio36::steal(),
                gpio37: Gpio37::steal(),
                gpio38: Gpio38::steal(),
                gpio39: Gpio39::steal(),
                gpio40: Gpio40::steal(),
                gpio41: Gpio41::steal(),
                gpio42: Gpio42::steal(),
                gpio43: Gpio43::steal(),
                gpio44: Gpio44::steal(),
                gpio45: Gpio45::steal(),
                gpio46: Gpio46::steal(),
                #[cfg(esp32s3)]
                gpio47: Gpio47::steal(),
                #[cfg(esp32s3)]
                gpio48: Gpio48::steal(),
            }
        }
    }
}

#[cfg(esp32c3)]
mod chip {
    #[cfg(feature = "alloc")]
    extern crate alloc;

    #[cfg(feature = "alloc")]
    use alloc::boxed::Box;

    use crate::interrupt::asynch::HalIsrNotification;

    use super::*;

    #[allow(clippy::type_complexity)]
    #[cfg(feature = "alloc")]
    pub(crate) static mut PIN_ISR_HANDLER: [Option<Box<dyn FnMut() + Send + 'static>>; 22] =
        [PIN_ISR_INIT; 22];

    pub(crate) static PIN_NOTIF: [HalIsrNotification; 22] = [PIN_NOTIF_INIT; 22];

    // NOTE: Gpio12 - Gpio17 are used by SPI0/SPI1 for external PSRAM/SPI Flash and
    //       are not recommended for other uses
    pin!(Gpio0:0,   IO,   RTC:0, ADC1:ADCCH0, NODAC:0, NOTOUCH:0);
    pin!(Gpio1:1,   IO,   RTC:1, ADC1:ADCCH1, NODAC:0, NOTOUCH:0);
    pin!(Gpio2:2,   IO,   RTC:2, ADC1:ADCCH2, NODAC:0, NOTOUCH:0);
    pin!(Gpio3:3,   IO,   RTC:3, ADC1:ADCCH3, NODAC:0, NOTOUCH:0);
    pin!(Gpio4:4,   IO,   RTC:4, ADC1:ADCCH4, NODAC:0, NOTOUCH:0);
    pin!(Gpio5:5,   IO,   RTC:5, ADC2:ADCCH0, NODAC:0, NOTOUCH:0);
    pin!(Gpio6:6,   IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio7:7,   IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio8:8,   IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio9:9,   IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio10:10, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio11:11, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio12:12, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio13:13, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio14:14, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio15:15, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio16:16, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio17:17, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio18:18, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio19:19, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio20:20, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio21:21, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pub struct Pins {
        pub gpio0: Gpio0<'static>,
        pub gpio1: Gpio1<'static>,
        pub gpio2: Gpio2<'static>,
        pub gpio3: Gpio3<'static>,
        pub gpio4: Gpio4<'static>,
        pub gpio5: Gpio5<'static>,
        pub gpio6: Gpio6<'static>,
        pub gpio7: Gpio7<'static>,
        pub gpio8: Gpio8<'static>,
        pub gpio9: Gpio9<'static>,
        pub gpio10: Gpio10<'static>,
        pub gpio11: Gpio11<'static>,
        pub gpio12: Gpio12<'static>,
        pub gpio13: Gpio13<'static>,
        pub gpio14: Gpio14<'static>,
        pub gpio15: Gpio15<'static>,
        pub gpio16: Gpio16<'static>,
        pub gpio17: Gpio17<'static>,
        pub gpio18: Gpio18<'static>,
        pub gpio19: Gpio19<'static>,
        pub gpio20: Gpio20<'static>,
        pub gpio21: Gpio21<'static>,
    }

    impl Pins {
        pub(crate) unsafe fn new() -> Self {
            Self {
                gpio0: Gpio0::steal(),
                gpio1: Gpio1::steal(),
                gpio2: Gpio2::steal(),
                gpio3: Gpio3::steal(),
                gpio4: Gpio4::steal(),
                gpio5: Gpio5::steal(),
                gpio6: Gpio6::steal(),
                gpio7: Gpio7::steal(),
                gpio8: Gpio8::steal(),
                gpio9: Gpio9::steal(),
                gpio10: Gpio10::steal(),
                gpio11: Gpio11::steal(),
                gpio12: Gpio12::steal(),
                gpio13: Gpio13::steal(),
                gpio14: Gpio14::steal(),
                gpio15: Gpio15::steal(),
                gpio16: Gpio16::steal(),
                gpio17: Gpio17::steal(),
                gpio18: Gpio18::steal(),
                gpio19: Gpio19::steal(),
                gpio20: Gpio20::steal(),
                gpio21: Gpio21::steal(),
            }
        }
    }
}

#[cfg(esp32c2)]
mod chip {
    #[cfg(feature = "alloc")]
    extern crate alloc;

    #[cfg(feature = "alloc")]
    use alloc::boxed::Box;

    use crate::interrupt::asynch::HalIsrNotification;

    use super::*;

    #[allow(clippy::type_complexity)]
    #[cfg(feature = "alloc")]
    pub(crate) static mut PIN_ISR_HANDLER: [Option<Box<dyn FnMut() + Send + 'static>>; 21] =
        [PIN_ISR_INIT; 21];

    pub(crate) static PIN_NOTIF: [HalIsrNotification; 21] = [PIN_NOTIF_INIT; 21];

    // NOTE: Gpio12 - Gpio17 are used by SPI0/SPI1 for external PSRAM/SPI Flash and
    //       are not recommended for other uses
    pin!(Gpio0:0,   IO,   RTC:0,  ADC1:ADCCH0, NODAC:0, NOTOUCH:0);
    pin!(Gpio1:1,   IO,   RTC:1,  ADC1:ADCCH1, NODAC:0, NOTOUCH:0);
    pin!(Gpio2:2,   IO,   RTC:2,  ADC1:ADCCH2, NODAC:0, NOTOUCH:0);
    pin!(Gpio3:3,   IO,   RTC:3,  ADC1:ADCCH3, NODAC:0, NOTOUCH:0);
    pin!(Gpio4:4,   IO,   RTC:4,  ADC1:ADCCH4, NODAC:0, NOTOUCH:0);
    pin!(Gpio5:5,   IO,   RTC:5, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio6:6,   IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio7:7,   IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio8:8,   IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio9:9,   IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio10:10, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio11:11, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio12:12, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio13:13, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio14:14, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio15:15, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio16:16, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio17:17, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio18:18, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio19:19, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio20:20, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pub struct Pins {
        pub gpio0: Gpio0<'static>,
        pub gpio1: Gpio1<'static>,
        pub gpio2: Gpio2<'static>,
        pub gpio3: Gpio3<'static>,
        pub gpio4: Gpio4<'static>,
        pub gpio5: Gpio5<'static>,
        pub gpio6: Gpio6<'static>,
        pub gpio7: Gpio7<'static>,
        pub gpio8: Gpio8<'static>,
        pub gpio9: Gpio9<'static>,
        pub gpio10: Gpio10<'static>,
        pub gpio11: Gpio11<'static>,
        pub gpio12: Gpio12<'static>,
        pub gpio13: Gpio13<'static>,
        pub gpio14: Gpio14<'static>,
        pub gpio15: Gpio15<'static>,
        pub gpio16: Gpio16<'static>,
        pub gpio17: Gpio17<'static>,
        pub gpio18: Gpio18<'static>,
        pub gpio19: Gpio19<'static>,
        pub gpio20: Gpio20<'static>,
    }

    impl Pins {
        /// # Safety
        ///
        /// Care should be taken not to instantiate the Pins structure, if it is
        /// already instantiated and used elsewhere
        pub unsafe fn new() -> Self {
            Self {
                gpio0: Gpio0::steal(),
                gpio1: Gpio1::steal(),
                gpio2: Gpio2::steal(),
                gpio3: Gpio3::steal(),
                gpio4: Gpio4::steal(),
                gpio5: Gpio5::steal(),
                gpio6: Gpio6::steal(),
                gpio7: Gpio7::steal(),
                gpio8: Gpio8::steal(),
                gpio9: Gpio9::steal(),
                gpio10: Gpio10::steal(),
                gpio11: Gpio11::steal(),
                gpio12: Gpio12::steal(),
                gpio13: Gpio13::steal(),
                gpio14: Gpio14::steal(),
                gpio15: Gpio15::steal(),
                gpio16: Gpio16::steal(),
                gpio17: Gpio17::steal(),
                gpio18: Gpio18::steal(),
                gpio19: Gpio19::steal(),
                gpio20: Gpio20::steal(),
            }
        }
    }
}

#[cfg(any(esp32h2, esp32h4))]
mod chip {
    #[cfg(feature = "alloc")]
    extern crate alloc;

    #[cfg(feature = "alloc")]
    use alloc::boxed::Box;

    use crate::interrupt::asynch::HalIsrNotification;

    use super::*;

    #[allow(clippy::type_complexity)]
    #[cfg(feature = "alloc")]
    pub(crate) static mut PIN_ISR_HANDLER: [Option<Box<dyn FnMut() + Send + 'static>>; 28] =
        [PIN_ISR_INIT; 28];

    pub(crate) static PIN_NOTIF: [HalIsrNotification; 28] = [PIN_NOTIF_INIT; 28];

    // NOTE: Following pins have special meaning and are not recommended for other uses. But one may use them with care.
    //  - Gpio12 - Gpio17 are used by SPI0/SPI1 for external PSRAM/SPI Flash
    //  - Gpio21 seems not to be exposed physically
    //  - Gpio23 + Gpio24 are used by serial debug interface
    //  - Gpio26 + Gpio27 are used by USB debug interface
    pin!(Gpio0:0,   IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio1:1,   IO, NORTC:0,  ADC1:ADCCH0, NODAC:0, NOTOUCH:0);
    pin!(Gpio2:2,   IO, NORTC:0,  ADC1:ADCCH1, NODAC:0, NOTOUCH:0);
    pin!(Gpio3:3,   IO, NORTC:0,  ADC1:ADCCH2, NODAC:0, NOTOUCH:0);
    pin!(Gpio4:4,   IO, NORTC:0,  ADC1:ADCCH3, NODAC:0, NOTOUCH:0);
    pin!(Gpio5:5,   IO, NORTC:0,  ADC1:ADCCH4, NODAC:0, NOTOUCH:0);
    pin!(Gpio6:6,   IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio7:7,   IO,   RTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio8:8,   IO,   RTC:1, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio9:9,   IO,   RTC:2, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio10:10, IO,   RTC:3, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio11:11, IO,   RTC:4, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio12:12, IO,   RTC:5, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio13:13, IO,   RTC:6, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio14:14, IO,   RTC:7, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio15:15, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio16:16, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio17:17, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio18:18, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio19:19, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio20:20, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio21:21, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio22:22, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio23:23, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio24:24, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio25:25, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio26:26, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio27:27, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pub struct Pins {
        pub gpio0: Gpio0<'static>,
        pub gpio1: Gpio1<'static>,
        pub gpio2: Gpio2<'static>,
        pub gpio3: Gpio3<'static>,
        pub gpio4: Gpio4<'static>,
        pub gpio5: Gpio5<'static>,
        pub gpio6: Gpio6<'static>,
        pub gpio7: Gpio7<'static>,
        pub gpio8: Gpio8<'static>,
        pub gpio9: Gpio9<'static>,
        pub gpio10: Gpio10<'static>,
        pub gpio11: Gpio11<'static>,
        pub gpio12: Gpio12<'static>,
        pub gpio13: Gpio13<'static>,
        pub gpio14: Gpio14<'static>,
        pub gpio15: Gpio15<'static>,
        pub gpio16: Gpio16<'static>,
        pub gpio17: Gpio17<'static>,
        pub gpio18: Gpio18<'static>,
        pub gpio19: Gpio19<'static>,
        pub gpio20: Gpio20<'static>,
        pub gpio21: Gpio21<'static>,
        pub gpio22: Gpio22<'static>,
        pub gpio23: Gpio23<'static>,
        pub gpio24: Gpio24<'static>,
        pub gpio25: Gpio25<'static>,
        pub gpio26: Gpio26<'static>,
        pub gpio27: Gpio27<'static>,
    }

    impl Pins {
        /// # Safety
        ///
        /// Care should be taken not to instantiate the Pins structure, if it is
        /// already instantiated and used elsewhere
        pub unsafe fn new() -> Self {
            Self {
                gpio0: Gpio0::steal(),
                gpio1: Gpio1::steal(),
                gpio2: Gpio2::steal(),
                gpio3: Gpio3::steal(),
                gpio4: Gpio4::steal(),
                gpio5: Gpio5::steal(),
                gpio6: Gpio6::steal(),
                gpio7: Gpio7::steal(),
                gpio8: Gpio8::steal(),
                gpio9: Gpio9::steal(),
                gpio10: Gpio10::steal(),
                gpio11: Gpio11::steal(),
                gpio12: Gpio12::steal(),
                gpio13: Gpio13::steal(),
                gpio14: Gpio14::steal(),
                gpio15: Gpio15::steal(),
                gpio16: Gpio16::steal(),
                gpio17: Gpio17::steal(),
                gpio18: Gpio18::steal(),
                gpio19: Gpio19::steal(),
                gpio20: Gpio20::steal(),
                gpio21: Gpio21::steal(),
                gpio22: Gpio22::steal(),
                gpio23: Gpio23::steal(),
                gpio24: Gpio24::steal(),
                gpio25: Gpio25::steal(),
                gpio26: Gpio26::steal(),
                gpio27: Gpio27::steal(),
            }
        }
    }
}

#[cfg(esp32c5)]
mod chip {
    #[cfg(feature = "alloc")]
    extern crate alloc;

    #[cfg(feature = "alloc")]
    use alloc::boxed::Box;

    use crate::interrupt::asynch::HalIsrNotification;

    use super::*;

    #[allow(clippy::type_complexity)]
    #[cfg(feature = "alloc")]
    pub(crate) static mut PIN_ISR_HANDLER: [Option<Box<dyn FnMut() + Send + 'static>>; 29] =
        [PIN_ISR_INIT; 29];

    #[allow(clippy::type_complexity)]
    pub(crate) static PIN_NOTIF: [HalIsrNotification; 29] = [PIN_NOTIF_INIT; 29];

    // NOTE: Gpio26 - Gpio32 (and Gpio33 - Gpio37 if using Octal RAM/Flash) are used
    //       by SPI0/SPI1 for external PSRAM/SPI Flash and are not recommended for
    //       other uses
    pin!(Gpio0:0, IO, RTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio1:1, IO, RTC:1, ADC1:ADCCH0, NODAC:0, NOTOUCH:0);
    pin!(Gpio2:2, IO, RTC:2, ADC1:ADCCH1, NODAC:0, NOTOUCH:0);
    pin!(Gpio3:3, IO, RTC:3, ADC1:ADCCH2, NODAC:0, NOTOUCH:0);
    pin!(Gpio4:4, IO, RTC:4, ADC1:ADCCH3, NODAC:0, NOTOUCH:0);
    pin!(Gpio5:5, IO, RTC:5, ADC1:ADCCH4, NODAC:0, NOTOUCH:0);
    pin!(Gpio6:6, IO, RTC:6, ADC1:ADCCH5, NODAC:0, NOTOUCH:0);
    pin!(Gpio7:7, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio8:8, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio9:9, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio10:10, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio11:11, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio12:12, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio13:13, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio14:14, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio15:15, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio16:16, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio17:17, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio18:18, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio19:19, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio20:20, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio21:21, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio22:22, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio23:23, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio24:24, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio25:25, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio26:26, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio27:27, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio28:28, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pub struct Pins {
        pub gpio0: Gpio0<'static>,
        pub gpio1: Gpio1<'static>,
        pub gpio2: Gpio2<'static>,
        pub gpio3: Gpio3<'static>,
        pub gpio4: Gpio4<'static>,
        pub gpio5: Gpio5<'static>,
        pub gpio6: Gpio6<'static>,
        pub gpio7: Gpio7<'static>,
        pub gpio8: Gpio8<'static>,
        pub gpio9: Gpio9<'static>,
        pub gpio10: Gpio10<'static>,
        pub gpio11: Gpio11<'static>,
        pub gpio12: Gpio12<'static>,
        pub gpio13: Gpio13<'static>,
        pub gpio14: Gpio14<'static>,
        pub gpio15: Gpio15<'static>,
        pub gpio16: Gpio16<'static>,
        pub gpio17: Gpio17<'static>,
        pub gpio18: Gpio18<'static>,
        pub gpio19: Gpio19<'static>,
        pub gpio20: Gpio20<'static>,
        pub gpio21: Gpio21<'static>,
        pub gpio22: Gpio22<'static>,
        pub gpio23: Gpio23<'static>,
        pub gpio24: Gpio24<'static>,
        pub gpio25: Gpio25<'static>,
        pub gpio26: Gpio26<'static>,
        pub gpio27: Gpio27<'static>,
        pub gpio28: Gpio28<'static>,
    }

    impl Pins {
        /// # Safety
        ///
        /// Care should be taken not to instantiate the Pins structure, if it is
        /// already instantiated and used elsewhere
        pub unsafe fn new() -> Self {
            Self {
                gpio0: Gpio0::steal(),
                gpio1: Gpio1::steal(),
                gpio2: Gpio2::steal(),
                gpio3: Gpio3::steal(),
                gpio4: Gpio4::steal(),
                gpio5: Gpio5::steal(),
                gpio6: Gpio6::steal(),
                gpio7: Gpio7::steal(),
                gpio8: Gpio8::steal(),
                gpio9: Gpio9::steal(),
                gpio10: Gpio10::steal(),
                gpio11: Gpio11::steal(),
                gpio12: Gpio12::steal(),
                gpio13: Gpio13::steal(),
                gpio14: Gpio14::steal(),
                gpio15: Gpio15::steal(),
                gpio16: Gpio16::steal(),
                gpio17: Gpio17::steal(),
                gpio18: Gpio18::steal(),
                gpio19: Gpio19::steal(),
                gpio20: Gpio20::steal(),
                gpio21: Gpio21::steal(),
                gpio22: Gpio22::steal(),
                gpio23: Gpio23::steal(),
                gpio24: Gpio24::steal(),
                gpio25: Gpio25::steal(),
                gpio26: Gpio26::steal(),
                gpio27: Gpio27::steal(),
                gpio28: Gpio28::steal(),
            }
        }
    }
}

#[cfg(esp32c6)]
mod chip {
    // TODO: Implement esp32c6 glitch filters

    #[cfg(feature = "alloc")]
    extern crate alloc;

    #[cfg(feature = "alloc")]
    use alloc::boxed::Box;

    use crate::interrupt::asynch::HalIsrNotification;

    use super::*;

    #[allow(clippy::type_complexity)]
    #[cfg(feature = "alloc")]
    pub(crate) static mut PIN_ISR_HANDLER: [Option<Box<dyn FnMut() + Send + 'static>>; 31] =
        [PIN_ISR_INIT; 31];

    #[allow(clippy::type_complexity)]
    pub(crate) static PIN_NOTIF: [HalIsrNotification; 31] = [PIN_NOTIF_INIT; 31];

    pin!(Gpio0:0, IO, RTC:0, ADC1:ADCCH0, NODAC:0, NOTOUCH:0);
    pin!(Gpio1:1, IO, RTC:1, ADC1:ADCCH1, NODAC:0, NOTOUCH:0);
    pin!(Gpio2:2, IO, RTC:2, ADC1:ADCCH2, NODAC:0, NOTOUCH:0);
    pin!(Gpio3:3, IO, RTC:3, ADC1:ADCCH3, NODAC:0, NOTOUCH:0);
    pin!(Gpio4:4, IO, RTC:4, ADC1:ADCCH4, NODAC:0, NOTOUCH:0);
    pin!(Gpio5:5, IO, RTC:5, ADC1:ADCCH5, NODAC:0, NOTOUCH:0);
    pin!(Gpio6:6, IO, RTC:6, ADC1:ADCCH6, NODAC:0, NOTOUCH:0);
    pin!(Gpio7:7, IO, RTC:7, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio8:8, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio9:9, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio10:10, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio11:11, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio12:12, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio13:13, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio14:14, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio15:15, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio16:16, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio17:17, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio18:18, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio19:19, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio20:20, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio21:21, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio22:22, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio23:23, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio24:24, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio25:25, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio26:26, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio27:27, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio28:28, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio29:29, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio30:30, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pub struct Pins {
        pub gpio0: Gpio0<'static>,
        pub gpio1: Gpio1<'static>,
        pub gpio2: Gpio2<'static>,
        pub gpio3: Gpio3<'static>,
        pub gpio4: Gpio4<'static>,
        pub gpio5: Gpio5<'static>,
        pub gpio6: Gpio6<'static>,
        pub gpio7: Gpio7<'static>,
        pub gpio8: Gpio8<'static>,
        pub gpio9: Gpio9<'static>,
        pub gpio10: Gpio10<'static>,
        pub gpio11: Gpio11<'static>,
        pub gpio12: Gpio12<'static>,
        pub gpio13: Gpio13<'static>,
        pub gpio14: Gpio14<'static>,
        pub gpio15: Gpio15<'static>,
        pub gpio16: Gpio16<'static>,
        pub gpio17: Gpio17<'static>,
        pub gpio18: Gpio18<'static>,
        pub gpio19: Gpio19<'static>,
        pub gpio20: Gpio20<'static>,
        pub gpio21: Gpio21<'static>,
        pub gpio22: Gpio22<'static>,
        pub gpio23: Gpio23<'static>,
        pub gpio24: Gpio24<'static>,
        pub gpio25: Gpio25<'static>,
        pub gpio26: Gpio26<'static>,
        pub gpio27: Gpio27<'static>,
        pub gpio28: Gpio28<'static>,
        pub gpio29: Gpio29<'static>,
        pub gpio30: Gpio30<'static>,
    }

    impl Pins {
        /// # Safety
        ///
        /// Care should be taken not to instantiate the Pins structure, if it is
        /// already instantiated and used elsewhere
        pub unsafe fn new() -> Self {
            Self {
                gpio0: Gpio0::steal(),
                gpio1: Gpio1::steal(),
                gpio2: Gpio2::steal(),
                gpio3: Gpio3::steal(),
                gpio4: Gpio4::steal(),
                gpio5: Gpio5::steal(),
                gpio6: Gpio6::steal(),
                gpio7: Gpio7::steal(),
                gpio8: Gpio8::steal(),
                gpio9: Gpio9::steal(),
                gpio10: Gpio10::steal(),
                gpio11: Gpio11::steal(),
                gpio12: Gpio12::steal(),
                gpio13: Gpio13::steal(),
                gpio14: Gpio14::steal(),
                gpio15: Gpio15::steal(),
                gpio16: Gpio16::steal(),
                gpio17: Gpio17::steal(),
                gpio18: Gpio18::steal(),
                gpio19: Gpio19::steal(),
                gpio20: Gpio20::steal(),
                gpio21: Gpio21::steal(),
                gpio22: Gpio22::steal(),
                gpio23: Gpio23::steal(),
                gpio24: Gpio24::steal(),
                gpio25: Gpio25::steal(),
                gpio26: Gpio26::steal(),
                gpio27: Gpio27::steal(),
                gpio28: Gpio28::steal(),
                gpio29: Gpio29::steal(),
                gpio30: Gpio30::steal(),
            }
        }
    }
}

#[cfg(esp32c61)]
mod chip {
    #[cfg(feature = "alloc")]
    extern crate alloc;

    #[cfg(feature = "alloc")]
    use alloc::boxed::Box;

    use crate::interrupt::asynch::HalIsrNotification;

    use super::*;

    #[allow(clippy::type_complexity)]
    #[cfg(feature = "alloc")]
    pub(crate) static mut PIN_ISR_HANDLER: [Option<Box<dyn FnMut() + Send + 'static>>; 30] =
        [PIN_ISR_INIT; 30];

    #[allow(clippy::type_complexity)]
    pub(crate) static PIN_NOTIF: [HalIsrNotification; 30] = [PIN_NOTIF_INIT; 30];

    pin!(Gpio0:0, IO, RTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio1:1, IO, RTC:1, ADC1:ADCCH0, NODAC:0, NOTOUCH:0);
    pin!(Gpio2:2, IO, RTC:2, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio3:3, IO, RTC:3, ADC1:ADCCH1, NODAC:0, NOTOUCH:0);
    pin!(Gpio4:4, IO, RTC:4, ADC1:ADCCH2, NODAC:0, NOTOUCH:0);
    pin!(Gpio5:5, IO, RTC:5, ADC1:ADCCH3, NODAC:0, NOTOUCH:0);
    pin!(Gpio6:6, IO, RTC:6, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio7:7, IO, RTC:7, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio8:8, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio9:9, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio10:10, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio11:11, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio12:12, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio13:13, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio14:14, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio15:15, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio16:16, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio17:17, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio18:18, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio19:19, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio20:20, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio21:21, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio22:22, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio23:23, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio24:24, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio25:25, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio26:26, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio27:27, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio28:28, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio29:29, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pub struct Pins {
        pub gpio0: Gpio0<'static>,
        pub gpio1: Gpio1<'static>,
        pub gpio2: Gpio2<'static>,
        pub gpio3: Gpio3<'static>,
        pub gpio4: Gpio4<'static>,
        pub gpio5: Gpio5<'static>,
        pub gpio6: Gpio6<'static>,
        pub gpio7: Gpio7<'static>,
        pub gpio8: Gpio8<'static>,
        pub gpio9: Gpio9<'static>,
        pub gpio10: Gpio10<'static>,
        pub gpio11: Gpio11<'static>,
        pub gpio12: Gpio12<'static>,
        pub gpio13: Gpio13<'static>,
        pub gpio14: Gpio14<'static>,
        pub gpio15: Gpio15<'static>,
        pub gpio16: Gpio16<'static>,
        pub gpio17: Gpio17<'static>,
        pub gpio18: Gpio18<'static>,
        pub gpio19: Gpio19<'static>,
        pub gpio20: Gpio20<'static>,
        pub gpio21: Gpio21<'static>,
        pub gpio22: Gpio22<'static>,
        pub gpio23: Gpio23<'static>,
        pub gpio24: Gpio24<'static>,
        pub gpio25: Gpio25<'static>,
        pub gpio26: Gpio26<'static>,
        pub gpio27: Gpio27<'static>,
        pub gpio28: Gpio28<'static>,
        pub gpio29: Gpio29<'static>,
    }

    impl Pins {
        /// # Safety
        ///
        /// Care should be taken not to instantiate the Pins structure, if it is
        /// already instantiated and used elsewhere
        pub unsafe fn new() -> Self {
            Self {
                gpio0: Gpio0::steal(),
                gpio1: Gpio1::steal(),
                gpio2: Gpio2::steal(),
                gpio3: Gpio3::steal(),
                gpio4: Gpio4::steal(),
                gpio5: Gpio5::steal(),
                gpio6: Gpio6::steal(),
                gpio7: Gpio7::steal(),
                gpio8: Gpio8::steal(),
                gpio9: Gpio9::steal(),
                gpio10: Gpio10::steal(),
                gpio11: Gpio11::steal(),
                gpio12: Gpio12::steal(),
                gpio13: Gpio13::steal(),
                gpio14: Gpio14::steal(),
                gpio15: Gpio15::steal(),
                gpio16: Gpio16::steal(),
                gpio17: Gpio17::steal(),
                gpio18: Gpio18::steal(),
                gpio19: Gpio19::steal(),
                gpio20: Gpio20::steal(),
                gpio21: Gpio21::steal(),
                gpio22: Gpio22::steal(),
                gpio23: Gpio23::steal(),
                gpio24: Gpio24::steal(),
                gpio25: Gpio25::steal(),
                gpio26: Gpio26::steal(),
                gpio27: Gpio27::steal(),
                gpio28: Gpio28::steal(),
                gpio29: Gpio29::steal(),
            }
        }
    }
}

#[cfg(esp32p4)]
mod chip {
    #[cfg(feature = "alloc")]
    extern crate alloc;

    #[cfg(feature = "alloc")]
    use alloc::boxed::Box;

    use crate::interrupt::asynch::HalIsrNotification;

    use super::*;

    #[allow(clippy::type_complexity)]
    #[cfg(feature = "alloc")]
    pub(crate) static mut PIN_ISR_HANDLER: [Option<Box<dyn FnMut() + Send + 'static>>; 54] =
        [PIN_ISR_INIT; 54];

    #[allow(clippy::type_complexity)]
    pub(crate) static PIN_NOTIF: [HalIsrNotification; 54] = [PIN_NOTIF_INIT; 54];

    pin!(Gpio0:0, IO, RTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio1:1, IO, RTC:1, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio2:2, IO, RTC:2, NOADC:NOADC, NODAC:0, TOUCH:0);
    pin!(Gpio3:3, IO, RTC:3, NOADC:NOADC, NODAC:0, TOUCH:1);
    pin!(Gpio4:4, IO, RTC:4, NOADC:NOADC, NODAC:0, TOUCH:2);
    pin!(Gpio5:5, IO, RTC:5, NOADC:NOADC, NODAC:0, TOUCH:3);
    pin!(Gpio6:6, IO, RTC:6, NOADC:NOADC, NODAC:0, TOUCH:4);
    pin!(Gpio7:7, IO, RTC:7, NOADC:NOADC, NODAC:0, TOUCH:5);
    pin!(Gpio8:8, IO, RTC:8, NOADC:NOADC, NODAC:0, TOUCH:6);
    pin!(Gpio9:9, IO, RTC:9, NOADC:NOADC, NODAC:0, TOUCH:7);

    pin!(Gpio10:10, IO, RTC:10, NOADC:NOADC, NODAC:0, TOUCH:8);
    pin!(Gpio11:11, IO, RTC:11, NOADC:NOADC, NODAC:0, TOUCH:9);
    pin!(Gpio12:12, IO, RTC:12, NOADC:NOADC, NODAC:0, TOUCH:10);
    pin!(Gpio13:13, IO, RTC:13, NOADC:NOADC, NODAC:0, TOUCH:11);
    pin!(Gpio14:14, IO, RTC:14, NOADC:NOADC, NODAC:0, TOUCH:12);
    pin!(Gpio15:15, IO, RTC:15, NOADC:NOADC, NODAC:0, TOUCH:13);
    pin!(Gpio16:16, IO, NORTC:0, ADC1:ADCCH0, NODAC:0, NOTOUCH:0);
    pin!(Gpio17:17, IO, NORTC:0, ADC1:ADCCH1, NODAC:0, NOTOUCH:0);
    pin!(Gpio18:18, IO, NORTC:0, ADC1:ADCCH2, NODAC:0, NOTOUCH:0);
    pin!(Gpio19:19, IO, NORTC:0, ADC1:ADCCH3, NODAC:0, NOTOUCH:0);

    pin!(Gpio20:20, IO, NORTC:0, ADC1:ADCCH4, NODAC:0, NOTOUCH:0);
    pin!(Gpio21:21, IO, NORTC:0, ADC1:ADCCH5, NODAC:0, NOTOUCH:0);
    pin!(Gpio22:22, IO, NORTC:0, ADC1:ADCCH6, NODAC:0, NOTOUCH:0);
    pin!(Gpio23:23, IO, NORTC:0, ADC1:ADCCH7, NODAC:0, NOTOUCH:0);
    pin!(Gpio24:24, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio25:25, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio26:26, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio27:27, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio28:28, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio29:29, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pin!(Gpio30:30, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio31:31, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio32:32, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio33:33, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio34:34, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio35:35, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio36:36, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio37:37, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio38:38, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio39:39, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pin!(Gpio40:40, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio41:41, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio42:42, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio43:43, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio44:44, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio45:45, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio46:46, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio47:47, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio48:48, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio49:49, IO, NORTC:0, ADC2:ADCCH0, NODAC:0, NOTOUCH:0);

    pin!(Gpio50:50, IO, NORTC:0, ADC2:ADCCH1, NODAC:0, NOTOUCH:0);
    pin!(Gpio51:51, IO, NORTC:0, ADC2:ADCCH2, NODAC:0, NOTOUCH:0);
    pin!(Gpio52:52, IO, NORTC:0, ADC2:ADCCH3, NODAC:0, NOTOUCH:0);
    pin!(Gpio53:53, IO, NORTC:0, ADC2:ADCCH4, NODAC:0, NOTOUCH:0);
    pin!(Gpio54:54, IO, NORTC:0, ADC2:ADCCH5, NODAC:0, NOTOUCH:0);

    pub struct Pins {
        pub gpio0: Gpio0<'static>,
        pub gpio1: Gpio1<'static>,
        pub gpio2: Gpio2<'static>,
        pub gpio3: Gpio3<'static>,
        pub gpio4: Gpio4<'static>,
        pub gpio5: Gpio5<'static>,
        pub gpio6: Gpio6<'static>,
        pub gpio7: Gpio7<'static>,
        pub gpio8: Gpio8<'static>,
        pub gpio9: Gpio9<'static>,

        pub gpio10: Gpio10<'static>,
        pub gpio11: Gpio11<'static>,
        pub gpio12: Gpio12<'static>,
        pub gpio13: Gpio13<'static>,
        pub gpio14: Gpio14<'static>,
        pub gpio15: Gpio15<'static>,
        pub gpio16: Gpio16<'static>,
        pub gpio17: Gpio17<'static>,
        pub gpio18: Gpio18<'static>,
        pub gpio19: Gpio19<'static>,

        pub gpio20: Gpio20<'static>,
        pub gpio21: Gpio21<'static>,
        pub gpio22: Gpio22<'static>,
        pub gpio23: Gpio23<'static>,
        pub gpio24: Gpio24<'static>,
        pub gpio25: Gpio25<'static>,
        pub gpio26: Gpio26<'static>,
        pub gpio27: Gpio27<'static>,
        pub gpio28: Gpio28<'static>,
        pub gpio29: Gpio29<'static>,

        pub gpio30: Gpio30<'static>,
        pub gpio31: Gpio31<'static>,
        pub gpio32: Gpio32<'static>,
        pub gpio33: Gpio33<'static>,
        pub gpio34: Gpio34<'static>,
        pub gpio35: Gpio35<'static>,
        pub gpio36: Gpio36<'static>,
        pub gpio37: Gpio37<'static>,
        pub gpio38: Gpio38<'static>,
        pub gpio39: Gpio39<'static>,

        pub gpio40: Gpio40<'static>,
        pub gpio41: Gpio41<'static>,
        pub gpio42: Gpio42<'static>,
        pub gpio43: Gpio43<'static>,
        pub gpio44: Gpio44<'static>,
        pub gpio45: Gpio45<'static>,
        pub gpio46: Gpio46<'static>,
        pub gpio47: Gpio47<'static>,
        pub gpio48: Gpio48<'static>,
        pub gpio49: Gpio49<'static>,

        pub gpio50: Gpio50<'static>,
        pub gpio51: Gpio51<'static>,
        pub gpio52: Gpio52<'static>,
        pub gpio53: Gpio53<'static>,
        pub gpio54: Gpio54<'static>,
    }

    impl Pins {
        /// # Safety
        ///
        /// Care should be taken not to instantiate the Pins structure, if it is
        /// already instantiated and used elsewhere
        pub unsafe fn new() -> Self {
            Self {
                gpio0: Gpio0::steal(),
                gpio1: Gpio1::steal(),
                gpio2: Gpio2::steal(),
                gpio3: Gpio3::steal(),
                gpio4: Gpio4::steal(),
                gpio5: Gpio5::steal(),
                gpio6: Gpio6::steal(),
                gpio7: Gpio7::steal(),
                gpio8: Gpio8::steal(),
                gpio9: Gpio9::steal(),

                gpio10: Gpio10::steal(),
                gpio11: Gpio11::steal(),
                gpio12: Gpio12::steal(),
                gpio13: Gpio13::steal(),
                gpio14: Gpio14::steal(),
                gpio15: Gpio15::steal(),
                gpio16: Gpio16::steal(),
                gpio17: Gpio17::steal(),
                gpio18: Gpio18::steal(),
                gpio19: Gpio19::steal(),

                gpio20: Gpio20::steal(),
                gpio21: Gpio21::steal(),
                gpio22: Gpio22::steal(),
                gpio23: Gpio23::steal(),
                gpio24: Gpio24::steal(),
                gpio25: Gpio25::steal(),
                gpio26: Gpio26::steal(),
                gpio27: Gpio27::steal(),
                gpio28: Gpio28::steal(),
                gpio29: Gpio29::steal(),

                gpio30: Gpio30::steal(),
                gpio31: Gpio31::steal(),
                gpio32: Gpio32::steal(),
                gpio33: Gpio33::steal(),
                gpio34: Gpio34::steal(),
                gpio35: Gpio35::steal(),
                gpio36: Gpio36::steal(),
                gpio37: Gpio37::steal(),
                gpio38: Gpio38::steal(),
                gpio39: Gpio39::steal(),

                gpio40: Gpio40::steal(),
                gpio41: Gpio41::steal(),
                gpio42: Gpio42::steal(),
                gpio43: Gpio43::steal(),
                gpio44: Gpio44::steal(),
                gpio45: Gpio45::steal(),
                gpio46: Gpio46::steal(),
                gpio47: Gpio47::steal(),
                gpio48: Gpio48::steal(),
                gpio49: Gpio49::steal(),

                gpio50: Gpio50::steal(),
                gpio51: Gpio51::steal(),
                gpio52: Gpio52::steal(),
                gpio53: Gpio53::steal(),
                gpio54: Gpio54::steal(),
            }
        }
    }
}

#[cfg(esp32s31)]
mod chip {
    #[cfg(feature = "alloc")]
    extern crate alloc;

    #[cfg(feature = "alloc")]
    use alloc::boxed::Box;

    use crate::interrupt::asynch::HalIsrNotification;

    use super::*;

    #[allow(clippy::type_complexity)]
    #[cfg(feature = "alloc")]
    pub(crate) static mut PIN_ISR_HANDLER: [Option<Box<dyn FnMut() + Send + 'static>>; 62] =
        [PIN_ISR_INIT; 62];

    #[allow(clippy::type_complexity)]
    pub(crate) static PIN_NOTIF: [HalIsrNotification; 62] = [PIN_NOTIF_INIT; 62];

    // NOTE: GPIO29 and GPIO41 do not exist on the esp32s31 (`SOC_GPIO_VALID_GPIO_MASK`)
    // NOTE: ADC1 (GPIO42-49) and ADC2 (GPIO50-57) are not supported yet: ESP-IDF v6.1 ships the
    //       esp32s31 ADC without attenuation or calibration and with 17-bit weighted-sum raw codes

    pin!(Gpio0:0, IO, RTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio1:1, IO, RTC:1, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio2:2, IO, RTC:2, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio3:3, IO, RTC:3, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio4:4, IO, RTC:4, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio5:5, IO, RTC:5, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio6:6, IO, RTC:6, NOADC:NOADC, NODAC:0, TOUCH:0);
    pin!(Gpio7:7, IO, RTC:7, NOADC:NOADC, NODAC:0, TOUCH:1);
    pin!(Gpio8:8, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:2);
    pin!(Gpio9:9, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:3);

    pin!(Gpio10:10, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:4);
    pin!(Gpio11:11, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:5);
    pin!(Gpio12:12, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:6);
    pin!(Gpio13:13, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:7);
    pin!(Gpio14:14, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:8);
    pin!(Gpio15:15, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:9);
    pin!(Gpio16:16, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:10);
    pin!(Gpio17:17, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:11);
    pin!(Gpio18:18, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:12);
    pin!(Gpio19:19, IO, NORTC:0, NOADC:NOADC, NODAC:0, TOUCH:13);

    pin!(Gpio20:20, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio21:21, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio22:22, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio23:23, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio24:24, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio25:25, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio26:26, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio27:27, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio28:28, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pin!(Gpio30:30, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio31:31, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio32:32, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio33:33, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio34:34, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio35:35, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio36:36, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio37:37, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio38:38, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio39:39, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pin!(Gpio40:40, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio42:42, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio43:43, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio44:44, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio45:45, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio46:46, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio47:47, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio48:48, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio49:49, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pin!(Gpio50:50, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio51:51, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio52:52, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio53:53, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio54:54, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio55:55, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio56:56, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio57:57, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio58:58, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio59:59, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pin!(Gpio60:60, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);
    pin!(Gpio61:61, IO, NORTC:0, NOADC:NOADC, NODAC:0, NOTOUCH:0);

    pub struct Pins {
        pub gpio0: Gpio0<'static>,
        pub gpio1: Gpio1<'static>,
        pub gpio2: Gpio2<'static>,
        pub gpio3: Gpio3<'static>,
        pub gpio4: Gpio4<'static>,
        pub gpio5: Gpio5<'static>,
        pub gpio6: Gpio6<'static>,
        pub gpio7: Gpio7<'static>,
        pub gpio8: Gpio8<'static>,
        pub gpio9: Gpio9<'static>,

        pub gpio10: Gpio10<'static>,
        pub gpio11: Gpio11<'static>,
        pub gpio12: Gpio12<'static>,
        pub gpio13: Gpio13<'static>,
        pub gpio14: Gpio14<'static>,
        pub gpio15: Gpio15<'static>,
        pub gpio16: Gpio16<'static>,
        pub gpio17: Gpio17<'static>,
        pub gpio18: Gpio18<'static>,
        pub gpio19: Gpio19<'static>,

        pub gpio20: Gpio20<'static>,
        pub gpio21: Gpio21<'static>,
        pub gpio22: Gpio22<'static>,
        pub gpio23: Gpio23<'static>,
        pub gpio24: Gpio24<'static>,
        pub gpio25: Gpio25<'static>,
        pub gpio26: Gpio26<'static>,
        pub gpio27: Gpio27<'static>,
        pub gpio28: Gpio28<'static>,

        pub gpio30: Gpio30<'static>,
        pub gpio31: Gpio31<'static>,
        pub gpio32: Gpio32<'static>,
        pub gpio33: Gpio33<'static>,
        pub gpio34: Gpio34<'static>,
        pub gpio35: Gpio35<'static>,
        pub gpio36: Gpio36<'static>,
        pub gpio37: Gpio37<'static>,
        pub gpio38: Gpio38<'static>,
        pub gpio39: Gpio39<'static>,

        pub gpio40: Gpio40<'static>,
        pub gpio42: Gpio42<'static>,
        pub gpio43: Gpio43<'static>,
        pub gpio44: Gpio44<'static>,
        pub gpio45: Gpio45<'static>,
        pub gpio46: Gpio46<'static>,
        pub gpio47: Gpio47<'static>,
        pub gpio48: Gpio48<'static>,
        pub gpio49: Gpio49<'static>,

        pub gpio50: Gpio50<'static>,
        pub gpio51: Gpio51<'static>,
        pub gpio52: Gpio52<'static>,
        pub gpio53: Gpio53<'static>,
        pub gpio54: Gpio54<'static>,
        pub gpio55: Gpio55<'static>,
        pub gpio56: Gpio56<'static>,
        pub gpio57: Gpio57<'static>,
        pub gpio58: Gpio58<'static>,
        pub gpio59: Gpio59<'static>,

        pub gpio60: Gpio60<'static>,
        pub gpio61: Gpio61<'static>,
    }

    impl Pins {
        /// # Safety
        ///
        /// Care should be taken not to instantiate the Pins structure, if it is
        /// already instantiated and used elsewhere
        pub unsafe fn new() -> Self {
            Self {
                gpio0: Gpio0::steal(),
                gpio1: Gpio1::steal(),
                gpio2: Gpio2::steal(),
                gpio3: Gpio3::steal(),
                gpio4: Gpio4::steal(),
                gpio5: Gpio5::steal(),
                gpio6: Gpio6::steal(),
                gpio7: Gpio7::steal(),
                gpio8: Gpio8::steal(),
                gpio9: Gpio9::steal(),

                gpio10: Gpio10::steal(),
                gpio11: Gpio11::steal(),
                gpio12: Gpio12::steal(),
                gpio13: Gpio13::steal(),
                gpio14: Gpio14::steal(),
                gpio15: Gpio15::steal(),
                gpio16: Gpio16::steal(),
                gpio17: Gpio17::steal(),
                gpio18: Gpio18::steal(),
                gpio19: Gpio19::steal(),

                gpio20: Gpio20::steal(),
                gpio21: Gpio21::steal(),
                gpio22: Gpio22::steal(),
                gpio23: Gpio23::steal(),
                gpio24: Gpio24::steal(),
                gpio25: Gpio25::steal(),
                gpio26: Gpio26::steal(),
                gpio27: Gpio27::steal(),
                gpio28: Gpio28::steal(),

                gpio30: Gpio30::steal(),
                gpio31: Gpio31::steal(),
                gpio32: Gpio32::steal(),
                gpio33: Gpio33::steal(),
                gpio34: Gpio34::steal(),
                gpio35: Gpio35::steal(),
                gpio36: Gpio36::steal(),
                gpio37: Gpio37::steal(),
                gpio38: Gpio38::steal(),
                gpio39: Gpio39::steal(),

                gpio40: Gpio40::steal(),
                gpio42: Gpio42::steal(),
                gpio43: Gpio43::steal(),
                gpio44: Gpio44::steal(),
                gpio45: Gpio45::steal(),
                gpio46: Gpio46::steal(),
                gpio47: Gpio47::steal(),
                gpio48: Gpio48::steal(),
                gpio49: Gpio49::steal(),

                gpio50: Gpio50::steal(),
                gpio51: Gpio51::steal(),
                gpio52: Gpio52::steal(),
                gpio53: Gpio53::steal(),
                gpio54: Gpio54::steal(),
                gpio55: Gpio55::steal(),
                gpio56: Gpio56::steal(),
                gpio57: Gpio57::steal(),
                gpio58: Gpio58::steal(),
                gpio59: Gpio59::steal(),

                gpio60: Gpio60::steal(),
                gpio61: Gpio61::steal(),
            }
        }
    }
}
