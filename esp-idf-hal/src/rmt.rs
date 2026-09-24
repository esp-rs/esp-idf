//! RMT (Remote Control) peripheral support.
//!
//! The RMT (Remote Control Transceiver) peripheral was designed to act as an infrared transceiver.
//! However, due to the flexibility of its data format, RMT can be extended to a versatile and
//! general-purpose transceiver, transmitting or receiving many other types of signals.
//! From the perspective of network layering, the RMT hardware contains both physical and data
//! link layers. The physical layer defines the communication media and bit signal representation.
//! The data link layer defines the format of an RMT frame. The minimal data unit in the frame is
//! called the RMT symbol, which is represented by [`Symbol`] in the driver.
//!
//! ESP32 contains multiple channels in the RMT peripheral. Each channel can be independently
//! configured as either transmitter or receiver.
//!
//! Typically, the RMT peripheral can be used in the following scenarios:
//! - Transmit or receive infrared signals, with any IR protocols, e.g., NEC
//! - General-purpose sequence generator
//! - Transmit signals in a hardware-controlled loop, with a finite or infinite number of times
//! - Multi-channel simultaneous transmission
//! - Modulate the carrier to the output signal or demodulate the carrier from the input signal
//!
//! # Driver redesign in ESP-IDF 5.0
//!
//! In ESP-IDF 5.0, the [RMT API was redesigned] to simplify and unify the usage of the
//! RMT peripheral.
//!
//! It is recommended to use the new API, but for now the old API is available through
//! the `rmt-legacy` feature. The ESP-IDF 6.0 release will remove support for the legacy API.
//!
//! [RMT API was redesigned]: https://docs.espressif.com/projects/esp-idf/en/stable/esp32/migration-guides/release-5.x/5.0/peripherals.html#rmt-driver
pub mod config;
pub mod encoder;

mod tx_channel;
pub use tx_channel::*;

mod tx_queue;
pub use tx_queue::*;

mod rx_channel;
pub use rx_channel::*;

mod sync_manager;
pub use sync_manager::*;
mod pulse;
pub use pulse::*;

use core::time::Duration;
use core::{fmt, ptr};

use esp_idf_sys::*;

use crate::rmt::config::CarrierConfig;
use crate::units::Hertz;

#[cfg(esp_idf_version_at_least_6_0_0)]
#[allow(non_camel_case_types)]
type rmt_carrier_config_t__bindgen_ty_1 = rmt_carrier_config_t_extra_rmt_carrier_config_flags;

/// A [`Symbol`] constists of two [`Pulse`]s, where each pulse defines a level of the pin (high or low)
/// and a duration ([`PulseTicks`]).
///
/// The `Pulse`s can be the same level (e.g., both high) and must not necessarily be different levels.
///
/// This is just a newtype over the IDF's `rmt_item32_t` or `rmt_symbol_word_t` type.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Symbol(rmt_symbol_word_t);

impl Symbol {
    /// Create a symbol from a pair of half-cycles.
    #[must_use]
    pub fn new(level0: Pulse, level1: Pulse) -> Self {
        let item = rmt_symbol_word_t { val: 0 };
        let mut this = Self(item);
        this.update(level0, level1);
        this
    }

    /// Constructs a symbol from the given levels and durations.
    ///
    /// This is a convenience function that combines [`Pulse::new_with_duration`] and [`Symbol::new`].
    pub fn new_with(
        resolution: Hertz,
        level0: PinState,
        duration0: Duration,
        level1: PinState,
        duration1: Duration,
    ) -> Result<Self, EspError> {
        Ok(Self::new(
            Pulse::new_with_duration(resolution, level0, duration0)?,
            Pulse::new_with_duration(resolution, level1, duration1)?,
        ))
    }

    /// Constructs a new symbol where the duration is split evenly between the two levels.
    ///
    /// It is guaranteed that the combined duration of the two levels adds up to the given duration.
    /// If the duration is odd, the first level might be a bit shorter than the second level.
    pub fn new_half_split(
        resolution: Hertz,
        level0: PinState,
        level1: PinState,
        duration: Duration,
    ) -> Result<Self, EspError> {
        let first_half_duration = duration / 2;
        // This ensures that the two halves always add up to the original duration,
        // even if the duration is odd.
        let second_half_duration = duration - first_half_duration;

        Ok(Self::new(
            Pulse::new_with_duration(resolution, level0, first_half_duration)?,
            Pulse::new_with_duration(resolution, level1, second_half_duration)?,
        ))
    }

