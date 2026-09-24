use enumset::{EnumSet, EnumSetType};

use esp_idf_sys::*;

/// For backwards compatibility
pub type IntrFlags = InterruptType;

/// Interrupt allocation flags.
/// These flags can be used to specify which interrupt qualities the code calling esp_intr_alloc* needs.
#[derive(Debug, EnumSetType)]
pub enum InterruptType {
    // Accept a Level 1 interrupt vector (lowest priority)
    Level1,
    // Accept a Level 2 interrupt vector.
    Level2,
    // Accept a Level 3 interrupt vector.
    Level3,
    // Accept a Level 4 interrupt vector.
    Level4,
    // Accept a Level 5 interrupt vector.
    Level5,
    // Accept a Level 6 interrupt vector.
    Level6,
    // Accept a Level 7 interrupt vector (highest priority)
    Nmi,
    // Interrupt can be shared between ISRs.
    Shared,
    // Edge-triggered interrupt.
    Edge,
    // ISR can be called if cache is disabled.
    // Must be used with a proper option *_ISR_IN_IRAM in SDKCONFIG
    Iram,
    // Return with this interrupt disabled.
    IntrDisabled,
    // Low and medium prio interrupts. These can be handled in C.
    LowMed,
    // High level interrupts. Need to be handled in assembly.
    High,
}

impl InterruptType {
    pub fn levels(&self) -> EnumSet<Self> {
        Self::Level1
            | Self::Level2
            | Self::Level3
            | Self::Level4
            | Self::Level5
            | Self::Level6
            | Self::Nmi
    }

    pub(crate) fn to_native(flags: EnumSet<Self>) -> u32 {
        let mut result = 0;

        for flag in flags {
            result |= u32::from(flag);
        }

        result
    }
}

impl From<InterruptType> for u32 {
    fn from(flag: InterruptType) -> Self {
        match flag {
            InterruptType::Level1 => esp_idf_sys::ESP_INTR_FLAG_LEVEL1,
            InterruptType::Level2 => esp_idf_sys::ESP_INTR_FLAG_LEVEL2,
            InterruptType::Level3 => esp_idf_sys::ESP_INTR_FLAG_LEVEL3,
            InterruptType::Level4 => esp_idf_sys::ESP_INTR_FLAG_LEVEL4,
            InterruptType::Level5 => esp_idf_sys::ESP_INTR_FLAG_LEVEL5,
            InterruptType::Level6 => esp_idf_sys::ESP_INTR_FLAG_LEVEL6,
            InterruptType::Nmi => esp_idf_sys::ESP_INTR_FLAG_NMI,
            InterruptType::Shared => esp_idf_sys::ESP_INTR_FLAG_SHARED,
            InterruptType::Edge => esp_idf_sys::ESP_INTR_FLAG_EDGE,
            InterruptType::Iram => esp_idf_sys::ESP_INTR_FLAG_IRAM,
            InterruptType::IntrDisabled => esp_idf_sys::ESP_INTR_FLAG_INTRDISABLED,
            InterruptType::LowMed => esp_idf_sys::ESP_INTR_FLAG_LOWMED,
            InterruptType::High => esp_idf_sys::ESP_INTR_FLAG_HIGH,
        }
    }
}

pub(crate) static CS: IsrCriticalSection = IsrCriticalSection::new();

/// Returns true if the currently active core is executing an ISR request
#[inline(always)]
#[link_section = ".iram1.interrupt_active"]
pub fn active() -> bool {
    unsafe { xPortInIsrContext() != 0 }
}

pub fn with_isr_yield_signal(cb: impl FnOnce()) -> bool {
    if !active() {
        panic!("with_isr_yield_signal() can only be called from an ISR context");
    }

    let mut signaled = false;

    let prev_yielder =
        unsafe { set_isr_yielder(Some((do_yield_signal, &mut signaled as *mut _ as _))) };

    cb();

    unsafe { set_isr_yielder(prev_yielder) };

    signaled
}

