//! Pulse Counter (PCNT) peripheral.
//!
//! The PCNT (Pulse Counter) module is designed to count the number of rising
//! and/or falling edges of input signals. The ESP32 chip contains multiple pulse
//! counter units in the module. The number of available units and channels is
//! different for each chip. See the [Technical Reference Manual] of your chip for
//! details (or the table below).
//!
//! Each unit is in effect an independent counter with multiple channels (see [`PcntChannel`]),
//! where each channel can increment/decrement the counter on a rising/falling
//! edge. Furthermore, each channel can be configured separately.
//!
//! Besides that, PCNT unit is equipped with a separate glitch filter, which is
//! helpful to remove noise from the signal. See
//! [`PcntUnitDriver::set_glitch_filter`](pcnt::PcntUnitDriver::set_glitch_filter) for details.
//!
//! Typically, a PCNT module can be used in scenarios like:
//! - Calculate periodic signal's frequency by counting the pulse numbers within a time slice
//! - Decode quadrature signals into speed and direction
//! - Count the number of pulses that happen within a time slice (e.g. step signal of a stepper motor driver)
//!
//! [Technical Reference Manual]: https://www.espressif.com/sites/default/files/documentation/esp32_technical_reference_manual_en.pdf#pcnt
//!
//! # Driver redesign in ESP-IDF 5.0
//!
//! In ESP-IDF 5.0, the [PCNT API was redesigned] to simplify and unify the usage of the
//! PCNT peripheral.
//!
//! It is recommended to use the new API, but for now the old API is available through
//! the `pcnt-legacy` feature. The ESP-IDF 6.0 release will remove support for the legacy API.
//!
//! [PCNT API was redesigned]: https://docs.espressif.com/projects/esp-idf/en/stable/esp32/migration-guides/release-5.x/5.0/peripherals.html#pulse-counter-driver
//!
//! # Chip Capabilities
//!
//! The following table is based on the ESP-IDF source code and the relevant
//! constants are not public. Therefore, this table might be inaccurate or incomplete.
//!
//! |         | Units | Channels | Extra Watchpoints |
//! |---------|-------|----------|-------------------|
//! | esp32   | 8     | 2        | 2                 |
//! | esp32c6 | 4     | 2        | 2                 |
//! | esp32h4 | 4     | 2        | 2                 |
//! | esp32p4 | 4     | 2        | 2                 |
//! | esp32s2 | 4     | 2        | 2                 |
//! | esp32s3 | 4     | 2        | 2                 |
//!
//! **Units**: Maximum number of [`PcntUnitDriver`](pcnt::PcntUnitDriver) that can be created
//!
//! **Channels**: Maximum number of [`PcntChannel`](pcnt::PcntChannel) channels for each unit
//!
//! **Extra Watchpoints**: Maximum number of watchpoints (in addition to zero point and high/low limits) that can be set for each unit.
//!
//! The values seem to map to the following constants in `esp-idf-sys`:
//! - Maximum number of units: [`SOC_PCNT_UNITS_PER_GROUP`]
//! - Maximum number of channels per unit: [`SOC_PCNT_CHANNELS_PER_UNIT`]
//! - Maximum number of extra watchpoints: [`SOC_PCNT_THRES_POINT_PER_UNIT`]
//!
//! These 3 constants disappeared in ESP-IDF 6.0, as they were required by the
//! pcnt-legacy API which was removed.
//! The API rejects additional `new_*` calls beyond the maximum number of supported
//! channels. We do hardcode the maximum to 2 if alloc is not enabled, though.

use core::marker::PhantomData;
#[cfg(feature = "alloc")]
use core::pin::Pin;
use core::ptr;

#[cfg(feature = "alloc")]
use alloc::boxed::Box;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
#[cfg(not(feature = "alloc"))]
use heapless::Vec;

use esp_idf_sys::*;

// The anonymous `flags` struct of `pcnt_chan_config_t` was given a name in ESP-IDF 6.1,
// which changes how `bindgen` names the type it generates for it.
#[cfg(esp_idf_version_at_least_6_1_0)]
#[allow(non_camel_case_types)]
type pcnt_chan_config_t__bindgen_ty_1 = pcnt_chan_config_t_extra_pcnt_chan_flags;

// SOC_PCNT_CHANNELS_PER_UNIT was removed in IDF 6.0; the value is 2 for all supported chips,
// and we support more channels in the alloc case.
#[cfg(esp_idf_version_at_least_6_0_0)]
const PCNT_CHANNELS_PER_UNIT: usize = 2;
#[cfg(not(esp_idf_version_at_least_6_0_0))]
const PCNT_CHANNELS_PER_UNIT: usize = SOC_PCNT_CHANNELS_PER_UNIT as usize;

use crate::gpio::InputPin;
#[cfg(feature = "alloc")]
use crate::interrupt;

use config::*;

/// An event that is passed to the callback registered with [`PcntUnitDriver::subscribe`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WatchEventData {
    /// The count value when the event was triggered.
    pub watch_point_value: i32,
    /// How the PCNT unit crosses the zero point in the latest time.
    /// The possible zero cross modes are listed in the [`ZeroCrossMode`].
    ///
    /// Usually different zero cross mode means different counting direction and
    /// counting step size.
    pub zero_cross_mode: ZeroCrossMode,
}

impl From<pcnt_watch_event_data_t> for WatchEventData {
    #[inline(always)]
    fn from(value: pcnt_watch_event_data_t) -> Self {
        Self {
            watch_point_value: value.watch_point_value,
            zero_cross_mode: value.zero_cross_mode.into(),
        }
    }
}

