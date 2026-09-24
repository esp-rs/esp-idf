//! Timer driver for the [Timer Group peripheral].
//!
//! This timer can select different clock sources and prescalers to meet the requirements
//! of nanosecond-level resolution. Additionally, it has flexible timeout alarm functions
//! and allows automatic updating of the count value at the alarm moment,
//! achieving very precise timing cycles.
//!
//! Based on the high resolution, high count range, and high response capabilities of the
//! hardware timer, the main application scenarios of this driver include:
//! - Running freely as a calendar clock to provide timestamp services for other modules
//! - Generating periodic alarms to complete periodic tasks
//! - Generating one-shot alarms, which can be used to implement a monotonic software timer list
//!   with asynchronous updates of alarm values
//! - Working with the GPIO module to achieve PWM signal output and input capture
//! - ...
//!
//! [Timer Group peripheral]: https://documentation.espressif.com/esp32_technical_reference_manual_en.pdf#timg
//!
//! # Driver redesign in ESP-IDF 5.0
//!
//! In ESP-IDF 5.0, the [timer API was redesigned] to simplify and unify the usage of
//! general purpose timer.
//!
//! It is recommended to use the new API, but for now the old API is available through
//! the `timer-legacy` feature. The ESP-IDF 6.0 release will remove support for the legacy API.
//!
//! [timer API was redesigned]: https://docs.espressif.com/projects/esp-idf/en/stable/esp32/migration-guides/release-5.x/5.0/peripherals.html#timer-group-driver
//!
//! # Force enabling debug logs
//!
//! The `CONFIG_GPTIMER_ENABLE_DEBUG_LOG` option in the `sdkconfig` forces the
//! GPTimer driver to enable all debug logs, regardless of the global log level settings.
//! Enabling this option helps developers obtain more detailed log information during
//! debugging, making it easier to locate and solve problems.
use core::ffi::c_void;
use core::time::Duration;
use core::{fmt, ptr};

use alloc::boxed::Box;

use esp_idf_sys::*;

use crate::interrupt;
use crate::interrupt::asynch::HalIsrNotification;
use crate::units::{FromValueType, Hertz};
use config::*;

/// This might not always be available in the generated `esp-idf-sys` bindings,
/// which is why it is defined here.
pub const ERR_EOVERFLOW: esp_err_t = 139;

/// GPTimer alarm event data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AlarmEventData {
    /// Current count value
    pub count_value: u64,
    /// Current alarm value
    pub alarm_value: u64,
}

impl From<gptimer_alarm_event_data_t> for AlarmEventData {
    fn from(value: gptimer_alarm_event_data_t) -> Self {
        Self {
            count_value: value.count_value,
            alarm_value: value.alarm_value,
        }
    }
}

/// Timer configuration
pub mod config {
    use esp_idf_sys::*;

    use crate::units::{FromValueType, Hertz};

    /// GPTimer count direction
    #[derive(Debug, Clone, Default)]
    #[non_exhaustive]
    pub enum CountDirection {
        /// Decrease count value
        #[default]
        Up,
        /// Increase count value
        Down,
    }

    impl From<CountDirection> for gptimer_count_direction_t {
        fn from(value: CountDirection) -> Self {
            match value {
                CountDirection::Up => gptimer_count_direction_t_GPTIMER_COUNT_UP,
                CountDirection::Down => gptimer_count_direction_t_GPTIMER_COUNT_DOWN,
            }
        }
    }

