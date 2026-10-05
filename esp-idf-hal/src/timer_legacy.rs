use core::marker::PhantomData;

use esp_idf_sys::*;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::boxed::Box;

pub type TimerConfig = config::Config;

/// Timer configuration
pub mod config {

    #[derive(Copy, Clone)]
    pub struct Config {
        pub divider: u32,
        #[cfg(not(esp32))]
        pub xtal: bool,

        /// Enable or disable counter reload function when alarm event occurs.
        ///
        /// Enabling this makes the hardware automatically reset the counter
        /// to the value set by [`TimerDriver::set_counter`](super::TimerDriver::set_counter) when the alarm is fired.
        /// This allows creating timers that automatically fire at a given interval
        /// without the software having to do anything after the timer setup.
        pub auto_reload: bool,
    }

    impl Config {
        pub fn new() -> Self {
            Default::default()
        }

        #[must_use]
        pub fn divider(mut self, divider: u32) -> Self {
            self.divider = divider;
            self
        }

        #[must_use]
        #[cfg(not(esp32))]
        pub fn xtal(mut self, xtal: bool) -> Self {
            self.xtal = xtal;
            self
        }

        #[must_use]
        pub fn auto_reload(mut self, auto_reload: bool) -> Self {
            self.auto_reload = auto_reload;
            self
        }
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                divider: 80,
                #[cfg(not(esp32))]
                xtal: false,
                auto_reload: false,
            }
        }
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    #[allow(clippy::upper_case_acronyms)]
    #[derive(Default)]
    pub(crate) enum ClockSource {
        #[cfg(any(esp32, esp32s2, esp32s3, esp32c3))]
        #[default]
        APB,
        #[cfg(esp32c2)]
        #[default]
        PLL40,
        #[cfg(any(esp32h2, esp32h21))]
        #[default]
        PLL48,
        #[cfg(any(esp32c5, esp32c6, esp32c61))]
        #[default]
        PLL80,
        #[cfg(not(esp32))]
        XTAL,
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    #[allow(clippy::from_over_into)]
    impl Into<esp_idf_sys::soc_periph_tg_clk_src_legacy_t> for ClockSource {
        fn into(self) -> esp_idf_sys::soc_periph_tg_clk_src_legacy_t {
            match self {
                #[cfg(any(esp32, esp32s2, esp32s3, esp32c3))]
                ClockSource::APB => esp_idf_sys::soc_periph_tg_clk_src_legacy_t_TIMER_SRC_CLK_APB,
                #[cfg(esp32c2)]
                ClockSource::PLL40 => {
                    esp_idf_sys::soc_periph_tg_clk_src_legacy_t_TIMER_SRC_CLK_PLL_F40M
                }
                #[cfg(any(esp32h2, esp32h21))]
                ClockSource::PLL48 => {
                    esp_idf_sys::soc_periph_tg_clk_src_legacy_t_TIMER_SRC_CLK_PLL_F48M
                }
                #[cfg(any(esp32c5, esp32c6, esp32c61))]
                ClockSource::PLL80 => {
                    esp_idf_sys::soc_periph_tg_clk_src_legacy_t_TIMER_SRC_CLK_PLL_F80M
                }
                #[cfg(not(esp32))]
                ClockSource::XTAL => esp_idf_sys::soc_periph_tg_clk_src_legacy_t_TIMER_SRC_CLK_XTAL,
            }
        }
    }
}

pub trait Timer: Send {
    fn group() -> timer_group_t;
    fn index() -> timer_idx_t;
}

pub struct TimerDriver<'d> {
    timer: u8,
    divider: u32,
    #[cfg(all(not(esp32), not(esp_idf_version_major = "4")))]
    xtal: bool,
    isr_registered: bool,
    _p: PhantomData<&'d mut ()>,
}

