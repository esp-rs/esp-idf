//! High resolution hardware timer based task scheduling
//!
//! Although FreeRTOS provides software timers, these timers have a few
//! limitations:
//!
//! - Maximum resolution is equal to RTOS tick period
//! - Timer callbacks are dispatched from a low-priority task
//!
//! EspTimer is a set of APIs that provides one-shot and periodic timers,
//! microsecond time resolution, and 52-bit range.

use core::num::NonZeroU32;
use core::time::Duration;
use core::{ffi, ptr};

extern crate alloc;
use alloc::boxed::Box;
use alloc::sync::Arc;

use esp_idf_hal::task::asynch::Notification;

use crate::sys::*;

use ::log::debug;

#[cfg(esp_idf_esp_timer_supports_isr_dispatch_method)]
pub use isr::*;

use crate::handle::RawHandle;

struct UnsafeCallback<'a>(*mut Box<dyn FnMut() + Send + 'a>);

impl<'a> UnsafeCallback<'a> {
    fn from(boxed: &mut Box<dyn FnMut() + Send + 'a>) -> Self {
        Self(boxed)
    }

    unsafe fn from_ptr(ptr: *mut ffi::c_void) -> Self {
        Self(ptr as *mut _)
    }

    fn as_ptr(&self) -> *mut ffi::c_void {
        self.0 as *mut _
    }

    unsafe fn call(&self) {
        let reference = self.0.as_mut().unwrap();

        (reference)();
    }
}

pub struct EspTimer<'a> {
    handle: esp_timer_handle_t,
    _callback: Box<dyn FnMut() + Send + 'a>,
}

impl EspTimer<'_> {
    pub fn is_scheduled(&self) -> Result<bool, EspError> {
        Ok(unsafe { esp_timer_is_active(self.handle) })
    }

    pub fn cancel(&self) -> Result<bool, EspError> {
        let res = unsafe { esp_timer_stop(self.handle) };

        Ok(res != ESP_OK)
    }

    pub fn after(&self, duration: Duration) -> Result<(), EspError> {
        self.cancel()?;

        esp!(unsafe { esp_timer_start_once(self.handle, duration.as_micros() as _) })?;

        Ok(())
    }

    pub fn every(&self, duration: Duration) -> Result<(), EspError> {
        self.cancel()?;

        esp!(unsafe { esp_timer_start_periodic(self.handle, duration.as_micros() as _) })?;

        Ok(())
    }

    extern "C" fn handle(arg: *mut ffi::c_void) {
        if crate::hal::interrupt::active() {
            #[cfg(esp_idf_esp_timer_supports_isr_dispatch_method)]
            {
                let signaled = crate::hal::interrupt::with_isr_yield_signal(move || unsafe {
                    UnsafeCallback::from_ptr(arg).call();
                });

                if signaled {
                    unsafe {
                        crate::sys::esp_timer_isr_dispatch_need_yield();
                    }
                }
            }

            #[cfg(not(esp_idf_esp_timer_supports_isr_dispatch_method))]
            {
                unreachable!();
            }
        } else {
            unsafe {
                UnsafeCallback::from_ptr(arg).call();
            }
        }
    }
}

unsafe impl Send for EspTimer<'_> {}

impl Drop for EspTimer<'_> {
    fn drop(&mut self) {
        self.cancel().unwrap();

        while unsafe { esp_timer_delete(self.handle) } != ESP_OK {
            // Timer is still running, busy-loop
        }

        debug!("Timer dropped");
    }
}

impl RawHandle for EspTimer<'_> {
    type Handle = esp_timer_handle_t;

    fn handle(&self) -> Self::Handle {
        self.handle
    }
}

pub struct EspAsyncTimer {
    timer: EspTimer<'static>,
    notification: Arc<Notification>,
}

impl EspAsyncTimer {
    pub async fn after(&mut self, duration: Duration) -> Result<(), EspError> {
        self.timer.cancel()?;

        self.notification.reset();
        self.timer.after(duration)?;

        self.notification.wait().await;

        Ok(())
    }

    pub fn every(&mut self, duration: Duration) -> Result<&'_ mut Self, EspError> {
        self.timer.cancel()?;

        self.notification.reset();
        self.timer.every(duration)?;

        Ok(self)
    }

    pub async fn tick(&mut self) -> Result<(), EspError> {
        self.notification.wait().await;

        Ok(())
    }
}

impl embedded_hal_async::delay::DelayNs for EspAsyncTimer {
    async fn delay_ns(&mut self, ns: u32) {
        EspAsyncTimer::after(self, Duration::from_micros(ns as _))
            .await
            .unwrap();
    }

    async fn delay_ms(&mut self, ms: u32) {
        EspAsyncTimer::after(self, Duration::from_millis(ms as _))
            .await
            .unwrap();
    }
}

pub trait EspTimerServiceType {
    fn is_isr() -> bool;
}