/// This module contains configuration types and enums for the PCNT driver
pub mod config {
    use core::time::Duration;

    use esp_idf_sys::*;

    // The anonymous `flags` struct of `pcnt_unit_config_t` was given a name in ESP-IDF 6.1,
    // which changes how `bindgen` names the type it generates for it.
    #[cfg(esp_idf_version_at_least_6_1_0)]
    #[allow(non_camel_case_types)]
    type pcnt_unit_config_t__bindgen_ty_1 = pcnt_unit_config_t_extra_pcnt_unit_flags;

    /// Configuration for a PCNT unit.
    #[derive(Debug, Clone)]
    pub struct UnitConfig {
        /// Specifies the lower limit for the internal hardware counter.
        ///
        /// The counter will reset to zero automatically when it crosses the low limit.
        ///
        /// The internal limit in esp-idf is set to `-32_767` (= [`i16::MIN`]) for most chips,
        /// this might change in the future and should therefore not be relied on.
        pub low_limit: i32,
        /// Specifies the higher limit for the internal hardware counter.
        ///
        /// The counter will reset to zero automatically when it crosses the high limit.
        ///
        /// The internal limit in esp-idf is set to `32_767` (= [`i16::MAX`]) for most chips,
        /// this might change in the future and should therefore not be relied on.
        pub high_limit: i32,
        /// Sets the priority of the interrupt. If it is set to 0, the driver will
        /// allocate an interrupt with a default priority. Otherwise, the driver
        /// will use the given priority.
        ///
        /// Since all PCNT units share the same interrupt source, when installing
        /// multiple PCNT units make sure that the interrupt priority is the same for each unit.
        pub intr_priority: i32,
        /// If set to `true`, it will create an internal accumulator for the counter.
        /// This is helpful when you want to extend the counter's width, which by default
        /// is 16 bit at most, defined in the hardware.
        ///
        /// If enabled, the [`PcntUnitDriver::get_count`] will not only return the hardware's count
        /// value, but also the accumulated underflow/overflow.
        ///
        /// # Note
        ///
        /// You should add the high/low limits as the watch points.
        ///
        /// When enabling the count overflow compensation, it is recommended to use
        /// as large a high/low count limit as possible, as it can avoid frequent
        /// interrupt triggering, improve system performance, and avoid compensation
        /// failure due to multiple overflows.
        pub accum_count: bool,
        // This field is intentionally hidden to prevent non-exhaustive pattern matching.
        // You should only construct this struct using the `..Default::default()` pattern.
        // If you use this field directly, your code might break in future versions.
        #[doc(hidden)]
        #[allow(dead_code)]
        pub __internal: (),
    }

    impl Default for UnitConfig {
        fn default() -> Self {
            Self {
                low_limit: -1024,
                high_limit: 1024,
                intr_priority: 0,
                accum_count: false,
                __internal: (),
            }
        }
    }

    impl From<&UnitConfig> for pcnt_unit_config_t {
        fn from(value: &UnitConfig) -> Self {
            pcnt_unit_config_t {
                // All chips supported so far have a single PCNT peripheral instance
                #[cfg(esp_idf_version_at_least_6_1_0)]
                group_id: 0,
                // TODO: Do we want to make this configurable?
                // Most chips have only 1 option (APB==DEFAULT), but
                // esp32h4 supports XTAL(==DEFAULT) or RC_FAST.
                #[cfg(esp_idf_version_at_least_6_0_0)]
                clk_src: soc_periph_pcnt_clk_src_t_PCNT_CLK_SRC_DEFAULT,
                low_limit: value.low_limit,
                high_limit: value.high_limit,
                intr_priority: value.intr_priority,
                flags: pcnt_unit_config_t__bindgen_ty_1 {
                    _bitfield_1: pcnt_unit_config_t__bindgen_ty_1::new_bitfield_1(
                        value.accum_count as u32,
                        #[cfg(esp_idf_soc_pcnt_support_step_notify)]
                        {
                            0
                        },
                        #[cfg(esp_idf_soc_pcnt_support_step_notify)]
                        {
                            0
                        },
                    ),
                    ..Default::default()
                },
            }
        }
    }

    /// Configuration for the PCNT glitch filter.
    #[derive(Debug, Clone, Default)]
    pub struct GlitchFilterConfig {
        /// The maximum glitch width (will be converted to nanoseconds).
        ///
        /// If a signal pulse's width is smaller than this value, then it
        /// will be treated as noise and will not increase/decrease the
        /// internal counter.
        ///
        /// The upper limit for the maximum glitch width seems to be 1023ns
        /// (this might be different, depending on the chip).
        pub max_glitch: Duration,
        // This field is intentionally hidden to prevent non-exhaustive pattern matching.
        // You should only construct this struct using the `..Default::default()` pattern.
        // If you use this field directly, your code might break in future versions.
        #[doc(hidden)]
        #[allow(dead_code)]
        pub __internal: (),
    }

    impl From<&GlitchFilterConfig> for pcnt_glitch_filter_config_t {
        fn from(value: &GlitchFilterConfig) -> Self {
            pcnt_glitch_filter_config_t {
                max_glitch_ns: value.max_glitch.as_nanos().try_into().unwrap_or(u32::MAX),
            }
        }
    }