impl<'d> TimerDriver<'d> {
    pub fn new<TIMER: Timer + 'd>(
        _timer: TIMER,
        config: &config::Config,
    ) -> Result<Self, EspError> {
        esp!(unsafe {
            timer_init(
                TIMER::group(),
                TIMER::index(),
                &timer_config_t {
                    alarm_en: timer_alarm_t_TIMER_ALARM_DIS,
                    counter_en: timer_start_t_TIMER_PAUSE,
                    counter_dir: timer_count_dir_t_TIMER_COUNT_UP,
                    auto_reload: if config.auto_reload {
                        timer_autoreload_t_TIMER_AUTORELOAD_EN
                    } else {
                        timer_autoreload_t_TIMER_AUTORELOAD_DIS
                    },
                    intr_type: timer_intr_mode_t_TIMER_INTR_LEVEL,
                    divider: config.divider,
                    #[cfg(all(not(esp32), esp_idf_version_major = "4"))]
                    clk_src: if config.xtal {
                        timer_src_clk_t_TIMER_SRC_CLK_XTAL
                    } else {
                        timer_src_clk_t_TIMER_SRC_CLK_APB
                    },
                    #[cfg(all(not(esp32), not(esp_idf_version_major = "4")))]
                    clk_src: if config.xtal {
                        config::ClockSource::XTAL.into()
                    } else {
                        config::ClockSource::default().into()
                    },
                    #[cfg(all(esp32, not(esp_idf_version_major = "4")))]
                    clk_src: config::ClockSource::default().into(),
                },
            )
        })?;

        Ok(Self {
            timer: ((TIMER::group() as u8) << 4) | (TIMER::index() as u8),
            divider: config.divider,
            #[cfg(all(not(esp32), not(esp_idf_version_major = "4")))]
            xtal: config.xtal,
            isr_registered: false,
            _p: PhantomData,
        })
    }

    ///
    /// Returns the tick rate of the timer.
    ///
    pub fn tick_hz(&self) -> u64 {
        let hz;

        #[cfg(esp_idf_version_major = "4")]
        {
            hz = TIMER_BASE_CLK / self.divider;
        }

        #[cfg(not(esp_idf_version_major = "4"))]
        {
            #[cfg(not(esp32))]
            if self.xtal {
                #[cfg(esp_idf_xtal_freq_24)]
                {
                    hz = 24_000_000 / self.divider;
                }
                #[cfg(esp_idf_xtal_freq_26)]
                {
                    hz = 26_000_000 / self.divider;
                }
                #[cfg(esp_idf_xtal_freq_32)]
                {
                    hz = 32_000_000 / self.divider;
                }
                #[cfg(esp_idf_xtal_freq_40)]
                {
                    hz = 40_000_000 / self.divider;
                }
                #[cfg(esp_idf_xtal_freq_48)]
                {
                    hz = 48_000_000 / self.divider;
                }
                #[cfg(esp_idf_xtal_freq_auto)] // c5
                {
                    hz = 48_000_000 / self.divider;
                }
            } else {
                #[cfg(any(esp32, esp32s2, esp32s3, esp32c3))]
                {
                    hz = APB_CLK_FREQ / self.divider;
                }
                #[cfg(esp32c2)] //PLL40
                {
                    hz = 40_000_000 / self.divider;
                }
                #[cfg(any(esp32h2, esp32h21))] //PLL48
                {
                    hz = 48_000_000 / self.divider;
                }
                #[cfg(any(esp32c5, esp32c6, esp32c61))] //PLL80
                {
                    hz = 80_000_000 / self.divider;
                }
            }
            #[cfg(esp32)]
            {
                hz = APB_CLK_FREQ / self.divider;
            }
        }

        hz as _
    }

    ///
    /// Enable or disable the timer.
    ///
    /// Enabling the timer causes it to begin counting
    /// up from the current counter.
    ///
    /// Disabling the timer effectively pauses the counter.
    ///
    pub fn enable(&mut self, enable: bool) -> Result<(), EspError> {
        self.check();

        if enable {
            esp!(unsafe { timer_start(self.group(), self.index()) })?;
        } else {
            esp!(unsafe { timer_pause(self.group(), self.index()) })?;
        }

        Ok(())
    }

    ///
    /// Returns the current counter value of the timer
    ///
    pub fn counter(&self) -> Result<u64, EspError> {
        let value = if crate::interrupt::active() {
            unsafe { timer_group_get_counter_value_in_isr(self.group(), self.index()) }
        } else {
            let mut value = 0_u64;

            esp!(unsafe { timer_get_counter_value(self.group(), self.index(), &mut value) })?;

            value
        };

        Ok(value)
    }

    ///
    /// Manually set the current counter value of the timer.
    ///
    /// This does not enable or disable the timer.
    ///
    pub fn set_counter(&mut self, value: u64) -> Result<(), EspError> {
        self.check();

        esp!(unsafe { timer_set_counter_value(self.group(), self.index(), value) })?;

        Ok(())
    }

    ///
    /// Enable or disable the alarm.
    ///
    /// Enabling the alarm activates the following behaviors once it is triggered:
    /// - The counter will reset to 0, if auto-reload is set
    /// - An interrupt will be triggered, if configured
    ///
    pub fn enable_alarm(&mut self, enable: bool) -> Result<(), EspError> {
        if crate::interrupt::active() {
            if enable {
                unsafe {
                    timer_group_enable_alarm_in_isr(self.group(), self.index());
                }
            } else {
                panic!("Disabling alarm from an ISR is not supported");
            }
        } else {
            esp!(unsafe {
                timer_set_alarm(
                    self.group(),
                    self.index(),
                    if enable {
                        timer_alarm_t_TIMER_ALARM_EN
                    } else {
                        timer_alarm_t_TIMER_ALARM_DIS
                    },
                )
            })?;
        }

        Ok(())
    }

    ///
    /// Returns the configured alarm value
    ///
    pub fn alarm(&self) -> Result<u64, EspError> {
        self.check();

        let mut value = 0_u64;

        esp!(unsafe { timer_get_alarm_value(self.group(), self.index(), &mut value) })?;

        Ok(value)
    }

    ///
    /// Set the alarm value of the timer.
    ///
    /// NOTE: The alarm must be activated with enable_alarm for this value to take effect
    ///
    /// Once the counter exceeds this value:
    /// - The counter will reset to 0, if auto-reload is set
    /// - An interrupt will be triggered, if configured
    ///
    pub fn set_alarm(&mut self, value: u64) -> Result<(), EspError> {
        if crate::interrupt::active() {
            unsafe {
                timer_group_set_alarm_value_in_isr(self.group(), self.index(), value);
            }
        } else {
            esp!(unsafe { timer_set_alarm_value(self.group(), self.index(), value) })?;
        }

        Ok(())
    }

    pub fn enable_interrupt(&mut self) -> Result<(), EspError> {
        self.check();

        if !self.isr_registered {
            // Driver will complain if we try to register when ISR CB is already registered
            esp!(unsafe {
                timer_isr_callback_add(
                    self.group(),
                    self.index(),
                    Some(Self::handle_isr),
                    (self.group() * timer_idx_t_TIMER_MAX + self.index()) as *mut core::ffi::c_void,
                    0,
                )
            })?;

            self.isr_registered = true;
        }

        Ok(())
    }

    pub fn disable_interrupt(&mut self) -> Result<(), EspError> {
        self.check();

        if self.isr_registered {
            // Driver will complain if we try to deregister when ISR callback is not registered
            esp!(unsafe { timer_isr_callback_remove(self.group(), self.index()) })?;

            self.isr_registered = false;
        }

        Ok(())
    }

    ///
    /// Delays for `counter` ticks
    ///
    /// NOTE: This function resets the counter
    ///
    ///
    pub async fn delay(&mut self, counter: u64) -> Result<(), EspError> {
        self.enable(false)?;
        self.enable_alarm(false)?;
        self.set_counter(0)?;
        self.set_alarm(counter)?;

        self.reset_wait();

        self.enable_interrupt()?;
        self.enable_alarm(true)?;
        self.enable(true)?;

        self.wait().await
    }

    ///
    /// Resets the internal wait notification
    ///
    pub fn reset_wait(&mut self) {
        let notif = &PIN_NOTIF[(self.group() * timer_idx_t_TIMER_MAX + self.index()) as usize];
        notif.reset();
    }

    ///
    /// Wait for an alarm interrupt to occur
    ///
    ///
    /// NOTE: This requires interrupts to be enabled to work
    ///
    pub async fn wait(&mut self) -> Result<(), EspError> {
        let notif = &PIN_NOTIF[(self.group() * timer_idx_t_TIMER_MAX + self.index()) as usize];

        notif.wait().await;

        Ok(())
    }

    /// Subscribes the provided callback for ISR notifications.
    /// As a side effect, interrupts will be disabled, so to receive a notification, one has
    /// to also call `TimerDriver::enable_interrupt` after calling this method.
    ///
    /// # Safety
    ///
    /// Care should be taken not to call STD, libc or FreeRTOS APIs (except for a few allowed ones)
    /// in the callback passed to this function, as it is executed in an ISR context.
    #[cfg(feature = "alloc")]
    pub unsafe fn subscribe<F>(&mut self, callback: F) -> Result<(), EspError>
    where
        F: FnMut() + Send + 'static,
    {
        self.internal_subscribe(callback)
    }

    /// Subscribes the provided callback for ISR notifications.
    /// As a side effect, interrupts will be disabled, so to receive a notification, one has
    /// to also call `TimerDriver::enable_interrupt` after calling this method.
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
    pub unsafe fn subscribe_nonstatic<F>(&mut self, callback: F) -> Result<(), EspError>
    where
        F: FnMut() + Send + 'd,
    {
        self.internal_subscribe(callback)
    }

    #[cfg(feature = "alloc")]
    fn internal_subscribe<F>(&mut self, callback: F) -> Result<(), EspError>
    where
        F: FnMut() + Send + 'd,
    {
        self.check();

        self.disable_interrupt()?;

        let callback: Box<dyn FnMut() + Send + 'd> = Box::new(callback);

        unsafe {
            ISR_HANDLERS[(self.group() * timer_idx_t_TIMER_MAX + self.index()) as usize] =
                Some(core::mem::transmute::<
                    Box<dyn FnMut() + Send>,
                    Box<dyn FnMut() + Send>,
                >(callback));
        }

        Ok(())
    }

    #[cfg(feature = "alloc")]
    pub fn unsubscribe(&mut self) -> Result<(), EspError> {
        self.check();

        self.disable_interrupt()?;

        unsafe {
            ISR_HANDLERS[(self.group() * timer_idx_t_TIMER_MAX + self.index()) as usize] = None;
        }

        Ok(())
    }

    fn check(&self) {
        if crate::interrupt::active() {
            panic!("This function cannot be called from an ISR");
        }
    }

    unsafe extern "C" fn handle_isr(index: *mut core::ffi::c_void) -> bool {
        use core::num::NonZeroU32;

        let index = index as usize;

        crate::interrupt::with_isr_yield_signal(move || {
            #[cfg(feature = "alloc")]
            {
                if let Some(handler) = ISR_HANDLERS[index].as_mut() {
                    handler();
                }
            }

            PIN_NOTIF[index].notify(NonZeroU32::new(1).unwrap());
        })
    }

    pub fn group(&self) -> timer_group_t {
        (self.timer >> 4) as _
    }

    pub fn index(&self) -> timer_idx_t {
        (self.timer & 0xf) as _
    }
}