#[derive(Clone, Debug)]
pub struct Task;

impl EspTimerServiceType for Task {
    fn is_isr() -> bool {
        false
    }
}

pub struct EspTimerService<T>(T)
where
    T: EspTimerServiceType;

impl<T> EspTimerService<T>
where
    T: EspTimerServiceType,
{
    pub fn now(&self) -> Duration {
        Duration::from_micros(unsafe { esp_timer_get_time() as _ })
    }

    pub fn timer<F>(&self, callback: F) -> Result<EspTimer<'static>, EspError>
    where
        F: FnMut() + Send + 'static,
    {
        self.internal_timer(callback, false)
    }

    /// Same as `timer` but does not wake the device from light sleep.
    pub fn timer_nowake<F>(&self, callback: F) -> Result<EspTimer<'static>, EspError>
    where
        F: FnMut() + Send + 'static,
    {
        self.internal_timer(callback, true)
    }

    pub fn timer_async(&self) -> Result<EspAsyncTimer, EspError> {
        self.internal_timer_async(false)
    }

    /// Same as `timer_async` but does not wake the device from light sleep.
    pub fn timer_async_nowake(&self) -> Result<EspAsyncTimer, EspError> {
        self.internal_timer_async(true)
    }

    /// # Safety
    ///
    /// This method - in contrast to method `timer` - allows the user to pass
    /// a non-static callback/closure. This enables users to borrow
    /// - in the closure - variables that live on the stack - or more generally - in the same
    ///   scope where the service is created.
    ///
    /// HOWEVER: care should be taken NOT to call `core::mem::forget()` on the service,
    /// as that would immediately lead to an UB (crash).
    /// Also note that forgetting the service might happen with `Rc` and `Arc`
    /// when circular references are introduced: <https://github.com/rust-lang/rust/issues/24456>
    ///
    /// The reason is that the closure is actually sent to a hidden ESP IDF thread.
    /// This means that if the service is forgotten, Rust is free to e.g. unwind the stack
    /// and the closure now owned by this other thread will end up with references to variables that no longer exist.
    ///
    /// The destructor of the service takes care - prior to the service being dropped and e.g.
    /// the stack being unwind - to remove the closure from the hidden thread and destroy it.
    /// Unfortunately, when the service is forgotten, the un-subscription does not happen
    /// and invalid references are left dangling.
    ///
    /// This "local borrowing" will only be possible to express in a safe way once/if `!Leak` types
    /// are introduced to Rust (i.e. the impossibility to "forget" a type and thus not call its destructor).
    pub unsafe fn timer_nonstatic<'a, F>(&self, callback: F) -> Result<EspTimer<'a>, EspError>
    where
        F: FnMut() + Send + 'a,
    {
        self.internal_timer(callback, false)
    }

    /// # Safety
    ///
    /// Same as `timer_nonstatic` but does not wake the device from light sleep.
    pub unsafe fn timer_nonstatic_nowake<'a, F>(
        &self,
        callback: F,
    ) -> Result<EspTimer<'a>, EspError>
    where
        F: FnMut() + Send + 'a,
    {
        self.internal_timer(callback, true)
    }

    fn internal_timer<'a, F>(
        &self,
        callback: F,
        skip_unhandled_events: bool,
    ) -> Result<EspTimer<'a>, EspError>
    where
        F: FnMut() + Send + 'a,
    {
        let mut handle: esp_timer_handle_t = ptr::null_mut();

        let boxed_callback: Box<dyn FnMut() + Send + 'a> = Box::new(callback);

        let mut callback = Box::new(boxed_callback);
        let unsafe_callback = UnsafeCallback::from(&mut callback);

        #[cfg(esp_idf_esp_timer_supports_isr_dispatch_method)]
        let dispatch_method = if T::is_isr() {
            esp_timer_dispatch_t_ESP_TIMER_ISR
        } else {
            esp_timer_dispatch_t_ESP_TIMER_TASK
        };

        #[cfg(not(esp_idf_esp_timer_supports_isr_dispatch_method))]
        let dispatch_method = esp_timer_dispatch_t_ESP_TIMER_TASK;

        esp!(unsafe {
            esp_timer_create(
                &esp_timer_create_args_t {
                    callback: Some(EspTimer::handle),
                    name: b"rust\0" as *const _ as *const _, // TODO
                    arg: unsafe_callback.as_ptr(),
                    dispatch_method,
                    skip_unhandled_events,
                },
                &mut handle as *mut _,
            )
        })?;

        Ok(EspTimer {
            handle,
            _callback: callback,
        })
    }

    fn internal_timer_async(&self, skip_unhandled_events: bool) -> Result<EspAsyncTimer, EspError> {
        let notification = Arc::new(Notification::new());

        let timer = {
            let notification = Arc::downgrade(&notification);

            self.internal_timer(
                move || {
                    if let Some(notification) = notification.upgrade() {
                        notification.notify(NonZeroU32::new(1).unwrap());
                    }
                },
                skip_unhandled_events,
            )?
        };

        Ok(EspAsyncTimer {
            timer,
            notification,
        })
    }
}