    /// Type of GPTimer clock source.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
    pub enum ClockSource {
        /// Use the default clock source.
        #[default]
        Default,
        /// Select `APB` as the source clock
        #[cfg(any(esp32, esp32s2, esp32s3, esp32c3))]
        APB,
        /// Select `RC_FAST` as the source clock
        #[cfg(any(esp32c5, esp32c6, esp32c61, esp32h2, esp32p4, esp32s31))]
        RcFast,
        /// Select `XTAL` as the source clock
        #[cfg(any(
            esp32s2, esp32s3, esp32c2, esp32c3, esp32c5, esp32c6, esp32c61, esp32h2, esp32p4,
            esp32s31
        ))]
        XTAL,
        /// Select `PLL_F40M` as the source clock
        #[cfg(esp32c2)]
        PLLF40M,
        /// Select `PLL_F48M` as the source clock
        #[cfg(esp32h2)]
        PLLF48M,
        /// Select `PLL_F80M` as the source clock
        #[cfg(any(esp32c5, esp32c6, esp32c61, esp32p4, esp32s31))]
        PLLF80M,
    }

    impl From<ClockSource> for soc_periph_gptimer_clk_src_t {
        fn from(clock: ClockSource) -> Self {
            match clock {
                ClockSource::Default => soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_DEFAULT,
                #[cfg(any(esp32, esp32s2, esp32s3, esp32c3))]
                ClockSource::APB => soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_APB,
                #[cfg(any(esp32c5, esp32c6, esp32c61, esp32h2, esp32p4, esp32s31))]
                ClockSource::RcFast => soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_RC_FAST,
                #[cfg(any(
                    esp32s2, esp32s3, esp32c2, esp32c3, esp32c5, esp32c6, esp32c61, esp32h2,
                    esp32p4, esp32s31
                ))]
                ClockSource::XTAL => soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_XTAL,
                #[cfg(esp32c2)]
                ClockSource::PLLF40M => soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_PLL_F40M,
                #[cfg(esp32h2)]
                ClockSource::PLLF48M => soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_PLL_F48M,
                #[cfg(any(esp32c5, esp32c6, esp32c61, esp32p4, esp32s31))]
                ClockSource::PLLF80M => soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_PLL_F80M,
            }
        }
    }

    /// Configuration for [`TimerDriver`](super::TimerDriver)
    #[derive(Debug, Clone)]
    pub struct TimerConfig {
        /// GPTimer clock source
        pub clock_source: ClockSource,
        /// Count direction
        pub direction: CountDirection,
        /// Counter resolution (working frequency) in Hz, hence,
        /// the step size of each count tick equals to (1 / resolution_hz) seconds
        pub resolution: Hertz,
        /// GPTimer interrupt priority, if set to 0, the driver will try to allocate an interrupt
        /// with a relative low priority (1,2,3)
        #[cfg(esp_idf_version_at_least_5_1_2)]
        #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_1_2)))]
        pub intr_priority: i32,
        /// If set true, the timer interrupt number can be shared with other peripherals
        pub intr_shared: bool,
        /// If set, driver allows the power domain to be powered off when system enters
        /// sleep mode. This can save power, but at the expense of more RAM being consumed
        /// to save register context.
        #[cfg(esp_idf_version_at_least_5_4_0)]
        #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_4_0)))]
        pub allow_pd: bool,
        // This field is intentionally hidden to prevent non-exhaustive pattern matching.
        // You should only construct this struct using the `..Default::default()` pattern.
        // If you use this field directly, your code might break in future versions.
        #[doc(hidden)]
        #[allow(dead_code)]
        pub __internal: (),
    }

    impl Default for TimerConfig {
        fn default() -> Self {
            Self {
                clock_source: Default::default(),
                direction: Default::default(),
                resolution: 1_000_000.Hz(), // 1 MHz
                #[cfg(esp_idf_version_at_least_5_1_2)]
                intr_priority: 0,
                intr_shared: false,
                #[cfg(esp_idf_version_at_least_5_4_0)]
                allow_pd: false,
                __internal: (),
            }
        }
    }

    /// General Purpose Timer alarm configuration.
    #[derive(Debug, Clone)]
    pub struct AlarmConfig {
        /// Alarm reload count value, effect only when [`Self::auto_reload_on_alarm`] is set to true
        pub reload_count: u64,
        /// Alarm target count value
        pub alarm_count: u64,
        /// Reload the count value by hardware, immediately at the alarm event
        pub auto_reload_on_alarm: bool,
        // This field is intentionally hidden to prevent non-exhaustive pattern matching.
        // You should only construct this struct using the `..Default::default()` pattern.
        // If you use this field directly, your code might break in future versions.
        #[doc(hidden)]
        #[allow(dead_code)]
        pub __internal: (),
    }

    impl Default for AlarmConfig {
        fn default() -> Self {
            Self {
                reload_count: 0,
                alarm_count: 1_000_000,
                auto_reload_on_alarm: false,
                __internal: (),
            }
        }
    }

    impl From<&AlarmConfig> for gptimer_alarm_config_t {
        fn from(config: &AlarmConfig) -> Self {
            gptimer_alarm_config_t {
                alarm_count: config.alarm_count,
                reload_count: config.reload_count,
                flags: gptimer_alarm_config_t__bindgen_ty_1 {
                    _bitfield_1: gptimer_alarm_config_t__bindgen_ty_1::new_bitfield_1(
                        config.auto_reload_on_alarm as _,
                    ),
                    ..Default::default()
                },
            }
        }
    }
}