unsafe fn do_yield_signal(arg: *mut ()) {
    let signaled = arg.cast::<bool>().as_mut().unwrap();

    *signaled = true
}

#[allow(clippy::type_complexity)]
static mut ISR_YIELDER: Option<(unsafe fn(*mut ()), *mut ())> = None;

#[allow(clippy::type_complexity)]
#[inline(always)]
#[link_section = ".iram1.interrupt_get_isr_yielder"]
pub(crate) unsafe fn get_isr_yielder() -> Option<(unsafe fn(*mut ()), *mut ())> {
    if active() {
        free(|| {
            if let Some((func, arg)) = unsafe { ISR_YIELDER } {
                Some((func, arg))
            } else {
                None
            }
        })
    } else {
        None
    }
}

/// # Safety
///
/// This function should only be called from within an ISR handler, so as to set
/// a custom ISR yield function (e.g. when using the ESP-IDF timer service).
///
/// Thus, if some function further down the ISR call chain invokes `do_yield`,
/// the custom yield function set here will be called.
///
/// Users should not forget to call again `set_isr_yielder` at the end of the
/// ISR handler so as to reastore the yield function which was valid before the
/// ISR handler was invoked.
#[allow(clippy::type_complexity)]
#[inline(always)]
#[link_section = ".iram1.interrupt_set_isr_yielder"]
pub unsafe fn set_isr_yielder(
    yielder: Option<(unsafe fn(*mut ()), *mut ())>,
) -> Option<(unsafe fn(*mut ()), *mut ())> {
    if active() {
        free(|| {
            let previous = unsafe { ISR_YIELDER };

            unsafe { ISR_YIELDER = yielder };

            previous
        })
    } else {
        None
    }
}

/// A critical section allows the user to disable interrupts
#[cfg(any(esp32, esp32s2, esp32s3, esp32p4, esp32s31))]
pub struct IsrCriticalSection(core::cell::UnsafeCell<portMUX_TYPE>);

#[cfg(not(any(esp32, esp32s2, esp32s3, esp32p4, esp32s31)))]
pub struct IsrCriticalSection(core::marker::PhantomData<*const ()>);

#[cfg(not(any(esp32, esp32s2, esp32s3, esp32p4, esp32s31)))]
#[inline(always)]
#[link_section = ".iram1.interrupt_enter"]
fn enter(_cs: &IsrCriticalSection) {
    unsafe {
        vPortEnterCritical();
    }
}

#[cfg(any(esp32, esp32s2, esp32s3, esp32p4, esp32s31))]
#[inline(always)]
#[link_section = ".iram1.interrupt_enter"]
fn enter(cs: &IsrCriticalSection) {
    unsafe {
        xPortEnterCriticalTimeout(cs.0.get(), portMUX_NO_TIMEOUT);
    }
}

#[cfg(not(any(esp32, esp32s2, esp32s3, esp32p4, esp32s31)))]
#[inline(always)]
#[link_section = ".iram1.interrupt_exit"]
fn exit(_cs: &IsrCriticalSection) {
    unsafe {
        vPortExitCritical();
    }
}

#[cfg(any(esp32, esp32s2, esp32s3))]
#[inline(always)]
#[link_section = ".iram1.interrupt_exit"]
fn exit(cs: &IsrCriticalSection) {
    unsafe {
        vPortExitCritical(cs.0.get());
    }
}

#[cfg(any(esp32p4, esp32s31))]
#[inline(always)]
#[link_section = ".iram1.interrupt_exit"]
fn exit(cs: &IsrCriticalSection) {
    unsafe {
        vPortExitCriticalMultiCore(cs.0.get());
    }
}

impl IsrCriticalSection {
    /// Constructs a new `IsrCriticalSection` instance
    #[inline(always)]
    pub const fn new() -> Self {
        #[cfg(any(esp32, esp32s2, esp32s3, esp32p4, esp32s31))]
        let mux = core::cell::UnsafeCell::new(portMUX_TYPE {
            owner: portMUX_FREE_VAL,
            count: 0,
            #[cfg(esp_idf_freertos_portmux_debug)]
            lastLockedFn: b"(never locked)",
            #[cfg(esp_idf_freertos_portmux_debug)]
            lastLockedLine: -1,
        });

        #[cfg(not(any(esp32, esp32s2, esp32s3, esp32p4, esp32s31)))]
        let mux = core::marker::PhantomData;

        Self(mux)
    }