    /// Repeats the symbol to have the returned sequence of symbols last for
    /// exactly the given duration.
    ///
    /// There is an upper limit for how long a single symbol can be ([`PulseTicks::max`]).
    /// To create a symbol that lasts longer than that, it is split into multiple symbols.
    /// The returned iterator yields as many symbols as necessary to reach the
    /// desired duration.
    ///
    /// If the given duration is not a multiple of the symbol duration,
    /// the last symbol will be adjusted to occupy the remaining time.
    ///
    /// # Panics
    ///
    /// If `self` has a duration of zero.
    pub fn repeat_for(&self, resolution: Hertz, duration: Duration) -> impl Iterator<Item = Self> {
        // Calculate the maximum allowed duration for a single symbol consisting of two pulses:
        // let max_duration = PulseTicks::max().duration(resolution) * 2;

        let symbol_duration = self.duration(resolution);

        // Handle edge-case to prevent an infinite loop:
        if symbol_duration.is_zero() {
            panic!("Cannot repeat a symbol with zero duration for {duration:?}");
        }

        let count = duration.as_nanos() / symbol_duration.as_nanos();
        let remainder =
            Duration::from_nanos((duration.as_nanos() % symbol_duration.as_nanos()) as u64);

        let last_symbol = {
            if remainder.is_zero() {
                None
            } else {
                let duration0 = self.level0().ticks.duration(resolution).min(remainder);
                let duration1 = remainder.saturating_sub(duration0);

                Some(
                    Self::new_with(
                        resolution,
                        self.level0().pin_state,
                        duration0,
                        self.level1().pin_state,
                        duration1,
                    )
                    .unwrap(),
                )
            }
        };

        core::iter::repeat_n(*self, count as usize).chain(core::iter::once(last_symbol).flatten())
    }

    #[must_use]
    fn symbol_word_to_pulse(word: &rmt_symbol_word_t) -> (Pulse, Pulse) {
        let inner = unsafe { &word.__bindgen_anon_1 };
        (
            Pulse::new(
                (inner.level0() as u32).into(),
                PulseTicks::new(inner.duration0()).unwrap(),
            ),
            Pulse::new(
                (inner.level1() as u32).into(),
                PulseTicks::new(inner.duration1()).unwrap(),
            ),
        )
    }

    /// Returns the first half-cycle (pulse) of this symbol.
    #[must_use]
    pub fn level0(&self) -> Pulse {
        Self::symbol_word_to_pulse(&self.0).0
    }

    /// Returns the second half-cycle (pulse) of this symbol.
    #[must_use]
    pub fn level1(&self) -> Pulse {
        Self::symbol_word_to_pulse(&self.0).1
    }

    /// Returns the combined duration of both half-cycles of this symbol.
    pub fn duration(&self, resolution: Hertz) -> Duration {
        self.level0().ticks.duration(resolution) + self.level1().ticks.duration(resolution)
    }

    /// Mutate this symbol to store a different pair of half-cycles.
    pub fn update(&mut self, level0: Pulse, level1: Pulse) {
        // SAFETY: We're overriding all 32 bits, so it doesn't matter what was here before.
        let inner = unsafe { &mut self.0.__bindgen_anon_1 };
        inner.set_level0(level0.pin_state as u16);
        inner.set_duration0(level0.ticks.ticks());
        inner.set_level1(level1.pin_state as u16);
        inner.set_duration1(level1.ticks.ticks());
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Symbol")
            .field("level0", &self.level0())
            .field("level1", &self.level1())
            .finish()
    }
}

impl Default for Symbol {
    fn default() -> Self {
        Self::new(Default::default(), Default::default())
    }
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        (self.level0(), self.level1()) == (other.level0(), other.level1())
    }
}

impl Eq for Symbol {}

impl From<rmt_symbol_word_t> for Symbol {
    fn from(value: rmt_symbol_word_t) -> Self {
        Self(value)
    }
}

impl From<Symbol> for rmt_symbol_word_t {
    fn from(value: Symbol) -> Self {
        value.0
    }
}

/// A small helper function to assert that the current code is not running in an ISR.
///
/// This is useful for functions that must not be called from an ISR context.
fn assert_not_in_isr() {
    if crate::interrupt::active() {
        panic!("This function cannot be called from an ISR");
    }
}

/// Type of RMT TX done event data.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct TxDoneEventData {
    /// The number of symbols ([`Symbol`]) that have been transmitted.
    ///
    /// This also reflects the size of the encoding artifacts. Please
    /// note, this value accounts for the `EOF` symbol as well, which
    /// is automatically appended by the driver to mark the end of one
    /// transaction.
    pub num_symbols: usize,
}