    /// Configuration for a PCNT channel.
    #[derive(Debug, Clone, Default)]
    pub struct ChannelConfig {
        /// Invert the input edge signal.
        pub invert_edge_input: bool,
        /// Invert the input level signal.
        pub invert_level_input: bool,
        /// The initial level of the virtual IO pin for edge signal.
        /// This is only valid if no edge pin has been specified.
        pub virt_edge_io_level: bool,
        /// The initial level of the virtual IO pin for level signal.
        /// This is only valid if no level pin has been specified.
        pub virt_level_io_level: bool,
        // This field is intentionally hidden to prevent non-exhaustive pattern matching.
        // You should only construct this struct using the `..Default::default()` pattern.
        // If you use this field directly, your code might break in future versions.
        #[doc(hidden)]
        #[allow(dead_code)]
        pub __internal: (),
    }

    /// PCNT channel action on control level.
    #[non_exhaustive]
    pub enum ChannelLevelAction {
        /// Keep current count mode
        Keep,
        /// Invert current count mode (increase -> decrease, decrease -> increase).
        Inverse,
        /// Hold current count value.
        Hold,
    }

    impl From<ChannelLevelAction> for pcnt_channel_level_action_t {
        fn from(value: ChannelLevelAction) -> Self {
            match value {
                ChannelLevelAction::Keep => {
                    pcnt_channel_level_action_t_PCNT_CHANNEL_LEVEL_ACTION_KEEP
                }
                ChannelLevelAction::Inverse => {
                    pcnt_channel_level_action_t_PCNT_CHANNEL_LEVEL_ACTION_INVERSE
                }
                ChannelLevelAction::Hold => {
                    pcnt_channel_level_action_t_PCNT_CHANNEL_LEVEL_ACTION_HOLD
                }
            }
        }
    }

    /// PCNT channel action on signal edge.
    #[non_exhaustive]
    pub enum ChannelEdgeAction {
        /// Hold current count value
        Hold,
        /// Increase count value
        Increase,
        /// Decrease count value
        Decrease,
    }

    impl From<ChannelEdgeAction> for pcnt_channel_edge_action_t {
        fn from(value: ChannelEdgeAction) -> Self {
            match value {
                ChannelEdgeAction::Hold => pcnt_channel_edge_action_t_PCNT_CHANNEL_EDGE_ACTION_HOLD,
                ChannelEdgeAction::Increase => {
                    pcnt_channel_edge_action_t_PCNT_CHANNEL_EDGE_ACTION_INCREASE
                }
                ChannelEdgeAction::Decrease => {
                    pcnt_channel_edge_action_t_PCNT_CHANNEL_EDGE_ACTION_DECREASE
                }
            }
        }
    }

    /// PCNT unit zero cross mode.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum ZeroCrossMode {
        /// start from positive value, end to zero, i.e. +N->0
        PositionZero,
        /// start from negative value, end to zero, i.e. -N->0
        NegativeZero,
        /// start from negative value, end to positive value, i.e. -N->+M
        NegativePosition,
        /// start from positive value, end to negative value, i.e. +N->-M
        PositiveNegative,
        /// invalid zero cross mode
        #[cfg(esp_idf_version_at_least_5_4_0)]
        #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_4_0)))]
        Invalid,
    }

    impl From<pcnt_unit_zero_cross_mode_t> for ZeroCrossMode {
        fn from(value: pcnt_unit_zero_cross_mode_t) -> Self {
            #[allow(non_upper_case_globals)]
            match value {
                pcnt_unit_zero_cross_mode_t_PCNT_UNIT_ZERO_CROSS_POS_ZERO => Self::PositionZero,
                pcnt_unit_zero_cross_mode_t_PCNT_UNIT_ZERO_CROSS_NEG_ZERO => Self::NegativeZero,
                pcnt_unit_zero_cross_mode_t_PCNT_UNIT_ZERO_CROSS_NEG_POS => Self::NegativePosition,
                pcnt_unit_zero_cross_mode_t_PCNT_UNIT_ZERO_CROSS_POS_NEG => Self::PositiveNegative,
                #[cfg(esp_idf_version_at_least_5_4_0)]
                pcnt_unit_zero_cross_mode_t_PCNT_UNIT_ZERO_CROSS_INVALID => Self::Invalid,
                _ => unreachable!("unknown zero cross mode: {value}"),
            }
        }
    }
}

/// A PCNT channel of a PCNT unit.
///
/// PCNT channels can react to signals of **edge** type and **level** type,
/// however for simple applications, detecting the edge signal is usually
/// sufficient.
///
/// PCNT channels can be configured to react to both pulse edges (i.e., rising
/// and falling edge), and can be configured to increase, decrease or do nothing
/// to the unit's counter on each edge. The level signal is the so-called control
/// signal, which is used to control the counting mode of the edge signals that
/// are attached to the same channel.
///
/// By combining the usage of both edge and level signals, a PCNT unit can act as
/// a **quadrature decoder**.
#[derive(Debug)]
pub struct PcntChannel<'d> {
    handle: pcnt_channel_handle_t,
    _p: PhantomData<&'d mut ()>,
}