    /// Disables all interrupts for the lifetime of the returned guard instance.
    /// This method supports nesting in that is safe to be called multiple times.
    /// This method is also safe to call from ISR routines.
    ///
    /// NOTE: On dual-core esp32* chips, interrupts will be disabled only on one of
    /// the cores (the one where `IsrCriticalSection::enter` is called), while the other
    /// core will continue its execution. Moreover, if the same `IsrCriticalSection` instance
    /// is shared across multiple threads, where some of these happen to be scheduled on
    /// the second core (which has its interrupts enabled), the second core will then spinlock
    /// (busy-wait) in `IsrCriticalSection::enter`, until the first CPU releases the critical
    /// section and re-enables its interrupts. The second core will then - in turn - disable
    /// its interrupts and own the spinlock.
    ///
    /// For more information, refer to https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-guides/freertos-smp.html#critical-sections
    #[inline(always)]
    pub fn enter(&self) -> IsrCriticalSectionGuard<'_> {
        enter(self);

        IsrCriticalSectionGuard(self)
    }
}

impl Default for IsrCriticalSection {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Send for IsrCriticalSection {}
unsafe impl Sync for IsrCriticalSection {}

pub struct IsrCriticalSectionGuard<'a>(&'a IsrCriticalSection);

impl Drop for IsrCriticalSectionGuard<'_> {
    /// Drops the critical section guard thus potentially re-enabling
    /// al interrupts for the currently active core.
    ///
    /// Note that - due to the fact that calling `IsrCriticalSection::enter`
    /// multiple times on the same or multiple critical sections is supported -
    /// interrupts for the core will be re-enabled only when the last guard that
    /// disabled interrupts for the concrete core is dropped.
    #[inline(always)]
    fn drop(&mut self) {
        exit(self.0);
    }
}

/// Executes closure f in an interrupt-free context
#[inline(always)]
#[link_section = ".iram1.interrupt_free"]
pub fn free<R>(f: impl FnOnce() -> R) -> R {
    let _guard = CS.enter();

    f()
}

#[cfg(feature = "wake-from-isr")]
pub mod asynch {
    pub type HalIsrNotification = crate::task::asynch::Notification;
}

#[cfg(not(feature = "wake-from-isr"))]
pub mod asynch {
    use core::{
        cell::UnsafeCell,
        ffi::{c_void, CStr},
        future::Future,
        num::NonZeroU32,
        sync::atomic::{AtomicPtr, Ordering},
        task::{Context, Poll, Waker},
    };

    use esp_idf_sys::EspError;

    use ::log::info;

    use crate::{
        cpu::Core,
        delay,
        task::{asynch::Notification, CriticalSection},
    };

    use super::IsrCriticalSection;

    /// The HAL-global wake runner.
    /// You should use no more than 64 tasks with it.
    ///
    /// `*IsrNotification` instances use this wake runner when they are triggered from an ISR context.
    pub static HAL_ISR_REACTOR: IsrReactor<64> = IsrReactor::new(IsrReactorConfig::new());

    /// Wake runner configuration
    #[derive(Clone, Debug)]
    pub struct IsrReactorConfig {
        pub task_name: &'static CStr,
        pub task_stack_size: usize,
        pub task_priority: u8,
        pub task_pin_to_core: Option<Core>,
    }

    impl IsrReactorConfig {
        #[allow(clippy::manual_c_str_literals)]
        pub const fn new() -> Self {
            Self {
                task_name: unsafe { CStr::from_bytes_with_nul_unchecked(b"IsrReactor\0") },
                task_stack_size: 3084,
                task_priority: 11,
                task_pin_to_core: None,
            }
        }
    }