impl From<rmt_tx_done_event_data_t> for TxDoneEventData {
    fn from(data: rmt_tx_done_event_data_t) -> Self {
        Self {
            num_symbols: data.num_symbols,
        }
    }
}

/// Type of RMT RX done event data.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RxDoneEventData {
    /// Pointer to the received RMT symbols.
    ///
    /// These symbols are saved in the internal `buffer` of the `RxChannelDriver`.
    /// You are not allowed to free this buffer.
    ///
    /// If the partial receive feature is enabled, then the buffer will be
    /// used as a "second level buffer", where its content can be overwritten by
    /// data coming in afterwards. In this case, you should copy the received data to
    /// another place if you want to keep it or process it later.
    pub received_symbols: *mut Symbol,
    /// The number of received RMT symbols.
    ///
    /// This value will never be larger than the buffer size of the buffer passed to the
    /// receive function.
    ///
    /// If the buffer is not sufficient to accomodate all the received RMT symbols, the
    /// driver only keeps the maximum number of symbols that the buffer can hold, and excess
    /// symbols are discarded or ignored.
    pub num_symbols: usize,
    /// Indicates whether the current received data is the last part of the transaction.
    ///
    /// This is useful for when [`ReceiveConfig::enable_partial_rx`] is enabled,
    /// and the data is received in parts.
    ///
    /// For receives where partial rx is not enabled, this field will always be `true`.
    ///
    /// [`ReceiveConfig::enable_partial_rx`]: crate::rmt::config::ReceiveConfig::enable_partial_rx
    #[cfg(esp_idf_version_at_least_5_3_0)]
    pub is_last: bool,
}

impl RxDoneEventData {
    /// Returns the received symbols as a slice.
    ///
    /// # Safety
    ///
    /// Both the pointer and the length must be valid.
    /// If these haven't been changed from the values provided by the driver,
    /// this should be the case **in** the ISR callback.
    pub unsafe fn as_slice(&self) -> &[Symbol] {
        core::slice::from_raw_parts(self.received_symbols, self.num_symbols)
    }
}

impl From<rmt_rx_done_event_data_t> for RxDoneEventData {
    fn from(data: rmt_rx_done_event_data_t) -> Self {
        Self {
            received_symbols: data.received_symbols as *mut Symbol,
            num_symbols: data.num_symbols,
            #[cfg(esp_idf_version_at_least_5_3_0)]
            is_last: data.flags.is_last() != 0,
        }
    }
}

/// A trait implemented by an RMT channel that provides common functionality.
pub trait RmtChannel {
    /// Returns the underlying `rmt_channel_handle_t`.
    fn handle(&self) -> rmt_channel_handle_t;

    /// Returns whether the channel is currently enabled.
    #[must_use]
    fn is_enabled(&self) -> bool;

    /// Sets the internal `is_enabled` flag of the channel that is used to track the channel state.
    ///
    /// # Safety
    ///
    /// The channel assumes that the given `is_enabled` reflects the state of the channel.
    /// There shouldn't be any reason for the user to call this function directly.
    /// Use `enable`/`disable` instead.
    #[doc(hidden)]
    unsafe fn set_internal_enabled(&mut self, is_enabled: bool);

    /// Must be called in advance before transmitting or receiving RMT symbols.
    /// For TX channels, enabling a channel enables a specific interrupt and
    /// prepares the hardware to dispatch transactions.
    ///
    /// For RX channels, enabling a channel enables an interrupt, but the receiver
    /// is not started during this time, as the characteristics of the incoming
    /// signal have yet to be specified.
    ///
    /// The receiver is started in [`RxChannel::receive`].
    fn enable(&mut self) -> Result<(), EspError> {
        esp!(unsafe { rmt_enable(self.handle()) })?;

        // SAFETY: The channel has been enabled -> it is safe to mark it as enabled.
        unsafe { self.set_internal_enabled(true) };
        Ok(())
    }

    /// Disables the interrupt and clearing any pending interrupts. The transmitter
    /// and receiver are disabled as well.
    ///
    /// # Note
    ///
    /// This function will release a power management (PM) lock that might be
    /// installed during channel allocation
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Disable RMT channel failed because of invalid argument
    /// - `ESP_ERR_INVALID_STATE`: Disable RMT channel failed because it's not enabled yet
    /// - `ESP_FAIL`: Disable RMT channel failed because of other error
    fn disable(&mut self) -> Result<(), EspError> {
        esp!(unsafe { rmt_disable(self.handle()) })?;
        // SAFETY: The channel has been disable -> it is safe to mark it as disabled.
        unsafe { self.set_internal_enabled(false) };
        Ok(())
    }