impl<'d> PcntChannel<'d> {
    unsafe fn new(
        pcnt_unit: pcnt_unit_handle_t,
        edge_pin: Option<impl InputPin + 'd>,
        level_pin: Option<impl InputPin + 'd>,
        config: &ChannelConfig,
    ) -> Result<Self, EspError> {
        let sys_config = pcnt_chan_config_t {
            edge_gpio_num: edge_pin.map_or(-1, |p| p.pin().into()),
            level_gpio_num: level_pin.map_or(-1, |p| p.pin().into()),
            flags: pcnt_chan_config_t__bindgen_ty_1 {
                _bitfield_1: pcnt_chan_config_t__bindgen_ty_1::new_bitfield_1(
                    config.invert_edge_input as u32,
                    config.invert_level_input as u32,
                    config.virt_edge_io_level as u32,
                    config.virt_level_io_level as u32,
                    // This field is marked as for removal in ESP-IDF 6.0.
                    #[cfg(not(esp_idf_version_at_least_6_0_0))]
                    {
                        false as u32
                    },
                ),
                ..Default::default()
            },
        };

        let mut handle = ptr::null_mut();
        esp!(unsafe { pcnt_new_channel(pcnt_unit, &sys_config, &mut handle) })?;

        Ok(Self {
            handle,
            _p: PhantomData,
        })
    }

    /// Set channel actions when edge signal changes (e.g. falling or rising edge occurred).
    ///
    /// The edge signal is input from the edge pin this channel was configured with.
    /// We use these actions to control when and how to change the counter value.
    ///
    /// # What is an edge?
    ///
    /// When you plot the signal on a graph, where the x-axis is time and the y-axis is the signal level
    /// (high or low, 1.5V or 0V, ...), an edge is the point in time where a signal changes from one level
    /// to another
    ///
    /// A rising edge looks like this:
    /// ```text
    ///      ┌─────
    ///      │
    ///      │
    /// ─────┘
    ///      ^ a positive edge
    /// ```
    /// and a falling edge where the signal goes from high to low looks like this:
    /// ```text
    /// ─────┐
    ///      │
    ///      │
    ///      └─────
    ///      ^ a negative edge
    /// ```
    ///
    /// You can then configure what actions should happen on these edges.
    /// Given the signal
    ///
    /// ```text
    ///            ┌─────────────────┐     ┌──────────────
    ///            │                 │     │
    ///            │                 │     │
    /// ───────────┘                 └─────┘
    ///            ↑                 ↑     ↑
    ///         rising            falling rising
    /// ```
    /// you could configure it to only count on rising edges by setting
    /// `pos_act` to [`Increase`] and `neg_act` to [`Hold`].
    /// This would result in the counter being increased twice (for each rising edge),
    /// and the falling edges would not change the counter value.
    ///
    /// # Nonsensical configurations
    ///
    /// There are some configurations that are unlikely to be what you want, when setting
    /// - `pos_act` to [`Increase`] and `neg_act` to [`Decrease`] the counter would never change,
    ///   if the signal has an equal number of rising and falling edges.
    /// - `pos_act` to [`Decrease`] and `neg_act` to [`Increase`] would have the same effect.
    /// - `pos_act` to [`Hold`] and `neg_act` to [`Hold`] would never change the counter.
    ///
    /// [`Increase`]: config::ChannelEdgeAction::Increase
    /// [`Decrease`]: config::ChannelEdgeAction::Decrease
    /// [`Hold`]: config::ChannelEdgeAction::Hold
    pub fn set_edge_action(
        &mut self,
        pos_act: ChannelEdgeAction,
        neg_act: ChannelEdgeAction,
    ) -> Result<&mut Self, EspError> {
        esp!(unsafe { pcnt_channel_set_edge_action(self.handle, pos_act.into(), neg_act.into()) })?;

        Ok(self)
    }