    impl Default for IsrReactorConfig {
        fn default() -> Self {
            Self::new()
        }
    }

    /// IsrReactor is a utility allowing `Waker` instances to be awoken fron an ISR context.
    ///
    /// General problem:
    /// In an interrupt, using Waker instances coming from generic executors is impossible,
    /// because these are not designed with an ISR-safety in mind.
    ///
    /// Waking a waker means that its task would be scheduled on the executor queue, which might involve
    /// allocation, and/or synchronization primitives which are not safe to use from an ISR context.
    ///
    /// Similarly, dropping a waker might also drop the executor task, resulting in a deallocation, which is also
    /// not safe in an ISR context.
    ///
    /// These problems are alleviated by replacing direct `waker.wake()` calls to `WakerRunner::schedule(waker)`.
    /// What `IsrReactor::schedule` does is to push the waker into a bounded queue and then notify a hidden FreeRTOS task.
    /// Once the FreeRTOS task gets awoken, it wakes all wakers scheduled on the bounded queue and empties the queue.
    pub struct IsrReactor<const N: usize> {
        wakers_cs: IsrCriticalSection,
        wakers: UnsafeCell<heapless::Deque<Waker, N>>,
        task_cs: CriticalSection,
        task: AtomicPtr<crate::sys::tskTaskControlBlock>,
        task_config: IsrReactorConfig,
    }

    impl<const N: usize> IsrReactor<N> {
        /// Create a new `IsrReactor` instance.
        pub const fn new(config: IsrReactorConfig) -> Self {
            Self {
                wakers_cs: IsrCriticalSection::new(),
                wakers: UnsafeCell::new(heapless::Deque::new()),
                task_cs: CriticalSection::new(),
                task: AtomicPtr::new(core::ptr::null_mut()),
                task_config: config,
            }
        }

        /// Returns `true` if the wake runner is started.
        pub fn is_started(&self) -> bool {
            !self.task.load(Ordering::SeqCst).is_null()
        }

        /// Starts the wake runner. Returns `false` if it had been already started.
        pub fn start(&'static self) -> Result<bool, EspError> {
            let _guard = self.task_cs.enter();

            if self.task.load(Ordering::SeqCst).is_null() {
                let task = unsafe {
                    crate::task::create(
                        Self::task_run,
                        self.task_config.task_name,
                        self.task_config.task_stack_size,
                        self as *const _ as *const c_void as *mut _,
                        self.task_config.task_priority,
                        self.task_config.task_pin_to_core,
                    )?
                };

                self.task.store(task as _, Ordering::SeqCst);

                info!("IsrReactor {:?} started.", self.task_config.task_name);

                Ok(true)
            } else {
                Ok(false)
            }
        }

        /// Stops the wake runner. Returns `false` if it had been already stopped.
        pub fn stop(&self) -> bool {
            let _guard = self.task_cs.enter();

            let task = self.task.swap(core::ptr::null_mut(), Ordering::SeqCst);

            if !task.is_null() {
                unsafe {
                    crate::task::destroy(task as _);
                }

                info!("IsrReactor {:?} stopped.", self.task_config.task_name);

                true
            } else {
                false
            }
        }

        /// Schedules a waker to be awoken by the hidden FreeRTOS task running in the background.
        /// If not called from within an ISR context, calls `waker.wake()` directly instead of scheduling the waker.
        /// NOTE: If the wake runner is not started yet, scheduling fron an ISR content would fail silently.
        ///
        /// This and only this method is safe to call from an ISR context.
        pub fn schedule(&self, waker: Waker) {
            if super::active() {
                self.wakers(|wakers| {
                    let earlier_waker = wakers.iter_mut().find(|a_waker| a_waker.will_wake(&waker));

                    if let Some(earlier_waker) = earlier_waker {
                        *earlier_waker = waker;
                    } else if wakers.push_back(waker).is_err() {
                        panic!("IsrReactor queue overflow");
                    }

                    let task = self.task.load(Ordering::SeqCst);

                    if !task.is_null() {
                        unsafe {
                            crate::task::notify_and_yield(task as _, NonZeroU32::new(1).unwrap());
                        }
                    }
                })
            } else {
                waker.wake();
            }
        }