impl Drop for TimerDriver<'_> {
    fn drop(&mut self) {
        self.disable_interrupt().unwrap();

        #[cfg(feature = "alloc")]
        unsafe {
            ISR_HANDLERS[(self.group() * timer_idx_t_TIMER_MAX + self.index()) as usize] = None;
        }

        PIN_NOTIF[(self.group() * timer_idx_t_TIMER_MAX + self.index()) as usize].reset();

        esp!(unsafe { timer_deinit(self.group(), self.index()) }).unwrap();
    }
}

unsafe impl Send for TimerDriver<'_> {}

impl embedded_hal_async::delay::DelayNs for TimerDriver<'_> {
    async fn delay_ns(&mut self, ns: u32) {
        let counter = core::cmp::max((self.tick_hz() * ns as u64) / 1000000, 1);

        self.delay(counter).await.unwrap();
    }

    async fn delay_ms(&mut self, ms: u32) {
        let counter = core::cmp::max((self.tick_hz() * ms as u64) / 1000, 1);

        self.delay(counter).await.unwrap();
    }
}

macro_rules! impl_timer {
    ($timer:ident: $group:expr, $index:expr) => {
        crate::impl_peripheral!($timer);

        impl Timer for $timer<'_> {
            #[inline(always)]
            fn group() -> timer_group_t {
                $group
            }

            #[inline(always)]
            fn index() -> timer_idx_t {
                $index
            }
        }
    };
}