    /// Set channel actions when level signal changes (e.g. signal level goes from high to low).
    ///
    /// The level signal is input from the `level_pin` configured in [`Self::new`].
    /// We use these actions to control when and how to change the counting mode.
    ///
    /// # What is a level?
    ///
    /// When you plot the signal on a graph, where the x-axis is time and the y-axis is the signal level
    /// (high or low, 1.5V or 0V, ...), the level is the current state of the signal.
    ///
    ///
    /// ```text
    ///      v the level is high, until here      v
    ///      ┌────────────────────────────────────┐
    ///      │                                    │
    ///      │                                    │
    /// ─────┘                                    └────
    /// ^^^^^                                      ^^^^
    /// here the level is low                      and here it is low too
    /// ```
    ///
    /// # What is the difference between triggering on edge and on level?
    ///
    /// The counter will count on edges, the level does not increase or decrease the counter.
    /// For example, if you set both edge actions to [`Hold`](config::ChannelEdgeAction::Hold)
    /// it would never change the counter, no matter what the level actions are.
    ///
    /// The level only influences how it is counted.
    ///
    /// ### An example
    /// Assume the edge actions are set to `Increase` for rising edges and `Hold` for falling edges,
    /// then it will only count up when a rising edge is encountered.
    ///
    /// If the level actions are set to [`Keep`] for **high** level and `Hold` for **low** level,
    /// the counter will only count up **while** the level signal is **high** and ignore
    /// rising edges while the level signal is **low**.
    ///
    /// If the level actions are set to [`Inverse`] for high level and [`Keep`] for **low** level,
    /// it will count down **while** the level is **high** on rising edges and count up **while**
    /// the level is **low** on rising edges.
    ///
    /// You can think of the level actions like this:
    /// ```ignore
    /// let mut counter: i32 = 0;
    /// let mut increment = 1;
    ///
    /// loop {
    ///     // read the level signal and set the action accordingly:
    ///     let action = {
    ///         if level_signal_is_high() {
    ///             high_act
    ///         } else { // if it is not high, it must be low
    ///             low_act
    ///         }
    ///     };
    ///
    ///     // If it was previously set to Hold, increment would be 0, but with
    ///     // the actions Keep/Inverse it should start counting again
    ///     // -> it is set to 1 here
    ///     if increment == 0 && !matches!(action, ChannelLevelAction::Hold) {
    ///         increment = 1; // reset increment to default
    ///     }
    ///
    ///     // level is high, so set increment based on high_act
    ///     match action {
    ///         ChannelLevelAction::Keep => {}, // do nothing, increment stays 1
    ///         ChannelLevelAction::Inverse => { increment = -increment; }, // invert increment
    ///         ChannelLevelAction::Hold => { increment = 0; }, // hold increment (no counting)
    ///     }
    ///
    ///
    ///     match readEdgeSignal() {
    ///        Edge::Rising => counter += increment,
    ///        Edge::Falling => {}, // Hold
    ///        _ => {}, // no edge
    ///     }
    /// }
    /// ```
    /// note that this is just an illustrative example, the code does not work.
    ///
    /// # Nonsensical configurations
    ///
    /// There are some configurations that are unlikely to be what you want, when setting
    /// - `high_act` to [`Keep`] and `low_act` to [`Keep`] the counting would never change based on the level.
    /// - `high_act` to [`Hold`] and `low_act` to [`Hold`] would never change the counter.
    /// - `high_act` to [`Inverse`] and `low_act` to [`Inverse`], with this the rising and falling edges would
    ///   add up to no zero.
    ///
    /// If you do not want to change the counting mode based on the level, it is recommended to not specify a
    /// level pin in [`PcntUnitDriver::add_channel`], instead leaving it as `None`. A **virtual IO** pin will
    /// be assigned instead. The default level of the virtual IO pin can be configured with
    /// [`ChannelConfig::virt_level_io_level`].
    ///
    /// [`Keep`]: config::ChannelLevelAction::Keep
    /// [`Inverse`]: config::ChannelLevelAction::Inverse
    /// [`Hold`]: config::ChannelLevelAction::Hold
    pub fn set_level_action(
        &mut self,
        high_act: ChannelLevelAction,
        low_act: ChannelLevelAction,
    ) -> Result<&mut Self, EspError> {
        esp!(unsafe {
            pcnt_channel_set_level_action(self.handle, high_act.into(), low_act.into())
        })?;

        Ok(self)
    }
}

impl<'d> Drop for PcntChannel<'d> {
    fn drop(&mut self) {
        unsafe {
            pcnt_del_channel(self.handle);
        }
    }
}

// SAFETY: The PCNT channel does not use thread locals -> it should be safe to send it to another thread
unsafe impl<'d> Send for PcntChannel<'d> {}

#[cfg(feature = "alloc")]
struct DelegateUserData<'d> {
    on_reach: Box<dyn FnMut(WatchEventData) + Send + 'd>,
}

/// A driver for a PCNT unit.
///
/// Each driver has a counter, which can be controlled by one or more channels,
/// that have to be added with [`PcntUnitDriver::add_channel`].
pub struct PcntUnitDriver<'d> {
    handle: pcnt_unit_handle_t,
    #[cfg(feature = "alloc")]
    channels: alloc::vec::Vec<PcntChannel<'d>>,
    #[cfg(not(feature = "alloc"))]
    channels: Vec<PcntChannel<'d>, PCNT_CHANNELS_PER_UNIT>,
    #[cfg(feature = "alloc")]
    user_data: Option<Pin<Box<DelegateUserData<'d>>>>,
    _p: PhantomData<&'d mut ()>,
}

/// The functions listed in this `impl` are available in any state (enabled or disabled).
impl<'d> PcntUnitDriver<'d> {
    /// Create a new PCNT unit driver instance, with the given configuration.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Create PCNT unit failed because of invalid argument (e.g. high/low limit value out of the range)
    /// - `ESP_ERR_NO_MEM`: Create PCNT unit failed because out of memory
    /// - `ESP_ERR_NOT_FOUND`: Create PCNT unit failed because all PCNT units are used up and no more free one
    /// - `ESP_FAIL`: Create PCNT unit failed because of other error
    pub fn new(config: &UnitConfig) -> Result<Self, EspError> {
        let sys_config = config.into();
        let mut handle = ptr::null_mut();
        esp!(unsafe { pcnt_new_unit(&sys_config, &mut handle) })?;

        Ok(Self {
            handle,
            #[cfg(feature = "alloc")] // Pre-allocate to usual maximum size
            channels: Vec::with_capacity(PCNT_CHANNELS_PER_UNIT),
            #[cfg(not(feature = "alloc"))] // Fixed size
            channels: Vec::new(),
            #[cfg(feature = "alloc")]
            user_data: None,
            _p: PhantomData,
        })
    }

    /// Returns a handle to the underlying PCNT unit.
    #[must_use]
    pub fn handle(&self) -> pcnt_unit_handle_t {
        self.handle
    }

    /// Add a watch point to the PCNT unit.
    ///
    /// PCNT will generate an event when the counter value reaches the watch point value.
    ///
    /// It is recommended to call [`Self::clear_count`] after adding a watch point,
    /// so that the newly added watch point can take effect immediately.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Add watch point failed because of invalid argument, like the value is out of range
    /// - `ESP_ERR_INVALID_STATE`: Add watch point failed because the same watch point has already been added
    /// - `ESP_ERR_NOT_FOUND`: Add watch point failed because no more hardware watch point can be configured
    /// - `ESP_FAIL`: Add watch point failed because of other error
    pub fn add_watch_point(&mut self, watch_point: i32) -> Result<(), EspError> {
        esp!(unsafe { pcnt_unit_add_watch_point(self.handle, watch_point) })
    }