        fn run(&self) {
            loop {
                loop {
                    let waker = self.wakers(|wakers| wakers.pop_front());

                    if let Some(waker) = waker {
                        waker.wake();
                    } else {
                        break;
                    }
                }

                crate::task::wait_notification(delay::BLOCK);
            }
        }

        fn wakers<F: FnOnce(&mut heapless::Deque<Waker, N>) -> R, R>(&self, f: F) -> R {
            // if super::active() {
            //     let wakers = unsafe { self.wakers.get().as_mut().unwrap() };

            //     f(wakers)
            // } else {
            let _guard = self.wakers_cs.enter();

            let wakers = unsafe { self.wakers.get().as_mut().unwrap() };

            f(wakers)
            // }
        }

        extern "C" fn task_run(ctx: *mut c_void) {
            let this =
                unsafe { (ctx as *mut IsrReactor<N> as *const IsrReactor<N>).as_ref() }.unwrap();

            this.run();
        }
    }

    impl<const N: usize> Drop for IsrReactor<N> {
        fn drop(&mut self) {
            self.stop();
        }
    }

    unsafe impl<const N: usize> Send for IsrReactor<N> {}
    unsafe impl<const N: usize> Sync for IsrReactor<N> {}

    /// Single-slot lock-free signaling primitive supporting signalling with a `u32` bit-set.
    ///
    /// A variation of the `Notification` HAL primitive which is however safe to be notified from an ISR context.
    ///
    /// It is useful for sending data between an ISR routine (or a regular task context) and an async task when the
    /// receiver only cares about the latest data, and therefore it's fine to "lose" messages.
    /// This is often the case for "state" updates.
    ///
    /// The sending part of the primitive is non-blocking and ISR-safe, so it can be called from anywhere.
    ///
    /// Similar in spirit to the ESP-IDF FreeRTOS task notifications in that it is light-weight and operates on bit-sets,
    /// but for synchronization between an asynchronous task, and another one, which might be blocking or asynchronous.
    pub struct IsrNotification<const N: usize> {
        inner: Notification,
        reactor: &'static IsrReactor<N>,
    }

    impl<const N: usize> IsrNotification<N> {
        /// Creates a new `IsrNotification`.
        /// This method is safe to call from an ISR context, yet such use cases should not normally occur in practice.
        pub const fn new(reactor: &'static IsrReactor<N>) -> Self {
            Self {
                inner: Notification::new(),
                reactor,
            }
        }

        /// Marks the least significant bit (bit 0) in this `IsrNotification` as nofified.
        /// This method is safe to call from an ISR context.
        /// Returns `true` if there was a registered waker which got awoken.
        pub fn notify_lsb(&self) -> bool {
            self.notify(NonZeroU32::new(1).unwrap())
        }

        /// Marks the supplied bits in this `IsrNotification` as notified.
        /// This method is safe to call from an ISR context.
        /// Returns `true` if there was a registered waker which got awoken.
        pub fn notify(&self, bits: NonZeroU32) -> bool {
            if let Some(waker) = self.inner.notify_waker(bits) {
                self.reactor.schedule(waker);

                true
            } else {
                false
            }
        }

        /// Clears the state of this notification by removing any registered waker and setting all bits to 0.
        /// This method is NOT safe to call from an ISR context.
        pub fn reset(&self) {
            self.inner.reset();
        }

        /// Future that completes when this `IsrNotification` has been notified.
        /// This method is NOT safe to call from an ISR context.
        #[allow(unused)]
        pub fn wait(&self) -> impl Future<Output = NonZeroU32> + '_ {
            self.reactor.start().unwrap();

            self.inner.wait()
        }