pub type EspTaskTimerService = EspTimerService<Task>;

impl EspTimerService<Task> {
    pub fn new() -> Result<Self, EspError> {
        Ok(Self(Task))
    }
}

impl<T> Clone for EspTimerService<T>
where
    T: EspTimerServiceType + Clone,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[cfg(esp_idf_esp_timer_supports_isr_dispatch_method)]
mod isr {
    use crate::sys::EspError;

    #[derive(Clone, Debug)]
    pub struct ISR;

    impl super::EspTimerServiceType for ISR {
        fn is_isr() -> bool {
            true
        }
    }

    pub type EspISRTimerService = super::EspTimerService<ISR>;

    impl EspISRTimerService {
        /// # Safety
        /// TODO
        pub unsafe fn new() -> Result<Self, EspError> {
            Ok(Self(ISR))
        }
    }
}

/// This module is used to provide a time driver for the `embassy-time` crate.
///
/// The minimum provided resolution is ~ 20-30us when the CPU is at top speed of 240MHz
/// (https://docs.espressif.com/projects/esp-idf/en/v5.4/esp32/api-reference/system/esp_timer.html#timeout-value-limits)
///
/// The tick-rate is 1MHz (i.e. 1 tick is 1us).
#[cfg(feature = "embassy-time-driver")]
pub mod embassy_time_driver {
    use core::cell::RefCell;
    use core::task::Waker;

    use ::embassy_time_driver::Driver;
    use embassy_time_queue_utils::Queue;

    use crate::private::mutex::Mutex;
    use crate::timer::*;

    struct EspDriverInner {
        queue: embassy_time_queue_utils::Queue,
        timer: Option<EspTimer<'static>>,
    }

    impl EspDriverInner {
        fn now() -> u64 {
            unsafe { esp_timer_get_time() as _ }
        }

        fn schedule_next_expiration(&mut self) {
            /// The longest the ESP-IDF timer is ever armed for.
            ///
            /// `esp_timer` adds the timeout to its own "now", and its HW backends convert
            /// the resulting deadline to ticks and compare it as a signed 64-bit value
            /// (and the systimer counter is only 52 bits wide). Far-away deadlines thus
            /// overflow there and fire immediately, so instead the timer is armed for at
            /// most this long, and later deadlines are reached in several steps: each
            /// premature expiry just re-arms the timer via `schedule_next_expiration`.
            const MAX_ARM_US: u64 = 24 * 60 * 60 * 1000 * 1000;

            let timer = self.timer.as_mut().unwrap();

            loop {
                let now = Self::now();
                let next_at = self.queue.next_expiration(now);

                if now < next_at {
                    // `u64::MAX` means that the queue is empty
                    if next_at != u64::MAX {
                        let after = (next_at - now).min(MAX_ARM_US);

                        timer.after(Duration::from_micros(after)).unwrap();
                    }

                    break;
                }
            }
        }
    }

    struct EspDriver {
        inner: Mutex<RefCell<EspDriverInner>>,
    }

    impl EspDriver {
        const fn new() -> Self {
            Self {
                inner: Mutex::new(RefCell::new(EspDriverInner {
                    queue: Queue::new(),
                    timer: None,
                })),
            }
        }
    }

    unsafe impl Send for EspDriver {}
    unsafe impl Sync for EspDriver {}

    impl Driver for EspDriver {
        fn now(&self) -> u64 {
            EspDriverInner::now()
        }

        fn schedule_wake(&self, at: u64, waker: &Waker) {
            let service = EspTimerService::<Task>::new().unwrap();

            let guard = self.inner.lock();
            let mut inner = guard.borrow_mut();

            if inner.timer.is_none() {
                // Driver is always statically allocated, so this is safe
                let static_self: &'static Self = unsafe { core::mem::transmute(self) };

                inner.timer = Some(
                    service
                        .timer(move || {
                            static_self
                                .inner
                                .lock()
                                .borrow_mut()
                                .schedule_next_expiration()
                        })
                        .unwrap(),
                );
            }

            if inner.queue.schedule_wake(at, waker) {
                inner.schedule_next_expiration();
            }
        }
    }

    pub type LinkWorkaround = [*mut (); 2];

    #[used]
    static mut __INTERNAL_REFERENCE: LinkWorkaround = [
        _embassy_time_now as *mut _,
        _embassy_time_schedule_wake as *mut _,
    ];

    pub fn link() -> LinkWorkaround {
        unsafe { __INTERNAL_REFERENCE }
    }

    ::embassy_time_driver::time_driver_impl!(static DRIVER: EspDriver = EspDriver::new());
}