    /// Apply (de)modulation feature for the channel.
    ///
    /// If `carrier_config` is `None`, the carrier (de)modulation will be disabled.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Apply carrier failed because of invalid argument
    /// - `ESP_FAIL`: Apply carrier failed because of other error
    fn apply_carrier(&mut self, carrier_config: Option<&CarrierConfig>) -> Result<(), EspError> {
        let mut sys_carrier = None;
        if let Some(CarrierConfig {
            frequency,
            duty_cycle,
            polarity_active_low,
            always_on,
            // match __internal to get a compiler error if new fields are added in the future
            __internal,
        }) = carrier_config
        {
            sys_carrier = Some(rmt_carrier_config_t {
                frequency_hz: (*frequency).into(),
                duty_cycle: duty_cycle.to_f32(),
                flags: rmt_carrier_config_t__bindgen_ty_1 {
                    _bitfield_1: rmt_carrier_config_t__bindgen_ty_1::new_bitfield_1(
                        *polarity_active_low as u32,
                        *always_on as u32,
                    ),
                    ..Default::default()
                },
            })
        }

        esp!(unsafe {
            rmt_apply_carrier(
                self.handle(),
                sys_carrier.as_ref().map_or(ptr::null(), |c| c as *const _),
            )
        })
    }
}

impl<R: RmtChannel> RmtChannel for &mut R {
    fn handle(&self) -> rmt_channel_handle_t {
        (**self).handle()
    }

    fn is_enabled(&self) -> bool {
        (**self).is_enabled()
    }

    unsafe fn set_internal_enabled(&mut self, is_enabled: bool) {
        (**self).set_internal_enabled(is_enabled)
    }

    fn enable(&mut self) -> Result<(), EspError> {
        (**self).enable()
    }

    fn disable(&mut self) -> Result<(), EspError> {
        (**self).disable()
    }

    fn apply_carrier(&mut self, carrier_config: Option<&CarrierConfig>) -> Result<(), EspError> {
        (**self).apply_carrier(carrier_config)
    }
}

/// Clock source for RMT channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum ClockSource {
    #[default]
    Default,
    #[cfg(any(esp32, esp32c3, esp32s2, esp32s3))]
    APB,
    #[cfg(any(
        esp32c3, esp32c5, esp32c6, esp32h2, esp32h4, esp32p4, esp32s3, esp32s31
    ))]
    RcFast,
    #[cfg(any(esp32, esp32s2))]
    RefTick,
    #[cfg(any(
        esp32c3, esp32c5, esp32c6, esp32h2, esp32h4, esp32p4, esp32s3, esp32s31
    ))]
    XTAL,
    #[cfg(any(esp32c5, esp32c6, esp32p4, esp32s31))]
    PLLF80M,
}

impl From<ClockSource> for rmt_clock_source_t {
    fn from(clock: ClockSource) -> Self {
        match clock {
            ClockSource::Default => soc_periph_rmt_clk_src_t_RMT_CLK_SRC_DEFAULT,
            #[cfg(any(esp32, esp32c3, esp32s2, esp32s3))]
            ClockSource::APB => soc_periph_rmt_clk_src_t_RMT_CLK_SRC_APB,
            #[cfg(any(
                esp32c3, esp32c5, esp32c6, esp32h2, esp32h4, esp32p4, esp32s3, esp32s31
            ))]
            ClockSource::RcFast => soc_periph_rmt_clk_src_t_RMT_CLK_SRC_RC_FAST,
            #[cfg(any(esp32, esp32s2))]
            ClockSource::RefTick => soc_periph_rmt_clk_src_t_RMT_CLK_SRC_REF_TICK,
            #[cfg(any(
                esp32c3, esp32c5, esp32c6, esp32h2, esp32h4, esp32p4, esp32s3, esp32s31
            ))]
            ClockSource::XTAL => soc_periph_rmt_clk_src_t_RMT_CLK_SRC_XTAL,
            #[cfg(any(esp32c5, esp32c6, esp32p4, esp32s31))]
            ClockSource::PLLF80M => soc_periph_rmt_clk_src_t_RMT_CLK_SRC_PLL_F80M,
        }
    }
}