struct AlarmUserData<'d> {
    on_alarm: Box<dyn FnMut(AlarmEventData) + Send + 'd>,
    notif: HalIsrNotification,
}

/// General Purpose Timer driver.
///
/// You can use this driver to get notified when a certain amount of time has passed
/// ([`TimerDriver::subscribe`]), or to measure time intervals ([`TimerDriver::get_raw_count`]).
///
/// The driver has the following states:
/// - "init" state: After creation of the driver, or after calling [`TimerDriver::disable`].
/// - "enable" state: After calling [`TimerDriver::enable`]. In this state, the timer is ready to be started,
///   but the internal counter is not running yet.
/// - "run" state: After calling [`TimerDriver::start`]. In this state, the internal counter is running.
///   To stop the counter, call [`TimerDriver::stop`], which would transition back to "enable" state.
pub struct TimerDriver<'d> {
    handle: gptimer_handle_t,
    on_alarm: Option<Box<AlarmUserData<'d>>>,
}

impl<'d> TimerDriver<'d> {
    /// Create a new General Purpose Timer, and return the handle.
    ///
    /// The state of the returned timer will be "init" state.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of invalid argument
    /// - `ESP_ERR_NO_MEM`: Failed because out of memory
    /// - `ESP_ERR_NOT_FOUND`: Failed because all hardware timers are used up and no more free one
    /// - `ESP_FAIL`: Failed because of other error
    pub fn new(config: &TimerConfig) -> Result<Self, EspError> {
        let sys_config = gptimer_config_t {
            clk_src: config.clock_source.into(),
            direction: config.direction.clone().into(),
            resolution_hz: config.resolution.into(),
            #[cfg(esp_idf_version_at_least_5_1_2)]
            intr_priority: config.intr_priority,
            flags: gptimer_config_t__bindgen_ty_1 {
                _bitfield_1: gptimer_config_t__bindgen_ty_1::new_bitfield_1(
                    config.intr_shared as _,
                    #[cfg(esp_idf_version_at_least_5_4_0)]
                    {
                        config.allow_pd as _
                    },
                    // The `backup_before_sleep` field was removed in IDF 6.0
                    #[cfg(all(
                        esp_idf_version_at_least_5_3_0,
                        not(esp_idf_version_at_least_6_0_0)
                    ))]
                    {
                        false as _
                    },
                ),
                ..Default::default()
            },
        };
        let mut handle = ptr::null_mut();

        esp!(unsafe { gptimer_new_timer(&sys_config, &raw mut handle) })?;

        Ok(Self {
            handle,
            on_alarm: None,
        })
    }

    /// Returns the underlying GPTimer handle.
    pub fn handle(&self) -> gptimer_handle_t {
        self.handle
    }

    /// Converts the given duration to the corresponding timer count value.
    ///
    /// # Errors
    ///
    /// If [`Self::get_resolution`] fails, this function will return the same error.
    ///
    /// This function might overflow if the duration is too long to be represented
    /// as ticks with the current timer resolution.
    /// In that case an error with the code [`ERR_EOVERFLOW`] will be returned.
    #[cfg(esp_idf_version_at_least_5_1_0)]
    #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_1_0)))]
    pub fn duration_to_count(&self, duration: Duration) -> Result<u64, EspError> {
        // 1 / resolution = how many seconds per tick
        // -> duration / (1 / resolution) = duration * resolution = how many ticks in duration (where duration is in seconds)
        //
        // The below calculation splits the duration into full seconds and the remainder in nanoseconds.
        // The full seconds can simply be multiplied by the ticks per second (resolution).
        //
        // The remainder in nanoseconds would have to be converted to seconds to multiply with the resolution,
        // but with whole numbers that would round down to zero (remainder is less than a full second).
        // Therefore instead of:
        // ticks += resolution * (duration_rem_in_nanos / 1_000_000_000)
        // we do:
        // ticks += (resolution * duration_rem_in_nanos) / 1_000_000_000
        //
        // The 1_000_000_000 is one second in nanoseconds.

        let ticks_per_second = self.get_resolution()?.0 as u64;
        let duration_in_seconds = duration.as_secs();
        let duration_rem_in_nanos = duration.subsec_nanos() as u64;

        let Some(ticks) = ticks_per_second.checked_mul(duration_in_seconds) else {
            return Err(EspError::from_infallible::<ERR_EOVERFLOW>());
        };

        ticks
            .checked_add(
                (ticks_per_second * duration_rem_in_nanos)
                    / Duration::from_secs(1).as_nanos() as u64,
            )
            .ok_or(EspError::from_infallible::<ERR_EOVERFLOW>())
    }

    /// Asynchronously delay for the specified duration.
    ///
    /// This function will reset the timer count to 0, and set a one-shot alarm.
    /// Any existing count or alarm configuration will be overwritten.
    /// This function does **not** [`Self::start`] or [`Self::enable`] the timer,
    /// this must be done beforehand.
    ///
    /// # Errors
    ///
    /// If there is no interrupt service registered, it will return an `ESP_ERR_INVALID_STATE`.
    /// To enable interrupts, either register your own callback through
    /// [`Self::subscribe`]/[`Self::subscribe_nonstatic`], or call [`Self::subscribe_default`].
    #[cfg(esp_idf_version_at_least_5_1_0)]
    #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_1_0)))]
    pub async fn delay(&self, duration: Duration) -> Result<(), EspError> {
        let alarm_count = self.duration_to_count(duration)?;

        // Set alarm and reset the current count to 0
        self.set_raw_count(0)?;
        self.set_alarm_action(Some(&AlarmConfig {
            alarm_count,
            ..Default::default()
        }))?;

        let res = self.wait().await;

        // Unset the previous alarm
        self.set_alarm_action(None)?;

        res
    }

    fn notif(&self) -> Result<&HalIsrNotification, EspError> {
        self.on_alarm
            .as_ref()
            .map(|data| &data.notif)
            .ok_or(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())
    }

    /// Wait for the timer alarm event interrupt.
    ///
    /// # Errors
    ///
    /// If this function is called without a registered ISR, it will
    /// return an `ESP_ERR_INVALID_STATE`.
    /// To enable interrupts, either register your own callback through
    /// [`Self::subscribe`]/[`Self::subscribe_nonstatic`], or call [`Self::subscribe_default`].
    pub async fn wait(&self) -> Result<(), EspError> {
        self.notif()?.wait().await;

        Ok(())
    }

    /// Resets the internal wait notification.
    ///
    /// If no callback is registered, this function does nothing.
    pub fn reset_wait(&self) {
        if let Ok(notif) = self.notif() {
            notif.reset();
        }
    }

    /// Set GPTimer raw count value.
    ///
    /// When updating the raw count of an active timer, the timer will
    /// immediately start counting from the new value.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of invalid argument
    /// - `ESP_FAIL`: Failed because of other error
    pub fn set_raw_count(&self, value: u64) -> Result<(), EspError> {
        esp!(unsafe { gptimer_set_raw_count(self.handle, value) })
    }

    /// Get GPTimer raw count value.
    ///
    /// This function will trigger a software capture event and then return the captured count value.
    ///
    /// With the raw count value and the resolution returned from [`Self::get_resolution`], you can
    /// convert the count value into seconds.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of invalid argument (should not happen)
    /// - `ESP_FAIL`: Failed because of other error
    pub fn get_raw_count(&self) -> Result<u64, EspError> {
        let mut value: u64 = 0;
        esp!(unsafe { gptimer_get_raw_count(self.handle, &raw mut value) })?;
        Ok(value)
    }

    /// Return the real resolution of the timer.
    ///
    /// Usually the timer resolution is same as what you configured in the [`TimerConfig::resolution`],
    /// but some unstable clock source (e.g. `RC_FAST`) will do a calibration, the real resolution can
    /// be different from the configured one.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of invalid argument (should not happen)
    /// - `ESP_FAIL`: Failed because of other error
    #[cfg(esp_idf_version_at_least_5_1_0)]
    #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_1_0)))]
    pub fn get_resolution(&self) -> Result<Hertz, EspError> {
        let mut value: u32 = 0;
        esp!(unsafe { gptimer_get_resolution(self.handle, &raw mut value) })?;
        Ok(value.Hz())
    }

    /// Get GPTimer captured count value.
    ///
    /// Different from [`Self::get_raw_count`], this function won't trigger a software capture event.
    /// It just returns the last captured count value. It's especially useful when the capture has
    /// already been triggered by an external event and you want to read the captured value.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of invalid argument (should not happen)
    /// - `ESP_FAIL`: Failed because of other error
    #[cfg(esp_idf_version_at_least_5_1_0)]
    #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_1_0)))]
    pub fn get_captured_count(&self) -> Result<u64, EspError> {
        let mut value: u64 = 0;
        esp!(unsafe { gptimer_get_captured_count(self.handle, &raw mut value) })?;
        Ok(value)
    }

    /// Define the ISR handler for when the alarm event occurs.
    ///
    /// The callbacks are expected to run in ISR context.
    /// The first call to this function should happen before the timer is enabled
    /// through [`Self::enable`].
    ///
    /// There is only one callback possible, you can not subscribe multiple callbacks.
    ///
    /// # ISR Safety
    ///
    /// Care should be taken not to call std, libc or FreeRTOS APIs (except for a few allowed ones)
    /// in the callback passed to this function, as it is executed in an ISR context.
    ///
    /// You are not allowed to block, but you are allowed to call FreeRTOS APIs with the FromISR suffix.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of invalid argument
    /// - `ESP_ERR_INVALID_STATE`: Failed because the timer is not in init state
    /// - `ESP_FAIL`: Failed because of other error
    pub fn subscribe(
        &mut self,
        on_alarm: impl FnMut(AlarmEventData) + Send + 'static,
    ) -> Result<(), EspError> {
        unsafe { self.subscribe_nonstatic(on_alarm) }
    }

    /// Subscribe a non-'static callback for when a transmission is done.
    ///
    /// # Safety
    ///
    /// You must not forget the driver (for example through [`core::mem::forget`]), while the callback
    /// is still subscribed, otherwise this would lead to undefined behavior.
    ///
    /// To unsubscribe the callback, call [`Self::unsubscribe`].
    pub unsafe fn subscribe_nonstatic(
        &mut self,
        on_alarm: impl FnMut(AlarmEventData) + Send + 'd,
    ) -> Result<(), EspError> {
        let mut user_data = Box::new(AlarmUserData {
            on_alarm: Box::new(on_alarm),
            notif: HalIsrNotification::new(),
        });
        let cbs = gptimer_event_callbacks_t {
            on_alarm: Some(Self::handle_isr),
        };

        esp!(unsafe {
            gptimer_register_event_callbacks(self.handle, &cbs, (&raw mut *user_data) as *mut _)
        })?;

        // Store the user data in the struct to prevent it from being freed too early
        self.on_alarm = Some(user_data);

        Ok(())
    }

    /// Register the default callback.
    ///
    /// This function will overwrite any previously registered callbacks.
    /// This is useful if you want to asynchronously wait for an alarm event
    /// through [`Self::wait`] or [`Self::delay`].
    pub fn subscribe_default(&mut self) -> Result<(), EspError> {
        self.subscribe(|_| {})
    }

    /// Unregister the previously registered callback.
    pub fn unsubscribe(&mut self) -> Result<(), EspError> {
        esp!(unsafe {
            gptimer_register_event_callbacks(self.handle, ptr::null(), ptr::null_mut())
        })?;
        self.on_alarm = None;

        Ok(())
    }

    /// Set alarm event actions for GPTimer.
    ///
    /// If the config is `None`, the alarm will be disabled.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of invalid argument (should not happen)
    /// - `ESP_FAIL`: Failed because of other error
    pub fn set_alarm_action(&self, config: Option<&AlarmConfig>) -> Result<(), EspError> {
        let sys_config = config.map(|c| c.into());

        esp!(unsafe {
            gptimer_set_alarm_action(self.handle, sys_config.as_ref().map_or(ptr::null(), |c| c))
        })
    }

    /// Enable the timer.
    ///
    /// This function will transition the timer from the "init" state to "enable" state.
    ///
    /// # Note
    ///
    /// This function will enable the interrupt service, if a callback has been registered
    /// through [`Self::subscribe`].
    ///
    /// It will acquire a power management lock, if a specific source clock (e.g. APB) is selected
    /// in the timer configuration, while `CONFIG_PM_ENABLE` is set in the project configuration.
    ///
    /// To make the timer start counting, call [`Self::start`].
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of invalid argument (should not happen)
    /// - `ESP_ERR_INVALID_STATE`: Failed because the timer is not in "init" state (e.g. already enabled)
    /// - `ESP_FAIL`: Failed because of other error
    pub fn enable(&self) -> Result<(), EspError> {
        esp!(unsafe { gptimer_enable(self.handle) })
    }

    /// Disable the timer.
    ///
    /// This function will transition the timer from the "enable" state to "init" state.
    ///
    /// # Note
    ///
    /// This function will disable the interrupt service, if a callback has been registered
    /// through [`Self::subscribe`].
    ///
    /// It will release the power management lock, if it acquired one in [`Self::enable`].
    ///
    /// Disabling the timer will not make it stop counting.
    /// To make the timer stop counting, call [`Self::stop`].
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of invalid argument (should not happen)
    /// - `ESP_ERR_INVALID_STATE`: Failed because the timer is not in "enable" state (e.g. already disabled)
    /// - `ESP_FAIL`: Failed because of other error
    pub fn disable(&self) -> Result<(), EspError> {
        esp!(unsafe { gptimer_disable(self.handle) })
    }

    /// Start GPTimer (internal counter starts counting)
    ///
    /// This function will transition the timer from the "enable" state to "run" state.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of invalid argument (should not happen)
    /// - `ESP_ERR_INVALID_STATE`: Failed because the timer is not enabled or already in running
    /// - `ESP_FAIL`: Failed because of other error
    pub fn start(&self) -> Result<(), EspError> {
        esp!(unsafe { gptimer_start(self.handle) })
    }

    /// Stop GPTimer (internal counter stops counting)
    ///
    /// This function will transition the timer from the "run" state to "enable" state.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of invalid argument (should not happen)
    /// - `ESP_ERR_INVALID_STATE`: Failed because the timer is not in running.
    /// - `ESP_FAIL`: Failed because of other error
    pub fn stop(&self) -> Result<(), EspError> {
        esp!(unsafe { gptimer_stop(self.handle) })
    }

    unsafe extern "C" fn handle_isr(
        _handle: gptimer_handle_t,
        event_data: *const gptimer_alarm_event_data_t,
        arg: *mut c_void,
    ) -> bool {
        let user_data = &mut *(arg as *mut AlarmUserData);
        let event = AlarmEventData::from(*event_data);

        interrupt::with_isr_yield_signal(|| {
            (user_data.on_alarm)(event);

            user_data.notif.notify_lsb();
        })
    }
}

// SAFETY: According to the ESP-IDF docs, the driver can be used in a multi-threaded context
//         without extra locking.
unsafe impl<'d> Send for TimerDriver<'d> {}
unsafe impl<'d> Sync for TimerDriver<'d> {}

impl<'d> Drop for TimerDriver<'d> {
    fn drop(&mut self) {
        // Timer must be from run -> enable state first before it is disabled
        let _ = self.stop();
        // Timer must be in "init" state before deletion (= disable state)
        let _ = self.disable();

        unsafe {
            gptimer_del_timer(self.handle);
        }
    }
}

impl<'d> fmt::Debug for TimerDriver<'d> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimerDriver")
            .field("handle", &self.handle)
            .finish()
    }
}