    /// This is a convenience function to add multiple watch points.
    ///
    /// It will add all the watch points from the iterator and clear the count
    /// if they were successfully added.
    ///
    /// # Errors
    ///
    /// If an error occurs while adding any of the watch points, it will not
    /// continue adding the subsequent watch points and not clear the count.
    /// Previously added watch points will **not** be removed, for this use
    /// [`Self::remove_watch_point`].
    pub fn add_watch_points_and_clear(
        &mut self,
        iterator: impl IntoIterator<Item = i32>,
    ) -> Result<(), EspError> {
        for watch_point in iterator {
            self.add_watch_point(watch_point)?;
        }

        self.clear_count()?;

        Ok(())
    }

    /// Remove a watch point for PCNT unit.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Remove watch point failed because of invalid argument
    /// - `ESP_ERR_INVALID_STATE`: Remove watch point failed because the watch point was not added by [`Self::add_watch_point`] yet
    /// - `ESP_FAIL`: Remove watch point failed because of other error
    pub fn remove_watch_point(&mut self, watch_point: i32) -> Result<(), EspError> {
        esp!(unsafe { pcnt_unit_remove_watch_point(self.handle, watch_point) })
    }

    /// Add a step notify for PCNT unit.
    ///
    /// PCNT will generate an event when the incremental (can be positive or negative)
    /// of counter value reaches the step interval value.
    ///
    /// A positive step interval is a step forward, while a negative step interval
    /// is a step backward.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Add step notify failed because of invalid argument (e.g. the value incremental to be watched is out of the limitation set in [`UnitConfig`])
    /// - `ESP_ERR_INVALID_STATE`: Add step notify failed because the step notify has already been added
    /// - `ESP_FAIL`: Add step notify failed because of other error
    #[cfg(esp_idf_version_at_least_5_4_0)]
    #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_4_0)))]
    pub fn add_watch_step(&mut self, step_interval: i32) -> Result<(), EspError> {
        esp!(unsafe { pcnt_unit_add_watch_step(self.handle, step_interval) })
    }

    /// Remove all watch steps for a PCNT unit.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_STATE`: Remove step notify failed because the step notify was not added by [`Self::add_watch_step`] yet
    /// - `ESP_FAIL`: Remove step notify failed because of other error
    #[cfg(esp_idf_version_at_least_5_4_0)]
    #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_4_0)))]
    pub fn remove_all_watch_step(&mut self) -> Result<(), EspError> {
        // On version 5.4 the remove_watch_step was equivalent to remove_all_watch_step.
        // In 5.5 it was renamed to remove_all_watch_step and a new function remove_single_watch_step was added.
        //
        // The below is to reduce confusion when switching IDF versions.
        #[cfg(esp_idf_version_at_least_5_5_0)]
        {
            esp!(unsafe { pcnt_unit_remove_all_watch_step(self.handle) })
        }
        #[cfg(all(esp_idf_version_at_least_5_4_0, not(esp_idf_version_at_least_5_5_0)))]
        {
            esp!(unsafe { pcnt_unit_remove_watch_step(self.handle) })
        }
    }

    /// Remove a watch step for PCNT unit
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Remove step notify failed because of invalid argument
    /// - `ESP_ERR_INVALID_STATE`: Remove step notify failed because the step notify was not added by [`Self::add_watch_step`] yet
    /// - `ESP_FAIL`: Remove step notify failed because of other error
    #[cfg(esp_idf_version_at_least_5_5_0)]
    #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_5_0)))]
    pub fn remove_single_watch_step(&mut self, step_interval: i32) -> Result<(), EspError> {
        esp!(unsafe { pcnt_unit_remove_single_watch_step(self.handle, step_interval) })
    }
}

#[cfg(feature = "alloc")]
const DISABLE_EVENT_CALLBACKS: pcnt_event_callbacks_t = pcnt_event_callbacks_t { on_reach: None };

/// The functions listed in this `impl` are only available, when the PCNT unit is **disabled**.
///
/// After calling [`PcntUnitDriver::new`], the unit will be in the disabled state. To enable
/// it, call [`PcntUnitDriver::enable`] and to disable it again, call [`PcntUnitDriver::disable`].
///
/// # Errors
///
/// If a function is called in the wrong state, it will return an [`EspError`] with the code [`ESP_ERR_INVALID_STATE`].
impl<'d> PcntUnitDriver<'d> {
    /// The PCNT unit features filters to ignore possible short glitches in the signals.
    ///
    /// You can enable the glitch filter for the pcnt unit, by calling this method with
    /// a filter configuration. If you want to disable the filter, call this method with
    /// `None`.
    ///
    /// # Note
    ///
    /// The glitch filter operates using the APB clock. To ensure the counter does not miss
    /// any pulses, the maximum glitch width should be longer than one APB_CLK cycle
    /// (typically 12.5 ns if APB is 80 MHz). Since the APB frequency can change with Dynamic
    /// Frequency Scaling (DFS), the filter may not function as expected in such cases.
    ///
    /// Therefore, the driver installs a power management lock for each PCNT unit. For more
    /// details on the power management strategy used in the PCNT driver, please refer
    /// to [Power Management].
    ///
    /// [Power Management]: https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/peripherals/pcnt.html#pcnt-power-management
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Set glitch filter failed because of invalid argument (e.g. glitch width is too big)
    /// - `ESP_FAIL`: Set glitch filter failed because of other error
    pub fn set_glitch_filter(
        &mut self,
        config: Option<&GlitchFilterConfig>,
    ) -> Result<&mut Self, EspError> {
        let sys_config = config.map(|config| config.into());
        esp!(unsafe {
            pcnt_unit_set_glitch_filter(self.handle, sys_config.as_ref().map_or(ptr::null(), |c| c))
        })?;
        Ok(self)
    }