#[allow(clippy::type_complexity)]
#[cfg(not(any(esp32, esp32s2, esp32s3)))]
#[cfg(feature = "alloc")]
static mut ISR_HANDLERS: [Option<Box<dyn FnMut() + Send + 'static>>; 2] = [None, None];

#[allow(clippy::type_complexity)]
#[cfg(not(any(esp32, esp32s2, esp32s3)))]
pub(crate) static PIN_NOTIF: [crate::interrupt::asynch::HalIsrNotification; 2] = [
    crate::interrupt::asynch::HalIsrNotification::new(),
    crate::interrupt::asynch::HalIsrNotification::new(),
];

#[allow(clippy::type_complexity)]
#[cfg(any(esp32, esp32s2, esp32s3))]
#[cfg(feature = "alloc")]
static mut ISR_HANDLERS: [Option<Box<dyn FnMut() + Send + 'static>>; 4] = [None, None, None, None];

#[allow(clippy::type_complexity)]
#[cfg(any(esp32, esp32s2, esp32s3))]
pub(crate) static PIN_NOTIF: [crate::interrupt::asynch::HalIsrNotification; 4] = [
    crate::interrupt::asynch::HalIsrNotification::new(),
    crate::interrupt::asynch::HalIsrNotification::new(),
    crate::interrupt::asynch::HalIsrNotification::new(),
    crate::interrupt::asynch::HalIsrNotification::new(),
];

impl_timer!(TIMER00: timer_group_t_TIMER_GROUP_0, timer_idx_t_TIMER_0);
#[cfg(any(esp32, esp32s2, esp32s3))]
impl_timer!(TIMER01: timer_group_t_TIMER_GROUP_0, timer_idx_t_TIMER_1);
#[cfg(not(esp32c2))]
impl_timer!(TIMER10: timer_group_t_TIMER_GROUP_1, timer_idx_t_TIMER_0);
#[cfg(any(esp32, esp32s2, esp32s3))]
impl_timer!(TIMER11: timer_group_t_TIMER_GROUP_1, timer_idx_t_TIMER_1);