        /// Non-blocking method to check whether this notification has been notified.
        /// This method is NOT safe to call from an ISR context.
        pub fn poll_wait(&self, cx: &Context<'_>) -> Poll<NonZeroU32> {
            self.reactor.start().unwrap();

            self.inner.poll_wait(cx)
        }
    }

    /// Single-slot lock-free signaling primitive supporting signalling with a `u32` bit-set.
    ///
    /// A variation of the `IsrNotification` HAL primitive which is however safe to be notified from an ISR context.
    /// The difference between this primitive and `IsrNotification` is that this one is hard-wired to the
    /// global HAL wake runner (`HAL_WAKE_RUNNER`) and is thus occupying less space.
    ///
    /// It is useful for sending data between an ISR routine (or a regular task context) and an async task when the
    /// receiver only cares about the latest data, and therefore it's fine to "lose" messages.
    /// This is often the case for "state" updates.
    ///
    /// The sending part of the primitive is non-blocking and ISR-safe, so it can be called from anywhere.
    ///
    /// Similar in spirit to the ESP-IDF FreeRTOS task notifications in that it is light-weight and operates on bit-sets,
    /// but for synchronization between an asynchronous task, and another one, which might be blocking or asynchronous.
    pub struct HalIsrNotification {
        inner: Notification,
    }

    impl Default for HalIsrNotification {
        fn default() -> Self {
            Self::new()
        }
    }

    impl HalIsrNotification {
        /// Creates a new `HalIsrNotification`.
        /// This method is safe to call from an ISR context, yet such use cases should not normally occur in practice.
        pub const fn new() -> Self {
            Self {
                inner: Notification::new(),
            }
        }

        /// Marks the least significant bit (bit 0) in this `IsrNotification` as nofified.
        /// This method is safe to call from an ISR context.
        /// Returns `true` if there was a registered waker which got awoken.
        pub fn notify_lsb(&self) -> bool {
            self.notify(NonZeroU32::new(1).unwrap())
        }

        /// Marks the supplied bits in this `HalIsrNotification` as notified.
        /// This method is safe to call from an ISR context.
        /// Returns `true` if there was a registered waker which got awoken.
        pub fn notify(&self, bits: NonZeroU32) -> bool {
            if let Some(waker) = self.inner.notify_waker(bits) {
                HAL_ISR_REACTOR.schedule(waker);

                true
            } else {
                false
            }
        }

        /// Clears the state of this notification by removing any registered waker and setting all bits to 0.
        /// This method is NOT safe to call from an ISR context.
        pub fn reset(&self) {
            self.inner.reset();
        }

        /// Future that completes when this `HalIsrNotification` has been notified.
        /// This method is NOT safe to call from an ISR context.
        #[allow(unused)]
        pub fn wait(&self) -> impl Future<Output = NonZeroU32> + '_ {
            HAL_ISR_REACTOR.start().unwrap();

            self.inner.wait()
        }

        /// Non-blocking method to check whether this notification has been notified.
        /// This method is NOT safe to call from an ISR context.
        pub fn poll_wait(&self, cx: &Context<'_>) -> Poll<NonZeroU32> {
            HAL_ISR_REACTOR.start().unwrap();

            self.inner.poll_wait(cx)
        }
    }
}

#[cfg(feature = "embassy-sync")]
pub mod embassy_sync {
    use embassy_sync::blocking_mutex::raw::RawMutex;

    /// A mutex that allows borrowing data across executors and interrupts.
    ///
    /// # Safety
    ///
    /// This mutex is safe to share between different executors and interrupts.
    pub struct IsrRawMutex(());

    unsafe impl Send for IsrRawMutex {}
    unsafe impl Sync for IsrRawMutex {}

    impl IsrRawMutex {
        /// Create a new `IsrRawMutex`.
        pub const fn new() -> Self {
            Self(())
        }
    }

    impl Default for IsrRawMutex {
        fn default() -> Self {
            Self::new()
        }
    }

    unsafe impl RawMutex for IsrRawMutex {
        const INIT: Self = Self::new();

        fn lock<R>(&self, f: impl FnOnce() -> R) -> R {
            super::free(f)
        }
    }
}