    /// Adds a new channel to the PCNT unit, with the given edge and level pins.
    ///
    /// If no pin is specified, a virtual IO pin will be assigned instead.
    /// The initial level of the virtual IO pin can be configured with the [`ChannelConfig`].
    ///
    /// # Number of channels
    ///
    /// There is a hardware limit for how many channels a PCNT unit can have.
    /// For most chips, each PCNT unit can have 2 channels.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Create PCNT channel failed because of invalid argument
    /// - `ESP_ERR_NO_MEM`: Create PCNT channel failed because of insufficient memory
    /// - `ESP_ERR_NOT_FOUND`: Create PCNT channel failed because all PCNT channels are used up and no more free one
    /// - `ESP_FAIL`: Create PCNT channel failed because of other error
    pub fn add_channel(
        &mut self,
        edge_pin: Option<impl InputPin + 'd>,
        level_pin: Option<impl InputPin + 'd>,
        config: &ChannelConfig,
    ) -> Result<&mut PcntChannel<'d>, EspError> {
        #[cfg(feature = "alloc")]
        self.channels
            .push(unsafe { PcntChannel::new(self.handle, edge_pin, level_pin, config)? });
        #[cfg(not(feature = "alloc"))]
        self.channels
            .push(unsafe { PcntChannel::new(self.handle, edge_pin, level_pin, config)? })
            .expect("Not enough channels available.");

        Ok(self.channels.last_mut().unwrap())
    }

    /// Set event callback for the PCNT unit.
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
    /// - `ESP_ERR_INVALID_ARG`: Set event callbacks failed because of invalid argument
    /// - `ESP_FAIL`: Set event callbacks failed because of other error
    #[cfg(feature = "alloc")]
    pub fn subscribe(
        &mut self,
        on_reach: impl FnMut(WatchEventData) + Send + 'static,
    ) -> Result<&mut Self, EspError> {
        // SAFETY: The closure is 'static -> even with `mem::forget` it wouldn't lead to undefined behavior.
        unsafe { self.subscribe_nonstatic(on_reach) }
    }

    /// Set event callback for the PCNT unit.
    ///
    /// # ISR Safety
    ///
    /// Care should be taken not to call std, libc or FreeRTOS APIs (except for a few allowed ones)
    /// in the callback passed to this function, as it is executed in an ISR context.
    ///
    /// You are not allowed to block, but you are allowed to call FreeRTOS APIs with the FromISR suffix.
    ///
    /// # Safety
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
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Set event callbacks failed because of invalid argument
    /// - `ESP_FAIL`: Set event callbacks failed because of other error
    #[cfg(feature = "alloc")]
    pub unsafe fn subscribe_nonstatic(
        &mut self,
        on_reach: impl FnMut(WatchEventData) + Send + 'd,
    ) -> Result<&mut Self, EspError> {
        let mut pinned = Box::pin(DelegateUserData {
            on_reach: Box::new(on_reach),
        });

        esp!(unsafe {
            pcnt_unit_register_event_callbacks(
                self.handle,
                &Self::ENABLE_EVENT_CALLBACKS,
                // SAFETY: The referenced data will not be moved out and this is the only reference to it
                (pinned.as_mut().get_unchecked_mut()) as *mut DelegateUserData
                    as *mut core::ffi::c_void,
            )
        })?;

        self.user_data = Some(pinned);

        Ok(self)
    }

    /// Unregister event callback for the PCNT unit.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_ARG`: Failed because of other error
    #[cfg(feature = "alloc")]
    pub fn unsubscribe(&mut self) -> Result<&mut Self, EspError> {
        esp!(unsafe {
            pcnt_unit_register_event_callbacks(
                self.handle,
                &DISABLE_EVENT_CALLBACKS,
                ptr::null_mut(),
            )
        })?;
        self.user_data = None;
        Ok(self)
    }

    #[cfg(feature = "alloc")]
    const ENABLE_EVENT_CALLBACKS: pcnt_event_callbacks_t = pcnt_event_callbacks_t {
        on_reach: Some(Self::delegate_on_reach),
    };

    /// Enable the PCNT unit.
    ///
    /// # Note
    ///
    /// This function will enable the interrupt service if it was subscribed to with [`Self::subscribe`].
    /// This function will acquire a power management lock if the glitch filter was enabled with [`Self::set_glitch_filter`].
    ///
    /// Enabling the PCNT unit will not start the counting. You must call [`PcntUnitDriver::start`] to start counting.
    ///
    /// # Errors
    ///
    /// - `ESP_ERR_INVALID_STATE`: Enable PCNT unit failed because the unit is already enabled
    /// - `ESP_FAIL`: Enable PCNT unit failed because of other error
    pub fn enable(&mut self) -> Result<(), EspError> {
        esp!(unsafe { pcnt_unit_enable(self.handle) })?;

        Ok(())
    }

    #[cfg(feature = "alloc")]
    unsafe extern "C" fn delegate_on_reach(
        _handle: pcnt_unit_handle_t,
        event_data: *const pcnt_watch_event_data_t,
        user_data: *mut core::ffi::c_void,
    ) -> bool {
        let user_data = &mut *(user_data as *mut DelegateUserData<'d>);
        let event_data = WatchEventData::from(*event_data);

        interrupt::with_isr_yield_signal(move || (user_data.on_reach)(event_data))
    }
}

/// The functions listed in this `impl` are only available, when the PCNT unit is **enabled**.
///
/// After calling [`PcntUnitDriver::new`], the unit will be in the disabled state. To enable
/// it, call [`PcntUnitDriver::enable`] and to disable it again, call [`PcntUnitDriver::disable`].
///
/// # Errors
///
/// If a function is called in the wrong state, it will return an [`EspError`] with the code [`ESP_ERR_INVALID_STATE`].
impl<'d> PcntUnitDriver<'d> {
    /// Disable the PCNT unit.
    ///
    /// # Note
    ///
    /// Disable a PCNT unit doesn't mean to stop it. See also [`Self::stop`] for how to stop the PCNT counter.
    ///
    /// # Errors
    ///
    ///
    /// - `ESP_ERR_INVALID_STATE`: Disable PCNT unit failed because the unit is already disabled
    /// - `ESP_FAIL`: Disable PCNT unit failed because of other error
    pub fn disable(&mut self) -> Result<(), EspError> {
        esp!(unsafe { pcnt_unit_disable(self.handle) })?;

        Ok(())
    }

    /// Start the PCNT unit, the counter will start to count according to the edge and/or level input signals.
    ///
    /// # ISR Safety
    ///
    /// This function is safe to be called from an ISR context.
    ///
    /// The underlying esp-idf function will be placed into IRAM if `CONFIG_PCNT_CTRL_FUNC_IN_IRAM`
    /// is on, so that it's allowed to be executed when cache is disabled.
    ///
    /// This is not the case for this rust function at the moment, this might change in the future.
    ///
    /// # Errors
    ///
    /// - `ESP_FAIL`: Start PCNT unit failed because of other error
    pub fn start(&self) -> Result<(), EspError> {
        esp!(unsafe { pcnt_unit_start(self.handle) })
    }

    /// Stop PCNT from counting.
    ///
    /// # ISR Safety
    ///
    /// This function is safe to be called from an ISR context.
    ///
    /// The underlying esp-idf function will be placed into IRAM if `CONFIG_PCNT_CTRL_FUNC_IN_IRAM`
    /// is on, so that it's allowed to be executed when cache is disabled.
    ///
    /// This is not the case for this rust function at the moment, this might change in the future.
    ///
    /// # Note
    ///
    /// The stop operation won't clear the counter. Also see [`Self::clear_count`]
    /// for how to clear pulse count value.
    ///
    /// # Errors
    ///
    /// - `ESP_FAIL`:Failed because of other error
    pub fn stop(&self) -> Result<(), EspError> {
        esp!(unsafe { pcnt_unit_stop(self.handle) })
    }

    /// Clear PCNT pulse count value to zero.
    ///
    /// # ISR Safety
    ///
    /// This function is safe to be called from an ISR context.
    ///
    /// The underlying esp-idf function will be placed into IRAM if `CONFIG_PCNT_CTRL_FUNC_IN_IRAM`
    /// is on, so that it's allowed to be executed when cache is disabled.
    ///
    /// This is not the case for this rust function at the moment, this might change in the future.
    ///
    /// # Errors
    ///
    /// - `ESP_FAIL`: Failed because of other error
    pub fn clear_count(&self) -> Result<(), EspError> {
        esp!(unsafe { pcnt_unit_clear_count(self.handle) })
    }

    /// Get the current pulse count value.
    ///
    /// If the accumulation mode was enabled in the unit configuration,
    /// this function will return the accumulated count value, which
    /// includes the hardware counter value and the accumulated underflow/overflow.
    ///
    /// # ISR Safety
    ///
    /// This function is safe to be called from an ISR context.
    ///
    /// The underlying esp-idf function will be placed into IRAM if `CONFIG_PCNT_CTRL_FUNC_IN_IRAM`
    /// is on, so that it's allowed to be executed when cache is disabled.
    ///
    /// This is not the case for this rust function at the moment, this might change in the future.
    ///
    /// # Errors
    ///
    /// - `ESP_FAIL`: Failed because of other error
    pub fn get_count(&self) -> Result<i32, EspError> {
        let mut count = 0;
        esp!(unsafe { pcnt_unit_get_count(self.handle, &mut count) })?;
        Ok(count)
    }
}

// SAFETY: No thread-locals are used
unsafe impl<'d> Send for PcntUnitDriver<'d> {}
// SAFETY: All the functions taking &self are safe to be called from other threads
unsafe impl<'d> Sync for PcntUnitDriver<'d> {}

impl<'d> Drop for PcntUnitDriver<'d> {
    fn drop(&mut self) {
        // Disable the unit before deleting it (this is necessary). If it is already disabled,
        // this call would error, but that is ignored here (there is nothing we can do about it).
        let _ = self.disable();

        #[cfg(feature = "alloc")]
        unsafe {
            pcnt_unit_register_event_callbacks(
                self.handle,
                &DISABLE_EVENT_CALLBACKS,
                ptr::null_mut(),
            )
        };

        // Channels must be deleted before the unit: pcnt_del_unit refuses
        // (ESP_ERR_INVALID_STATE) while any channel is still attached.
        self.channels.clear();

        unsafe {
            pcnt_del_unit(self.handle);
        }
    }
}
