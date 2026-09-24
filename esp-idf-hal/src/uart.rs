//: QueueHandle_t ! UART peripheral control
//! Controls UART peripherals (UART0, UART1, UART2).
//!
//! Notice that UART0 is typically already used for loading firmware and logging.
//! Therefore use UART1 and UART2 in your application.
//! Any pin can be used for `rx` and `tx`.
//!
//! # Example
//!
//! Create a serial peripheral and write to serial port.
//! ```
//! use std::fmt::Write;
//! use esp_idf_hal::prelude::*;
//! use esp_idf_hal::uart;
//!
//! let peripherals = Peripherals::take().unwrap();
//! let pins = peripherals.pins;
//!
//! let config = uart::config::Config::default().baudrate(Hertz(115_200));
//!
//! let mut uart: uart::UartDriver = uart::UartDriver::new(
//!     peripherals.uart1,
//!     pins.gpio1,
//!     pins.gpio3,
//!     Option::<AnyIOPin>::None,
//!     Option::<AnyIOPin>::None,
//!     &config
//! ).unwrap();
//!
//! for i in 0..10 {
//!     writeln!(uart, "{:}", format!("count {:}", i)).unwrap();
//! }
//! ```
//!
//! # TODO
//! - Add all extra features esp32 supports
//! - Free APB lock when TX is idle (and no RX used)
//! - Address errata 3.17: UART fifo_cnt is inconsistent with FIFO pointer

use core::borrow::BorrowMut;
use core::ffi::CStr;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ptr;
use core::sync::atomic::{AtomicU8, Ordering};

use crate::cpu::Core;
use crate::delay::{self, NON_BLOCK};
use crate::interrupt::InterruptType;
use crate::io::EspIOError;
use crate::task::asynch::Notification;
use crate::task::queue::Queue;
use crate::units::*;
use crate::{gpio::*, task};

use embedded_hal_nb::serial::ErrorKind;
use esp_idf_sys::*;

const UART_FIFO_SIZE: usize = SOC_UART_FIFO_LEN as usize;

pub type UartConfig = config::Config;

/// UART configuration
pub mod config {
    use crate::{interrupt::InterruptType, units::*};
    use enumset::{enum_set, EnumSet, EnumSetType};
    use esp_idf_sys::*;

    /// Mode
    #[derive(PartialEq, Eq, Copy, Clone, Debug)]
    pub enum Mode {
        /// regular UART mode
        UART,
        /// half duplex RS485 UART mode control by RTS pin
        RS485HalfDuplex,
    }

    impl From<Mode> for uart_mode_t {
        fn from(mode: Mode) -> Self {
            match mode {
                Mode::UART => uart_mode_t_UART_MODE_UART,
                Mode::RS485HalfDuplex => uart_mode_t_UART_MODE_RS485_HALF_DUPLEX,
            }
        }
    }

    impl From<uart_mode_t> for Mode {
        #[allow(non_upper_case_globals)]
        fn from(uart_mode: uart_mode_t) -> Self {
            match uart_mode {
                uart_mode_t_UART_MODE_UART => Mode::UART,
                uart_mode_t_UART_MODE_RS485_HALF_DUPLEX => Mode::RS485HalfDuplex,
                _ => unreachable!(),
            }
        }
    }

    /// Number of data bits
    #[derive(PartialEq, Eq, Copy, Clone, Debug)]
    pub enum DataBits {
        DataBits5,
        DataBits6,
        DataBits7,
        DataBits8,
    }

    impl From<DataBits> for uart_word_length_t {
        fn from(data_bits: DataBits) -> Self {
            match data_bits {
                DataBits::DataBits5 => uart_word_length_t_UART_DATA_5_BITS,
                DataBits::DataBits6 => uart_word_length_t_UART_DATA_6_BITS,
                DataBits::DataBits7 => uart_word_length_t_UART_DATA_7_BITS,
                DataBits::DataBits8 => uart_word_length_t_UART_DATA_8_BITS,
            }
        }
    }

    impl From<uart_word_length_t> for DataBits {
        #[allow(non_upper_case_globals)]
        fn from(word_length: uart_word_length_t) -> Self {
            match word_length {
                uart_word_length_t_UART_DATA_5_BITS => DataBits::DataBits5,
                uart_word_length_t_UART_DATA_6_BITS => DataBits::DataBits6,
                uart_word_length_t_UART_DATA_7_BITS => DataBits::DataBits7,
                uart_word_length_t_UART_DATA_8_BITS => DataBits::DataBits8,
                _ => unreachable!(),
            }
        }
    }

    /// Flow control
    #[derive(PartialEq, Eq, Copy, Clone, Debug)]
    pub enum FlowControl {
        None,
        RTS,
        CTS,
        CTSRTS,
        MAX,
    }

    impl From<FlowControl> for uart_hw_flowcontrol_t {
        fn from(flow_control: FlowControl) -> Self {
            match flow_control {
                FlowControl::None => uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_DISABLE,
                FlowControl::RTS => uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_RTS,
                FlowControl::CTS => uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_CTS,
                FlowControl::CTSRTS => uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_CTS_RTS,
                FlowControl::MAX => uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_MAX,
            }
        }
    }

    impl From<uart_hw_flowcontrol_t> for FlowControl {
        #[allow(non_upper_case_globals)]
        fn from(flow_control: uart_hw_flowcontrol_t) -> Self {
            match flow_control {
                uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_DISABLE => FlowControl::None,
                uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_RTS => FlowControl::RTS,
                uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_CTS => FlowControl::CTS,
                uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_CTS_RTS => FlowControl::CTSRTS,
                uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_MAX => FlowControl::MAX,
                _ => unreachable!(),
            }
        }
    }

    /// Parity check
    #[derive(PartialEq, Eq, Copy, Clone, Debug)]
    pub enum Parity {
        ParityNone,
        ParityEven,
        ParityOdd,
    }

    impl From<Parity> for uart_parity_t {
        fn from(parity: Parity) -> Self {
            match parity {
                Parity::ParityNone => uart_parity_t_UART_PARITY_DISABLE,
                Parity::ParityEven => uart_parity_t_UART_PARITY_EVEN,
                Parity::ParityOdd => uart_parity_t_UART_PARITY_ODD,
            }
        }
    }

    impl From<uart_parity_t> for Parity {
        #[allow(non_upper_case_globals)]
        fn from(parity: uart_parity_t) -> Self {
            match parity {
                uart_parity_t_UART_PARITY_DISABLE => Parity::ParityNone,
                uart_parity_t_UART_PARITY_EVEN => Parity::ParityEven,
                uart_parity_t_UART_PARITY_ODD => Parity::ParityOdd,
                _ => unreachable!(),
            }
        }
    }

    /// Number of stop bits
    #[derive(PartialEq, Eq, Copy, Clone, Debug)]
    pub enum StopBits {
        /// 1 stop bit
        STOP1,
        /// 1.5 stop bits
        STOP1P5,
        /// 2 stop bits
        STOP2,
    }

    impl From<StopBits> for uart_stop_bits_t {
        fn from(stop_bits: StopBits) -> Self {
            match stop_bits {
                StopBits::STOP1 => uart_stop_bits_t_UART_STOP_BITS_1,
                StopBits::STOP1P5 => uart_stop_bits_t_UART_STOP_BITS_1_5,
                StopBits::STOP2 => uart_stop_bits_t_UART_STOP_BITS_2,
            }
        }
    }

    impl From<uart_stop_bits_t> for StopBits {
        #[allow(non_upper_case_globals)]
        fn from(stop_bits: uart_stop_bits_t) -> Self {
            match stop_bits {
                uart_stop_bits_t_UART_STOP_BITS_1 => StopBits::STOP1,
                uart_stop_bits_t_UART_STOP_BITS_1_5 => StopBits::STOP1P5,
                uart_stop_bits_t_UART_STOP_BITS_2 => StopBits::STOP2,
                _ => unreachable!(),
            }
        }
    }

    /// UART source clock
    //
    // esp-idf 6.0 only exposes SOC_UART_SUPPORT_*_CLK config options for APB,
    // RTC (a.k.a RC_FAST), XTAL, REF_TICK, and surprisingly PLL_F40M (esp32c2 only).
    // For PLL_F48M and PLL_F80M support, we would need to rely on SOC_UART_CLKS array,
    // which is tricky to extract from the bindings, so we hardcode supported chips.
    #[derive(PartialEq, Eq, Copy, Clone, Debug)]
    pub enum SourceClock {
        /// UART source clock from `APB`
        #[cfg(any(
            esp_idf_soc_uart_support_apb_clk,
            esp_idf_soc_uart_support_pll_f40m_clk,
            esp_idf_version_major = "4",
        ))]
        APB,
        /// UART source clock from `RTC`
        #[cfg(esp_idf_soc_uart_support_rtc_clk)]
        RTC,
        /// UART source clock from `XTAL`
        #[cfg(esp_idf_soc_uart_support_xtal_clk)]
        Crystal,
        /// UART source clock from `PLL_F80M`
        #[allow(non_camel_case_types)]
        #[cfg(any(
            esp_idf_soc_uart_support_pll_f80m_clk,
            all(
                esp_idf_version_at_least_6_0_0,
                any(esp32c5, esp32c6, esp32c61, esp32p4, esp32s31)
            )
        ))]
        PLL_F80M,
        /// UART source clock from `PLL_F48M` (ESP32-H2, ESP32-H4, ESP32-H21)
        #[allow(non_camel_case_types)]
        #[cfg(any(
            // IDF < 6: rely on soc caps flags
            all(not(esp_idf_version_at_least_6_0_0), not(any(
                esp_idf_soc_uart_support_apb_clk,
                esp_idf_soc_uart_support_pll_f40m_clk,
                esp_idf_soc_uart_support_pll_f80m_clk,
                esp_idf_soc_uart_support_ref_tick,
                esp_idf_version_major = "4",
            ))),
            // IDF >= 6: only H-series chips have PLL_F48M
            all(esp_idf_version_at_least_6_0_0, any(esp32h2, esp32h4, esp32h21)),
        ))]
        PLL_F48M,
        /// UART source clock from `REF_TICK`
        #[cfg(esp_idf_soc_uart_support_ref_tick)]
        RefTick,
    }

    impl SourceClock {
        pub const fn default() -> Self {
            #[cfg(not(esp_idf_version_major = "4"))]
            const DEFAULT: uart_sclk_t = soc_periph_uart_clk_src_legacy_t_UART_SCLK_DEFAULT;
            #[cfg(esp_idf_version_major = "4")]
            const DEFAULT: uart_sclk_t = uart_sclk_t_UART_SCLK_APB;
            Self::from_raw(DEFAULT)
        }

        pub const fn from_raw(source_clock: uart_sclk_t) -> Self {
            match source_clock {
                #[cfg(any(
                    esp_idf_soc_uart_support_apb_clk,
                    esp_idf_soc_uart_support_pll_f40m_clk,
                    esp_idf_version_major = "4",
                ))]
                APB_SCLK => SourceClock::APB,
                #[cfg(esp_idf_soc_uart_support_rtc_clk)]
                RTC_SCLK => SourceClock::RTC,
                #[cfg(esp_idf_soc_uart_support_xtal_clk)]
                XTAL_SCLK => SourceClock::Crystal,
                #[cfg(any(
                    esp_idf_soc_uart_support_pll_f80m_clk,
                    all(
                        esp_idf_version_at_least_6_0_0,
                        any(esp32c5, esp32c6, esp32c61, esp32p4, esp32s31)
                    )
                ))]
                PLL_F80M_SCLK => SourceClock::PLL_F80M,
                #[cfg(any(
                    all(
                        not(esp_idf_version_at_least_6_0_0),
                        not(any(
                            esp_idf_soc_uart_support_apb_clk,
                            esp_idf_soc_uart_support_pll_f40m_clk,
                            esp_idf_soc_uart_support_pll_f80m_clk,
                            esp_idf_soc_uart_support_ref_tick,
                            esp_idf_version_major = "4",
                        ))
                    ),
                    all(esp_idf_version_at_least_6_0_0, any(esp32h2, esp32h4, esp32h21)),
                ))]
                PLL_F48M_SCLK => SourceClock::PLL_F48M,
                #[cfg(esp_idf_soc_uart_support_ref_tick)]
                REF_TICK_SCLK => SourceClock::RefTick,
                _ => unreachable!(),
            }
        }

        #[cfg(not(esp_idf_version_major = "4"))]
        pub fn frequency(self) -> Result<Hertz, EspError> {
            let mut frequency: u32 = 0;
            esp_result! {
                unsafe { uart_get_sclk_freq(self.into(), &mut frequency) },
                Hertz(frequency)
            }
        }
    }

    #[cfg(all(not(esp_idf_version_major = "4"), esp_idf_soc_uart_support_apb_clk))]
    const APB_SCLK: uart_sclk_t = soc_periph_uart_clk_src_legacy_t_UART_SCLK_APB;
    #[cfg(all(
        not(esp_idf_version_major = "4"),
        not(esp_idf_soc_uart_support_apb_clk),
        esp_idf_soc_uart_support_pll_f40m_clk,
    ))]
    const APB_SCLK: uart_sclk_t = soc_periph_uart_clk_src_legacy_t_UART_SCLK_PLL_F40M;
    #[cfg(esp_idf_version_major = "4")]
    const APB_SCLK: uart_sclk_t = uart_sclk_t_UART_SCLK_APB;

    #[cfg(all(not(esp_idf_version_major = "4"), esp_idf_soc_uart_support_rtc_clk))]
    const RTC_SCLK: uart_sclk_t = soc_periph_uart_clk_src_legacy_t_UART_SCLK_RTC;
    #[cfg(all(esp_idf_version_major = "4", esp_idf_soc_uart_support_rtc_clk))]
    const RTC_SCLK: uart_sclk_t = uart_sclk_t_UART_SCLK_RTC;

    #[cfg(all(not(esp_idf_version_major = "4"), esp_idf_soc_uart_support_xtal_clk))]
    const XTAL_SCLK: uart_sclk_t = soc_periph_uart_clk_src_legacy_t_UART_SCLK_XTAL;
    #[cfg(all(esp_idf_version_major = "4", esp_idf_soc_uart_support_xtal_clk))]
    const XTAL_SCLK: uart_sclk_t = uart_sclk_t_UART_SCLK_XTAL;

    #[cfg(all(
        not(esp_idf_version_major = "4"),
        any(
            esp_idf_soc_uart_support_pll_f80m_clk,
            all(
                esp_idf_version_at_least_6_0_0,
                any(esp32c5, esp32c6, esp32c61, esp32p4, esp32s31)
            )
        )
    ))]
    const PLL_F80M_SCLK: uart_sclk_t = soc_periph_uart_clk_src_legacy_t_UART_SCLK_PLL_F80M;
    #[cfg(all(esp_idf_version_major = "4", esp_idf_soc_uart_support_pll_f80m_clk))]
    const PLL_F80M_SCLK: uart_sclk_t = uart_sclk_t_UART_SCLK_PLL_F80M;

    #[cfg(any(
        all(
            not(esp_idf_version_major = "4"),
            not(esp_idf_version_at_least_6_0_0),
            not(any(
                esp_idf_soc_uart_support_apb_clk,
                esp_idf_soc_uart_support_pll_f40m_clk,
                esp_idf_soc_uart_support_pll_f80m_clk,
                esp_idf_soc_uart_support_ref_tick,
            ))
        ),
        all(esp_idf_version_at_least_6_0_0, any(esp32h2, esp32h4, esp32h21)),
    ))]
    const PLL_F48M_SCLK: uart_sclk_t = soc_periph_uart_clk_src_legacy_t_UART_SCLK_PLL_F48M;

    #[cfg(all(not(esp_idf_version_major = "4"), esp_idf_soc_uart_support_ref_tick))]
    const REF_TICK_SCLK: uart_sclk_t = soc_periph_uart_clk_src_legacy_t_UART_SCLK_REF_TICK;
    #[cfg(all(esp_idf_version_major = "4", esp_idf_soc_uart_support_ref_tick))]
    const REF_TICK_SCLK: uart_sclk_t = uart_sclk_t_UART_SCLK_REF_TICK;

    impl Default for SourceClock {
        fn default() -> Self {
            SourceClock::default()
        }
    }

    impl From<SourceClock> for uart_sclk_t {
        fn from(source_clock: SourceClock) -> Self {
            match source_clock {
                #[cfg(any(
                    esp_idf_soc_uart_support_apb_clk,
                    esp_idf_soc_uart_support_pll_f40m_clk,
                    esp_idf_version_major = "4",
                ))]
                SourceClock::APB => APB_SCLK,
                #[cfg(esp_idf_soc_uart_support_rtc_clk)]
                SourceClock::RTC => RTC_SCLK,
                #[cfg(esp_idf_soc_uart_support_xtal_clk)]
                SourceClock::Crystal => XTAL_SCLK,
                #[cfg(any(
                    esp_idf_soc_uart_support_pll_f80m_clk,
                    all(
                        esp_idf_version_at_least_6_0_0,
                        any(esp32c5, esp32c6, esp32c61, esp32p4, esp32s31)
                    )
                ))]
                SourceClock::PLL_F80M => PLL_F80M_SCLK,
                #[cfg(any(
                    all(
                        not(esp_idf_version_at_least_6_0_0),
                        not(any(
                            esp_idf_soc_uart_support_apb_clk,
                            esp_idf_soc_uart_support_pll_f40m_clk,
                            esp_idf_soc_uart_support_pll_f80m_clk,
                            esp_idf_soc_uart_support_ref_tick,
                            esp_idf_version_major = "4",
                        ))
                    ),
                    all(esp_idf_version_at_least_6_0_0, any(esp32h2, esp32h4, esp32h21)),
                ))]
                SourceClock::PLL_F48M => PLL_F48M_SCLK,
                #[cfg(esp_idf_soc_uart_support_ref_tick)]
                SourceClock::RefTick => REF_TICK_SCLK,
            }
        }
    }

    impl From<uart_sclk_t> for SourceClock {
        fn from(source_clock: uart_sclk_t) -> Self {
            Self::from_raw(source_clock)
        }
    }

    /// Configures the interrupts the UART driver should enable
    /// in order to be able to quickly inform us about the
    /// related event.
    #[derive(Debug, Clone)]
    pub struct EventConfig {
        /// If `Some(number_of_words)`, an interrupt will trigger
        /// after `number_of_words` could have been transmitted
        /// (unit is baudrate dependant).
        ///
        /// If `None` or `Some(0)` interrupt will be disabled.
        pub receive_timeout: Option<u8>,
        /// Sets the threshold at which an interrupt will
        /// be generated (the hardware receive FIFO contains more words than
        /// this number).
        ///
        /// If set to `None` interrupt will be disabled.
        pub rx_fifo_full: Option<u8>,
        /// Sets the threshold **below** which an interrupt will
        /// be generated (the hardware transmit FIFO contains less words than
        /// this number).
        ///
        /// If set to `None` interrupt will be disabled.
        /// Should not be set to `0` as the interrupt will trigger constantly.
        pub tx_fifo_empty: Option<u8>,
        /// Other interrupts to enable
        pub flags: EnumSet<EventFlags>,
        /// Allow using struct syntax,
        /// but signal users other fields may be added
        /// so `..Default::default()` should be used.
        #[doc(hidden)]
        pub _non_exhaustive: (),
    }

    impl EventConfig {
        pub const fn new() -> Self {
            EventConfig {
                receive_timeout: Some(10),
                rx_fifo_full: Some(120),
                tx_fifo_empty: Some(10),
                flags: enum_set!(
                    EventFlags::RxFifoFull
                        | EventFlags::RxFifoTimeout
                        | EventFlags::RxFifoOverflow
                        | EventFlags::BreakDetected
                        | EventFlags::ParityError
                ),
                _non_exhaustive: (),
            }
        }
    }

    impl Default for EventConfig {
        fn default() -> Self {
            EventConfig::new()
        }
    }

    impl From<EventConfig> for crate::sys::uart_intr_config_t {
        fn from(cfg: EventConfig) -> Self {
            let mut intr_enable_mask = cfg.flags;

            if cfg.receive_timeout.map(|to| to > 0).unwrap_or(false) {
                intr_enable_mask.insert(EventFlags::RxFifoTimeout);
            } else {
                intr_enable_mask.remove(EventFlags::RxFifoTimeout);
            }

            if cfg.rx_fifo_full.is_some() {
                intr_enable_mask.insert(EventFlags::RxFifoFull);
            } else {
                intr_enable_mask.remove(EventFlags::RxFifoFull);
            }

            if cfg.tx_fifo_empty.is_some() {
                intr_enable_mask.insert(EventFlags::TxFifoEmpty);
            } else {
                intr_enable_mask.remove(EventFlags::TxFifoEmpty);
            }

            crate::sys::uart_intr_config_t {
                intr_enable_mask: intr_enable_mask.as_repr(),
                rx_timeout_thresh: cfg.receive_timeout.unwrap_or(0),
                txfifo_empty_intr_thresh: cfg.tx_fifo_empty.unwrap_or(0),
                rxfifo_full_thresh: cfg.rx_fifo_full.unwrap_or(0),
            }
        }
    }

    #[derive(Debug, EnumSetType)]
    #[enumset(repr = "u32")]
    #[non_exhaustive]
    pub enum EventFlags {
        #[doc(hidden)]
        RxFifoFull = 0,
        #[doc(hidden)]
        TxFifoEmpty = 1,
        ParityError = 2,
        FrameError = 3,
        RxFifoOverflow = 4,
        DsrChange = 5,
        CtsChange = 6,
        BreakDetected = 7,
        #[doc(hidden)]
        RxFifoTimeout = 8,
        SwXon = 9,
        SwXoff = 10,
        GlitchDetected = 11,
        TxBreakDone = 12,
        TxBreakIdle = 13,
        TxDone = 14,
        Rs485ParityError = 15,
        Rs485FrameError = 16,
        Rs485Clash = 17,
        CmdCharDetected = 18,
    }

    /// UART configuration
    #[derive(Debug, Clone)]
    pub struct Config {
        pub mode: Mode,
        pub baudrate: Hertz,
        pub data_bits: DataBits,
        pub parity: Parity,
        pub stop_bits: StopBits,
        pub flow_control: FlowControl,
        pub flow_control_rts_threshold: u8,
        pub source_clock: SourceClock,
        /// Configures the flags to use for interrupt allocation,
        /// e.g. priority to use for the interrupt.
        ///
        /// Note that you should not set `Iram` here, because it will
        /// be automatically set depending on the value of `CONFIG_UART_ISR_IN_IRAM`.
        pub intr_flags: EnumSet<InterruptType>,
        /// Configures the interrupts the driver should enable.
        pub event_config: EventConfig,
        /// The size of the software rx buffer. Must be bigger than the hardware FIFO.
        pub rx_fifo_size: usize,
        /// The size of the software tx buffer. Must be bigger than the hardware FIFO
        /// or 0 to disable transmit buffering (note that this will make write operations
        /// block until data has been sent out).
        pub tx_fifo_size: usize,
        /// Number of events that should fit into the event queue.
        /// Specify 0 to prevent the creation of an event queue.
        pub queue_size: usize,
        /// Allow using struct syntax,
        /// but signal users other fields may be added
        /// so `..Default::default()` should be used.
        #[doc(hidden)]
        pub _non_exhaustive: (),
    }

    impl From<&Config> for uart_config_t {
        fn from(config: &Config) -> Self {
            #[allow(clippy::needless_update)]
            Self {
                baud_rate: config.baudrate.0 as i32,
                data_bits: config.data_bits.into(),
                parity: config.parity.into(),
                stop_bits: config.stop_bits.into(),
                flow_ctrl: config.flow_control.into(),
                rx_flow_ctrl_thresh: config.flow_control_rts_threshold,
                // ESP-IDF 5.0 and 5.1
                #[cfg(all(
                    esp_idf_version_major = "5",
                    any(esp_idf_version_minor = "0", esp_idf_version_minor = "1")
                ))]
                source_clk: config.source_clock.into(),
                // All others
                #[cfg(not(all(
                    esp_idf_version_major = "5",
                    any(esp_idf_version_minor = "0", esp_idf_version_minor = "1")
                )))]
                __bindgen_anon_1: uart_config_t__bindgen_ty_1 {
                    source_clk: config.source_clock.into(),
                },
                ..Default::default()
            }
        }
    }

    impl Config {
        pub const fn new() -> Config {
            Config {
                mode: Mode::UART,
                baudrate: Hertz(115_200),
                data_bits: DataBits::DataBits8,
                parity: Parity::ParityNone,
                stop_bits: StopBits::STOP1,
                flow_control: FlowControl::None,
                flow_control_rts_threshold: 122,
                source_clock: SourceClock::default(),
                intr_flags: EnumSet::empty(),
                event_config: EventConfig::new(),
                rx_fifo_size: super::UART_FIFO_SIZE * 2,
                tx_fifo_size: super::UART_FIFO_SIZE * 2,
                queue_size: 10,
                _non_exhaustive: (),
            }
        }

        #[must_use]
        pub fn mode(mut self, mode: Mode) -> Self {
            self.mode = mode;
            self
        }

        #[must_use]
        pub fn baudrate(mut self, baudrate: Hertz) -> Self {
            self.baudrate = baudrate;
            self
        }

        #[must_use]
        pub fn parity_none(mut self) -> Self {
            self.parity = Parity::ParityNone;
            self
        }

        #[must_use]
        pub fn parity_even(mut self) -> Self {
            self.parity = Parity::ParityEven;
            self
        }

        #[must_use]
        pub fn parity_odd(mut self) -> Self {
            self.parity = Parity::ParityOdd;
            self
        }

        #[must_use]
        pub fn data_bits(mut self, data_bits: DataBits) -> Self {
            self.data_bits = data_bits;
            self
        }

        #[must_use]
        pub fn stop_bits(mut self, stop_bits: StopBits) -> Self {
            self.stop_bits = stop_bits;
            self
        }

        #[must_use]
        pub fn flow_control(mut self, flow_control: FlowControl) -> Self {
            self.flow_control = flow_control;
            self
        }

        #[must_use]
        /// This setting only has effect if flow control is enabled.
        /// It determines how many bytes must be received before `RTS` line is asserted.
        /// Notice that count starts from `0` which means that `RTS` is asserted after every received byte.
        pub fn flow_control_rts_threshold(mut self, flow_control_rts_threshold: u8) -> Self {
            self.flow_control_rts_threshold = flow_control_rts_threshold;
            self
        }

        #[must_use]
        pub fn source_clock(mut self, source_clock: SourceClock) -> Self {
            self.source_clock = source_clock;
            self
        }

        #[must_use]
        pub fn tx_fifo_size(mut self, tx_fifo_size: usize) -> Self {
            self.tx_fifo_size = tx_fifo_size;
            self
        }

        #[must_use]
        pub fn rx_fifo_size(mut self, rx_fifo_size: usize) -> Self {
            self.rx_fifo_size = rx_fifo_size;
            self
        }

        #[must_use]
        pub fn queue_size(mut self, queue_size: usize) -> Self {
            self.queue_size = queue_size;
            self
        }
    }

    impl Default for Config {
        fn default() -> Config {
            Config::new()
        }
    }
}

pub trait Uart {
    fn port() -> uart_port_t;
}

crate::embedded_hal_error!(
    SerialError,
    embedded_hal_nb::serial::Error,
    embedded_hal_nb::serial::ErrorKind
);

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct UartEvent {
    raw: uart_event_t,
}

impl UartEvent {
    pub fn payload(&self) -> UartEventPayload {
        #[allow(non_upper_case_globals)]
        match self.raw.type_ {
            uart_event_type_t_UART_DATA => UartEventPayload::Data {
                size: self.raw.size,
                timeout: self.raw.timeout_flag,
            },
            uart_event_type_t_UART_BREAK => UartEventPayload::Break,
            uart_event_type_t_UART_BUFFER_FULL => UartEventPayload::RxBufferFull,
            uart_event_type_t_UART_FIFO_OVF => UartEventPayload::RxFifoOverflow,
            uart_event_type_t_UART_FRAME_ERR => UartEventPayload::FrameError,
            uart_event_type_t_UART_PARITY_ERR => UartEventPayload::ParityError,
            uart_event_type_t_UART_DATA_BREAK => UartEventPayload::DataBreak,
            uart_event_type_t_UART_PATTERN_DET => UartEventPayload::PatternDetected,
            _ => UartEventPayload::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum UartEventPayload {
    /// UART data was received and/or a timeout was triggered
    Data {
        /// The number of bytes received
        size: usize,
        /// Whether a timeout has occurred.
        /// It is possible that bytes have been received
        /// and this is set to `true` in case the driver
        /// processed both interrupts at the same time.
        timeout: bool,
    },
    /// Represents DATA event with timeout_flag set
    Break,
    RxBufferFull,
    RxFifoOverflow,
    FrameError,
    ParityError,
    DataBreak,
    PatternDetected,
    Unknown,
}

/// Serial abstraction
pub struct UartDriver<'d> {
    port: u8,
    queue: Option<Queue<UartEvent>>,
    _p: PhantomData<&'d mut ()>,
}

unsafe impl Send for UartDriver<'_> {}
unsafe impl Sync for UartDriver<'_> {}

/// Serial receiver
pub struct UartRxDriver<'d> {
    port: u8,
    owner: Owner,
    queue: Option<Queue<UartEvent>>,
    _p: PhantomData<&'d mut ()>,
}

/// Serial transmitter
pub struct UartTxDriver<'d> {
    port: u8,
    owner: Owner,
    queue: Option<Queue<UartEvent>>,
    _p: PhantomData<&'d mut ()>,
}

impl<'d> UartDriver<'d> {
    /// Create a new serial driver
    pub fn new<UART: Uart + 'd>(
        uart: UART,
        tx: impl OutputPin + 'd,
        rx: impl InputPin + 'd,
        cts: Option<impl InputPin + 'd>,
        rts: Option<impl OutputPin + 'd>,
        config: &config::Config,
    ) -> Result<Self, EspError> {
        let mut q_handle_raw = ptr::null_mut();
        let q_handle = if config.queue_size > 0 {
            Some(&mut q_handle_raw)
        } else {
            None
        };
        if let Err(err) = new_common(uart, Some(tx), Some(rx), cts, rts, config, q_handle) {
            // Roll back driver registration on failure to avoid dangling driver state
            if let Err(e) = delete_driver(UART::port() as _) {
                ::log::error!("Failed to delete UART driver: {}", e);
            }

            return Err(err);
        }

        // SAFTEY: okay because Queue borrows self
        // SAFETY: we can safely use UartEvent instead of uart_event_t because of repr(transparent)
        let queue = match q_handle_raw.is_null() {
            false => Some(unsafe { Queue::new_borrowed(q_handle_raw) }),
            true => None,
        };

        Ok(Self {
            port: UART::port() as _,
            queue,
            _p: PhantomData,
        })
    }

    /// Retrieves the event queue for this UART. Returns `None` if
    /// the config specified 0 for `queue_size`.
    pub fn event_queue(&self) -> Option<&Queue<UartEvent>> {
        self.queue.as_ref()
    }

    /// Change the number of stop bits
    pub fn change_stop_bits(&self, stop_bits: config::StopBits) -> Result<&Self, EspError> {
        change_stop_bits(self.port(), stop_bits).map(|_| self)
    }

    /// Returns the current number of stop bits
    pub fn stop_bits(&self) -> Result<config::StopBits, EspError> {
        stop_bits(self.port())
    }

    /// Change the number of data bits
    pub fn change_data_bits(&self, data_bits: config::DataBits) -> Result<&Self, EspError> {
        change_data_bits(self.port(), data_bits).map(|_| self)
    }

    /// Return the current number of data bits
    pub fn data_bits(&self) -> Result<config::DataBits, EspError> {
        data_bits(self.port())
    }

    /// Change the type of parity checking
    pub fn change_parity(&self, parity: config::Parity) -> Result<&Self, EspError> {
        change_parity(self.port(), parity).map(|_| self)
    }

    /// Returns the current type of parity checking
    pub fn parity(&self) -> Result<config::Parity, EspError> {
        parity(self.port())
    }

    /// Change the baudrate.
    ///
    /// Will automatically select the clock source. When possible the reference clock (1MHz) will
    /// be used, because this is constant when the clock source/frequency changes.
    /// However if one of the clock frequencies is below 10MHz or if the baudrate is above
    /// the reference clock or if the baudrate cannot be set within 1.5%
    /// then use the APB clock.
    pub fn change_baudrate<T: Into<Hertz> + Copy>(&self, baudrate: T) -> Result<&Self, EspError> {
        change_baudrate(self.port(), baudrate).map(|_| self)
    }

    /// Returns the current baudrate
    pub fn baudrate(&self) -> Result<Hertz, EspError> {
        baudrate(self.port())
    }

    /// Split the serial driver in separate TX and RX drivers
    pub fn split(&mut self) -> (UartTxDriver<'_>, UartRxDriver<'_>) {
        (
            UartTxDriver {
                port: self.port,
                owner: Owner::Borrowed,
                queue: self
                    .queue
                    .as_ref()
                    .map(|queue| unsafe { Queue::new_borrowed(queue.as_raw()) }),
                _p: PhantomData,
            },
            UartRxDriver {
                port: self.port,
                owner: Owner::Borrowed,
                queue: self
                    .queue
                    .as_ref()
                    .map(|queue| unsafe { Queue::new_borrowed(queue.as_raw()) }),
                _p: PhantomData,
            },
        )
    }

    /// Split the serial driver in separate TX and RX drivers.
    ///
    /// Unlike [`split`], the halves are owned and reference counted.
    pub fn into_split(self) -> (UartTxDriver<'d>, UartRxDriver<'d>) {
        let port = self.port;
        let tx_queue = self
            .queue
            .as_ref()
            .map(|queue| unsafe { Queue::new_borrowed(queue.as_raw()) });
        let rx_queue = self
            .queue
            .as_ref()
            .map(|queue| unsafe { Queue::new_borrowed(queue.as_raw()) });
        let _ = ManuallyDrop::new(self);
        REFS[port as usize].fetch_add(2, Ordering::SeqCst);
        (
            UartTxDriver {
                port,
                owner: Owner::Shared,
                queue: tx_queue,
                _p: PhantomData,
            },
            UartRxDriver {
                port,
                owner: Owner::Shared,
                queue: rx_queue,
                _p: PhantomData,
            },
        )
    }

    /// Read multiple bytes into a slice
    pub fn read(&self, buf: &mut [u8], timeout: TickType_t) -> Result<usize, EspError> {
        self.rx().read(buf, timeout)
    }

    /// Write multiple bytes from a slice
    pub fn write(&self, bytes: &[u8]) -> Result<usize, EspError> {
        self.tx().write(bytes)
    }

    /// Write multiple bytes from a slice, then send a break condition.
    pub fn write_with_break(&self, bytes: &[u8], brk_len: i32) -> Result<usize, EspError> {
        self.tx().write_with_break(bytes, brk_len)
    }

    /// Write multiple bytes from a slice directly to the TX FIFO hardware.
    /// Returns the number of bytes written, where 0 would mean that the TX FIFO is full.
    ///
    /// NOTE: In case the UART TX buffer is enabled, this method might have unpredictable results
    /// when used together with method `write`, as the latter will push the data to be sent to the
    /// TX buffer first.
    ///
    /// To avoid this, always call `wait_done` after the last call to `write` and before
    /// calling this method.
    pub fn write_nb(&self, bytes: &[u8]) -> Result<usize, EspError> {
        self.tx().write_nb(bytes)
    }

    /// Clears the receive buffer.
    #[deprecated(since = "0.41.3", note = "Use UartDriver::clear_rx instead")]
    pub fn flush_read(&self) -> Result<(), EspError> {
        self.rx().clear()
    }

    /// Clears the receive buffer.
    pub fn clear_rx(&self) -> Result<(), EspError> {
        self.rx().clear()
    }

    /// Waits for the transmission to complete.
    #[deprecated(since = "0.41.3", note = "Use UartDriver::wait_tx_done instead")]
    pub fn flush_write(&self) -> Result<(), EspError> {
        self.tx().wait_done(delay::BLOCK)
    }

    /// Waits until the transmission is complete or until the specified timeout expires.
    pub fn wait_tx_done(&self, timeout: TickType_t) -> Result<(), EspError> {
        self.tx().wait_done(timeout)
    }

    pub fn port(&self) -> uart_port_t {
        self.port as _
    }

    /// Get count of remaining bytes in the receive ring buffer
    pub fn remaining_read(&self) -> Result<usize, EspError> {
        remaining_unread_bytes(self.port())
    }

    /// Get count of remaining capacity in the transmit ring buffer
    #[cfg(any(
        not(esp_idf_version_major = "4"),
        all(
            esp_idf_version_minor = "4",
            not(any(esp_idf_version_patch = "0", esp_idf_version_patch = "1")),
        ),
    ))]
    pub fn remaining_write(&self) -> Result<usize, EspError> {
        remaining_write_capacity(self.port())
    }

    fn rx(&self) -> ManuallyDrop<UartRxDriver<'_>> {
        ManuallyDrop::new(UartRxDriver {
            port: self.port,
            owner: Owner::Borrowed,
            queue: self
                .queue
                .as_ref()
                .map(|queue| unsafe { Queue::new_borrowed(queue.as_raw()) }),
            _p: PhantomData,
        })
    }

    fn tx(&self) -> ManuallyDrop<UartTxDriver<'_>> {
        ManuallyDrop::new(UartTxDriver {
            port: self.port,
            owner: Owner::Borrowed,
            queue: self
                .queue
                .as_ref()
                .map(|queue| unsafe { Queue::new_borrowed(queue.as_raw()) }),
            _p: PhantomData,
        })
    }
}

impl Drop for UartDriver<'_> {
    fn drop(&mut self) {
        delete_driver(self.port()).unwrap();
    }
}

impl embedded_io::ErrorType for UartDriver<'_> {
    type Error = EspIOError;
}

impl embedded_io::Read for UartDriver<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        UartDriver::read(self, buf, delay::BLOCK).map_err(EspIOError)
    }
}

impl embedded_io::Write for UartDriver<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        UartDriver::write(self, buf).map_err(EspIOError)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        UartDriver::wait_tx_done(self, delay::BLOCK).map_err(EspIOError)
    }
}

impl embedded_hal_0_2::serial::Read<u8> for UartDriver<'_> {
    type Error = SerialError;

    fn read(&mut self) -> nb::Result<u8, Self::Error> {
        embedded_hal_0_2::serial::Read::read(&mut *self.rx())
    }
}

impl embedded_hal_0_2::serial::Write<u8> for UartDriver<'_> {
    type Error = SerialError;

    fn flush(&mut self) -> nb::Result<(), Self::Error> {
        embedded_hal_0_2::serial::Write::flush(&mut *self.tx())
    }

    fn write(&mut self, byte: u8) -> nb::Result<(), Self::Error> {
        embedded_hal_0_2::serial::Write::write(&mut *self.tx(), byte)
    }
}

impl embedded_hal_nb::serial::ErrorType for UartDriver<'_> {
    type Error = SerialError;
}

impl embedded_hal_nb::serial::Read<u8> for UartDriver<'_> {
    fn read(&mut self) -> nb::Result<u8, Self::Error> {
        embedded_hal_nb::serial::Read::read(&mut *self.rx())
    }
}

impl embedded_hal_nb::serial::Write<u8> for UartDriver<'_> {
    fn write(&mut self, byte: u8) -> nb::Result<(), Self::Error> {
        embedded_hal_nb::serial::Write::write(&mut *self.tx(), byte)
    }

    fn flush(&mut self) -> nb::Result<(), Self::Error> {
        embedded_hal_nb::serial::Write::flush(&mut *self.tx())
    }
}

impl core::fmt::Write for UartDriver<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.tx().write_str(s)
    }
}

impl<'d> UartRxDriver<'d> {
    /// Create a new serial receiver
    pub fn new<UART: Uart + 'd>(
        uart: UART,
        rx: impl InputPin + 'd,
        cts: Option<impl InputPin + 'd>,
        rts: Option<impl OutputPin + 'd>,
        config: &config::Config,
    ) -> Result<Self, EspError> {
        let mut q_handle_raw = ptr::null_mut();
        let q_handle = if config.queue_size > 0 {
            Some(&mut q_handle_raw)
        } else {
            None
        };
        new_common(
            uart,
            None::<AnyOutputPin>,
            Some(rx),
            cts,
            rts,
            config,
            q_handle,
        )?;

        // SAFTEY: okay because Queue borrows self
        // SAFETY: we can safely use UartEvent instead of uart_event_t because of repr(transparent)
        let queue = match q_handle_raw.is_null() {
            false => Some(unsafe { Queue::new_borrowed(q_handle_raw) }),
            true => None,
        };

        Ok(Self {
            port: UART::port() as _,
            owner: Owner::Owned,
            queue,
            _p: PhantomData,
        })
    }

    /// Retrieves the event queue for this UART. Returns `None` if
    /// the config specified 0 for `queue_size`.
    pub fn event_queue(&self) -> Option<&Queue<UartEvent>> {
        self.queue.as_ref()
    }

    /// Change the number of stop bits
    pub fn change_stop_bits(&self, stop_bits: config::StopBits) -> Result<&Self, EspError> {
        change_stop_bits(self.port(), stop_bits).map(|_| self)
    }

    /// Returns the current number of stop bits
    pub fn stop_bits(&self) -> Result<config::StopBits, EspError> {
        stop_bits(self.port())
    }

    /// Change the number of data bits
    pub fn change_data_bits(&self, data_bits: config::DataBits) -> Result<&Self, EspError> {
        change_data_bits(self.port(), data_bits).map(|_| self)
    }

    /// Return the current number of data bits
    pub fn data_bits(&self) -> Result<config::DataBits, EspError> {
        data_bits(self.port())
    }

    /// Change the type of parity checking
    pub fn change_parity(&self, parity: config::Parity) -> Result<&Self, EspError> {
        change_parity(self.port(), parity).map(|_| self)
    }

    /// Returns the current type of parity checking
    pub fn parity(&self) -> Result<config::Parity, EspError> {
        parity(self.port())
    }

    /// Change the baudrate.
    ///
    /// Will automatically select the clock source. When possible the reference clock (1MHz) will
    /// be used, because this is constant when the clock source/frequency changes.
    /// However if one of the clock frequencies is below 10MHz or if the baudrate is above
    /// the reference clock or if the baudrate cannot be set within 1.5%
    /// then use the APB clock.
    pub fn change_baudrate<T: Into<Hertz> + Copy>(&self, baudrate: T) -> Result<&Self, EspError> {
        change_baudrate(self.port(), baudrate).map(|_| self)
    }

    /// Returns the current baudrate
    pub fn baudrate(&self) -> Result<Hertz, EspError> {
        baudrate(self.port())
    }

    /// Read multiple bytes into a slice; block until specified timeout
    /// Returns:
    /// - `Ok(0)` if the buffer is of length 0
    /// - `Ok(n)` if `n` bytes were read, where n is > 0
    /// - `Err(EspError::Timeout)` if no bytes were read within the specified timeout
    pub fn read(&self, buf: &mut [u8], delay: TickType_t) -> Result<usize, EspError> {
        // `uart_read_bytes` has a WEIRD semantics:
        // - If the data in the internal ring-buffer is LESS than the passed `length`
        //   **it will wait (with a `delay` timeout) UNTIL it can return up to `length` bytes**
        //   (and if the timeout had expired, it will return whatever it was able to read - possibly nothing too)
        // - This is not matching the typical `read` syscall semantics where it only
        //   returns what is available in the internal buffer and does not wait for more;
        //   and only blocks if the internal buffer is empty, and only until _some_ data becomes available
        //   but NOT until `buf.len()` data is available.
        //
        // Therefore - and to avoid confusion - we will implement the typical `read` syscall
        // semantics here

        // Passing an empty buffer is valid, but it means we'll always read 0 bytes
        if buf.is_empty() {
            return Ok(0);
        }

        // First try to read without blocking
        let len = unsafe {
            uart_read_bytes(
                self.port(),
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                delay::NON_BLOCK,
            )
        };

        if len > 0 || delay == delay::NON_BLOCK {
            // Some data was read, or the user requested a non-blocking read anyway
            return match len {
                -1 | 0 => Err(EspError::from_infallible::<ESP_ERR_TIMEOUT>()),
                len => Ok(len as usize),
            };
        }

        // Now block until at least one byte is available
        let mut len =
            unsafe { uart_read_bytes(self.port(), buf.as_mut_ptr().cast(), 1_u32, delay) };

        if len > 0 && buf.len() > 1 {
            // Try to read more than that one byte in a non-blocking way
            // just because we can, and this lowers the latency of `read`.
            // To comply with the `read` syscall semantics we don't have to necessarily do this
            let extra_len = unsafe {
                uart_read_bytes(
                    self.port(),
                    buf[1..].as_mut_ptr().cast(),
                    (buf.len() - 1) as u32,
                    delay::NON_BLOCK,
                )
            };

            if extra_len > 0 {
                len += extra_len;
            }
        }

        match len {
            -1 | 0 => Err(EspError::from_infallible::<ESP_ERR_TIMEOUT>()),
            len => Ok(len as usize),
        }
    }

    /// Clears the receive buffer.
    #[deprecated(since = "0.41.3", note = "Use `UartRxDriver::clear` instead")]
    pub fn flush(&self) -> Result<(), EspError> {
        self.clear()
    }

    pub fn clear(&self) -> Result<(), EspError> {
        esp!(unsafe { uart_flush_input(self.port()) })?;

        Ok(())
    }

    pub fn port(&self) -> uart_port_t {
        self.port as _
    }

    /// Get count of remaining bytes in the receive ring buffer
    pub fn count(&self) -> Result<usize, EspError> {
        remaining_unread_bytes(self.port())
    }
}

impl Drop for UartRxDriver<'_> {
    fn drop(&mut self) {
        self.owner.drop_impl(self.port()).unwrap()
    }
}

impl embedded_io::ErrorType for UartRxDriver<'_> {
    type Error = EspIOError;
}

impl embedded_io::Read for UartRxDriver<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        UartRxDriver::read(self, buf, delay::BLOCK).map_err(EspIOError)
    }
}

impl embedded_hal_0_2::serial::Read<u8> for UartRxDriver<'_> {
    type Error = SerialError;

    fn read(&mut self) -> nb::Result<u8, Self::Error> {
        let mut buf = [0_u8];

        let result = UartRxDriver::read(self, &mut buf, NON_BLOCK);

        check_nb(result, buf[0])
    }
}

impl embedded_hal_nb::serial::ErrorType for UartRxDriver<'_> {
    type Error = SerialError;
}

impl embedded_hal_nb::serial::Read<u8> for UartRxDriver<'_> {
    fn read(&mut self) -> nb::Result<u8, Self::Error> {
        let mut buf = [0_u8];

        let result = UartRxDriver::read(self, &mut buf, NON_BLOCK);

        check_nb(result, buf[0])
    }
}

impl<'d> UartTxDriver<'d> {
    /// Create a new serial transmitter
    pub fn new<UART: Uart + 'd>(
        uart: UART,
        tx: impl OutputPin + 'd,
        cts: Option<impl InputPin + 'd>,
        rts: Option<impl OutputPin + 'd>,
        config: &config::Config,
    ) -> Result<Self, EspError> {
        let mut q_handle_raw = ptr::null_mut();
        let q_handle = if config.queue_size > 0 {
            Some(&mut q_handle_raw)
        } else {
            None
        };
        new_common(
            uart,
            Some(tx),
            None::<AnyInputPin>,
            cts,
            rts,
            config,
            q_handle,
        )?;

        // SAFTEY: okay because Queue borrows self
        // SAFETY: we can safely use UartEvent instead of uart_event_t because of repr(transparent)
        let queue = match q_handle_raw.is_null() {
            false => Some(unsafe { Queue::new_borrowed(q_handle_raw) }),
            true => None,
        };

        Ok(Self {
            port: UART::port() as _,
            owner: Owner::Owned,
            queue,
            _p: PhantomData,
        })
    }

    /// Retrieves the event queue for this UART. Returns `None` if
    /// the config specified 0 for `queue_size`.
    pub fn event_queue(&self) -> Option<&Queue<UartEvent>> {
        self.queue.as_ref()
    }

    /// Change the number of stop bits
    pub fn change_stop_bits(&self, stop_bits: config::StopBits) -> Result<&Self, EspError> {
        change_stop_bits(self.port(), stop_bits).map(|_| self)
    }

    /// Returns the current number of stop bits
    pub fn stop_bits(&self) -> Result<config::StopBits, EspError> {
        stop_bits(self.port())
    }

    /// Change the number of data bits
    pub fn change_data_bits(&self, data_bits: config::DataBits) -> Result<&Self, EspError> {
        change_data_bits(self.port(), data_bits).map(|_| self)
    }

    /// Return the current number of data bits
    pub fn data_bits(&self) -> Result<config::DataBits, EspError> {
        data_bits(self.port())
    }

    /// Change the type of parity checking
    pub fn change_parity(&self, parity: config::Parity) -> Result<&Self, EspError> {
        change_parity(self.port(), parity).map(|_| self)
    }

    /// Returns the current type of parity checking
    pub fn parity(&self) -> Result<config::Parity, EspError> {
        parity(self.port())
    }

    /// Change the baudrate.
    ///
    /// Will automatically select the clock source. When possible the reference clock (1MHz) will
    /// be used, because this is constant when the clock source/frequency changes.
    /// However if one of the clock frequencies is below 10MHz or if the baudrate is above
    /// the reference clock or if the baudrate cannot be set within 1.5%
    /// then use the APB clock.
    pub fn change_baudrate<T: Into<Hertz> + Copy>(&self, baudrate: T) -> Result<&Self, EspError> {
        change_baudrate(self.port(), baudrate).map(|_| self)
    }

    /// Returns the current baudrate
    pub fn baudrate(&self) -> Result<Hertz, EspError> {
        baudrate(self.port())
    }

    /// Write multiple bytes from a slice
    pub fn write(&mut self, bytes: &[u8]) -> Result<usize, EspError> {
        // `uart_write_bytes()` returns error (-1) or how many bytes were written
        let len = unsafe { uart_write_bytes(self.port(), bytes.as_ptr().cast(), bytes.len()) };

        if len >= 0 {
            Ok(len as usize)
        } else {
            Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())
        }
    }

    /// Write multiple bytes from a slice, then send a break condition.
    pub fn write_with_break(&mut self, bytes: &[u8], brk_len: i32) -> Result<usize, EspError> {
        // `uart_write_bytes_with_break()` returns error (-1) or how many bytes were written
        let len = unsafe {
            uart_write_bytes_with_break(self.port(), bytes.as_ptr().cast(), bytes.len(), brk_len)
        };

        if len >= 0 {
            Ok(len as usize)
        } else {
            Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())
        }
    }

    /// Write multiple bytes from a slice directly to the TX FIFO hardware.
    /// Returns the number of bytes written, where 0 would mean that the TX FIFO is full.
    ///
    /// NOTE: In case the UART TX buffer is enabled, this method might have unpredictable results
    /// when used together with method `write`, as the latter will push the data to be sent to the
    /// TX buffer first.
    ///
    /// To avoid this, always call `wait_done` after the last call to `write` and before
    /// calling this method.
    pub fn write_nb(&self, bytes: &[u8]) -> Result<usize, EspError> {
        let ret = unsafe { uart_tx_chars(self.port(), bytes.as_ptr().cast(), bytes.len() as _) };

        if ret < 0 {
            esp!(ret)?;
        }

        Ok(ret as usize)
    }

    /// Waits until the transmission is complete or until the specified timeout expires.
    pub fn wait_done(&self, timeout: TickType_t) -> Result<(), EspError> {
        esp!(unsafe { uart_wait_tx_done(self.port(), timeout) })?;

        Ok(())
    }

    /// Waits until the transmission is complete.
    #[deprecated(since = "0.41.3", note = "Use `UartTxDriver::wait_done` instead")]
    pub fn flush(&mut self) -> Result<(), EspError> {
        self.wait_done(delay::BLOCK)
    }

    pub fn port(&self) -> uart_port_t {
        self.port as _
    }

    /// Get count of remaining capacity in the transmit ring buffer
    #[cfg(any(
        not(esp_idf_version_major = "4"),
        all(
            esp_idf_version_minor = "4",
            not(any(esp_idf_version_patch = "0", esp_idf_version_patch = "1")),
        ),
    ))]
    pub fn count(&self) -> Result<usize, EspError> {
        remaining_write_capacity(self.port())
    }
}

impl Drop for UartTxDriver<'_> {
    fn drop(&mut self) {
        self.owner.drop_impl(self.port()).unwrap()
    }
}

impl embedded_io::Write for UartTxDriver<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        UartTxDriver::write(self, buf).map_err(EspIOError)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        UartTxDriver::wait_done(self, delay::BLOCK).map_err(EspIOError)
    }
}

impl embedded_io::ErrorType for UartTxDriver<'_> {
    type Error = EspIOError;
}

impl embedded_hal_0_2::serial::Write<u8> for UartTxDriver<'_> {
    type Error = SerialError;

    fn flush(&mut self) -> nb::Result<(), Self::Error> {
        check_nb_timeout(UartTxDriver::wait_done(self, delay::NON_BLOCK))
    }

    fn write(&mut self, byte: u8) -> nb::Result<(), Self::Error> {
        check_nb(UartTxDriver::write_nb(self, &[byte]), ())
    }
}

impl embedded_hal_nb::serial::ErrorType for UartTxDriver<'_> {
    type Error = SerialError;
}

impl embedded_hal_nb::serial::Write<u8> for UartTxDriver<'_> {
    fn flush(&mut self) -> nb::Result<(), Self::Error> {
        check_nb_timeout(UartTxDriver::wait_done(self, delay::NON_BLOCK))
    }

    fn write(&mut self, byte: u8) -> nb::Result<(), Self::Error> {
        check_nb(UartTxDriver::write_nb(self, &[byte]), ())
    }
}

impl core::fmt::Write for UartTxDriver<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let buf = s.as_bytes();
        let mut offset = 0;

        while offset < buf.len() {
            offset += self.write(buf).map_err(|_| core::fmt::Error)?
        }

        Ok(())
    }
}

pub struct AsyncUartDriver<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    driver: T,
    task: TaskHandle_t,
    _data: PhantomData<&'d ()>,
}

impl<'d> AsyncUartDriver<'d, UartDriver<'d>> {
    pub fn new<UART: Uart + 'd>(
        uart: UART,
        tx: impl OutputPin + 'd,
        rx: impl InputPin + 'd,
        cts: Option<impl InputPin + 'd>,
        rts: Option<impl OutputPin + 'd>,
        config: &config::Config,
    ) -> Result<Self, EspError> {
        Self::wrap(UartDriver::new(uart, tx, rx, cts, rts, config)?)
    }
}

impl<'d, T> AsyncUartDriver<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    pub fn wrap(driver: T) -> Result<Self, EspError> {
        Self::wrap_custom(driver, None, None)
    }

    pub fn wrap_custom(
        driver: T,
        priority: Option<u8>,
        pin_to_core: Option<Core>,
    ) -> Result<Self, EspError> {
        let task = new_task_common(
            driver.borrow().port,
            driver.borrow().event_queue(),
            priority,
            pin_to_core,
        )?;

        Ok(Self {
            driver,
            task,
            _data: PhantomData,
        })
    }

    pub fn driver(&self) -> &UartDriver<'d> {
        self.driver.borrow()
    }

    pub fn driver_mut(&mut self) -> &mut UartDriver<'d> {
        self.driver.borrow_mut()
    }

    /// Split the serial driver in separate TX and RX drivers
    pub fn split(
        &mut self,
    ) -> (
        AsyncUartTxDriver<'_, UartTxDriver<'_>>,
        AsyncUartRxDriver<'_, UartRxDriver<'_>>,
    ) {
        let (tx, rx) = self.driver_mut().split();

        (
            AsyncUartTxDriver {
                driver: tx,
                task: None,
                _data: PhantomData,
            },
            AsyncUartRxDriver {
                driver: rx,
                task: None,
                _data: PhantomData,
            },
        )
    }

    pub async fn read(&self, buf: &mut [u8]) -> Result<usize, EspError> {
        if buf.is_empty() {
            Ok(0)
        } else {
            loop {
                let res = self.driver.borrow().read(buf, delay::NON_BLOCK);

                match res {
                    Ok(len) if len > 0 => return Ok(len),
                    Err(e) if e.code() != ESP_ERR_TIMEOUT => return Err(e),
                    _ => (),
                }

                let port = self.driver.borrow().port as usize;
                READ_NOTIFS[port].wait().await;
            }
        }
    }

    pub async fn write(&self, bytes: &[u8]) -> Result<usize, EspError> {
        if bytes.is_empty() {
            Ok(0)
        } else {
            loop {
                let res = self.driver.borrow().write_nb(bytes);

                match res {
                    Ok(len) if len > 0 => return Ok(len),
                    Err(e) => return Err(e),
                    _ => (),
                }

                // We cannot properly wait for the TX FIFO queue to become non-full
                // because the ESP IDF UART ISR does not notify us on that
                //
                // Instead, spin a busy loop, however still allowing other futures to be polled too.
                crate::task::yield_now().await;
            }
        }
    }

    pub async fn wait_tx_done(&self) -> Result<(), EspError> {
        loop {
            let res = self.driver.borrow().wait_tx_done(delay::NON_BLOCK);

            match res {
                Ok(()) => return Ok(()),
                Err(e) if e.code() != ESP_ERR_TIMEOUT => return Err(e),
                _ => (),
            }

            // We cannot properly wait for the TX FIFO queue to become empty
            // because the ESP IDF UART ISR does not notify us on that
            //
            // Instead, spin a busy loop, however still allowing other futures to be polled too.
            crate::task::yield_now().await;
        }
    }
}

unsafe impl<'d, T> Send for AsyncUartDriver<'d, T> where T: BorrowMut<UartDriver<'d>> + Send {}
unsafe impl<'d, T> Sync for AsyncUartDriver<'d, T> where T: BorrowMut<UartDriver<'d>> + Send + Sync {}

impl<'d, T> Drop for AsyncUartDriver<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    fn drop(&mut self) {
        drop_task_common(self.task, self.driver.borrow().port);
    }
}

impl<'d, T> embedded_io::ErrorType for AsyncUartDriver<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    type Error = EspIOError;
}

impl<'d, T> embedded_io_async::Read for AsyncUartDriver<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        AsyncUartDriver::read(self, buf).await.map_err(EspIOError)
    }
}

impl<'d, T> embedded_io_async::Write for AsyncUartDriver<'d, T>
where
    T: BorrowMut<UartDriver<'d>>,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        AsyncUartDriver::write(self, buf).await.map_err(EspIOError)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        AsyncUartDriver::wait_tx_done(self)
            .await
            .map_err(EspIOError)
    }
}

pub struct AsyncUartRxDriver<'d, T>
where
    T: BorrowMut<UartRxDriver<'d>>,
{
    driver: T,
    task: Option<TaskHandle_t>,
    _data: PhantomData<&'d ()>,
}

impl<'d> AsyncUartRxDriver<'d, UartRxDriver<'d>> {
    pub fn new<UART: Uart + 'd>(
        uart: UART,
        rx: impl InputPin + 'd,
        cts: Option<impl InputPin + 'd>,
        rts: Option<impl OutputPin + 'd>,
        config: &config::Config,
    ) -> Result<Self, EspError> {
        Self::wrap(UartRxDriver::new(uart, rx, cts, rts, config)?)
    }
}

impl<'d, T> AsyncUartRxDriver<'d, T>
where
    T: BorrowMut<UartRxDriver<'d>>,
{
    pub fn wrap(driver: T) -> Result<Self, EspError> {
        Self::wrap_custom(driver, None, None)
    }

    pub fn wrap_custom(
        driver: T,
        priority: Option<u8>,
        pin_to_core: Option<Core>,
    ) -> Result<Self, EspError> {
        let task = new_task_common(
            driver.borrow().port,
            driver.borrow().event_queue(),
            priority,
            pin_to_core,
        )?;

        Ok(Self {
            driver,
            task: Some(task),
            _data: PhantomData,
        })
    }

    pub fn driver(&self) -> &UartRxDriver<'d> {
        self.driver.borrow()
    }

    pub fn driver_mut(&mut self) -> &mut UartRxDriver<'d> {
        self.driver.borrow_mut()
    }

    pub async fn read(&self, buf: &mut [u8]) -> Result<usize, EspError> {
        if buf.is_empty() {
            Ok(0)
        } else {
            loop {
                let res = self.driver.borrow().read(buf, delay::NON_BLOCK);

                match res {
                    Ok(len) if len > 0 => return Ok(len),
                    Err(e) if e.code() != ESP_ERR_TIMEOUT => return Err(e),
                    _ => (),
                }

                let port = self.driver.borrow().port as usize;
                READ_NOTIFS[port].wait().await;
            }
        }
    }
}

impl<'d, T> Drop for AsyncUartRxDriver<'d, T>
where
    T: BorrowMut<UartRxDriver<'d>>,
{
    fn drop(&mut self) {
        if let Some(task) = self.task {
            drop_task_common(task, self.driver.borrow().port);
        }
    }
}

impl<'d, T> embedded_io::ErrorType for AsyncUartRxDriver<'d, T>
where
    T: BorrowMut<UartRxDriver<'d>>,
{
    type Error = EspIOError;
}

impl<'d, T> embedded_io_async::Read for AsyncUartRxDriver<'d, T>
where
    T: BorrowMut<UartRxDriver<'d>>,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        AsyncUartRxDriver::read(self, buf).await.map_err(EspIOError)
    }
}

pub struct AsyncUartTxDriver<'d, T>
where
    T: BorrowMut<UartTxDriver<'d>>,
{
    driver: T,
    task: Option<TaskHandle_t>,
    _data: PhantomData<&'d ()>,
}

impl<'d> AsyncUartTxDriver<'d, UartTxDriver<'d>> {
    pub fn new<UART: Uart + 'd>(
        uart: UART,
        tx: impl OutputPin + 'd,
        cts: Option<impl InputPin + 'd>,
        rts: Option<impl OutputPin + 'd>,
        config: &config::Config,
    ) -> Result<Self, EspError> {
        Self::wrap(UartTxDriver::new(uart, tx, cts, rts, config)?)
    }
}

impl<'d, T> AsyncUartTxDriver<'d, T>
where
    T: BorrowMut<UartTxDriver<'d>>,
{
    pub fn wrap(driver: T) -> Result<Self, EspError> {
        Self::wrap_custom(driver, None, None)
    }

    pub fn wrap_custom(
        driver: T,
        priority: Option<u8>,
        pin_to_core: Option<Core>,
    ) -> Result<Self, EspError> {
        let task = new_task_common(
            driver.borrow().port,
            driver.borrow().event_queue(),
            priority,
            pin_to_core,
        )?;

        Ok(Self {
            driver,
            task: Some(task),
            _data: PhantomData,
        })
    }

    pub fn driver(&self) -> &UartTxDriver<'d> {
        self.driver.borrow()
    }

    pub fn driver_mut(&mut self) -> &mut UartTxDriver<'d> {
        self.driver.borrow_mut()
    }

    pub async fn write(&self, bytes: &[u8]) -> Result<usize, EspError> {
        if bytes.is_empty() {
            Ok(0)
        } else {
            loop {
                let res = self.driver.borrow().write_nb(bytes);

                match res {
                    Ok(len) if len > 0 => return Ok(len),
                    Err(e) => return Err(e),
                    _ => (),
                }

                // We cannot properly wait for the TX FIFO queue to become non-full
                // because the ESP IDF UART ISR does not notify us on that
                //
                // Instead, spin a busy loop, however still allowing other futures to be polled too.
                crate::task::yield_now().await;
            }
        }
    }

    pub async fn wait_done(&self) -> Result<(), EspError> {
        loop {
            let res = self.driver.borrow().wait_done(delay::NON_BLOCK);

            match res {
                Ok(()) => return Ok(()),
                Err(e) if e.code() != ESP_ERR_TIMEOUT => return Err(e),
                _ => (),
            }

            // We cannot properly wait for the TX FIFO queue to become empty
            // because the ESP IDF UART ISR does not notify us on that
            //
            // Instead, spin a busy loop, however still allowing other futures to be polled too.
            crate::task::yield_now().await;
        }
    }
}

impl<'d, T> Drop for AsyncUartTxDriver<'d, T>
where
    T: BorrowMut<UartTxDriver<'d>>,
{
    fn drop(&mut self) {
        if let Some(task) = self.task {
            drop_task_common(task, self.driver.borrow().port);
        }
    }
}

impl<'d, T> embedded_io::ErrorType for AsyncUartTxDriver<'d, T>
where
    T: BorrowMut<UartTxDriver<'d>>,
{
    type Error = EspIOError;
}

impl<'d, T> embedded_io_async::Write for AsyncUartTxDriver<'d, T>
where
    T: BorrowMut<UartTxDriver<'d>>,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        AsyncUartTxDriver::write(self, buf)
            .await
            .map_err(EspIOError)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        AsyncUartTxDriver::wait_done(self).await.map_err(EspIOError)
    }
}

fn new_task_common(
    port: u8,
    queue: Option<&Queue<UartEvent>>,
    priority: Option<u8>,
    pin_to_core: Option<Core>,
) -> Result<TaskHandle_t, EspError> {
    if let Some(queue) = queue {
        let port = port as usize;

        unsafe {
            QUEUES[port] = queue.as_raw() as _;
        }

        let res = unsafe {
            task::create(
                process_events,
                CStr::from_bytes_until_nul(b"UART - Events task\0").unwrap(),
                2048,
                port as _,
                priority.unwrap_or(6),
                pin_to_core,
            )
        };

        if res.is_err() {
            unsafe {
                QUEUES[port] = core::ptr::null();
            }
        }

        res
    } else {
        Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>())
    }
}

fn drop_task_common(task: TaskHandle_t, port: u8) {
    unsafe {
        task::destroy(task);
        QUEUES[port as usize] = core::ptr::null_mut();

        READ_NOTIFS[port as usize].reset();
        WRITE_NOTIFS[port as usize].reset();
        TX_NOTIFS[port as usize].reset();
    }
}

extern "C" fn process_events(arg: *mut core::ffi::c_void) {
    let port: usize = arg as _;
    let queue: Queue<UartEvent> = unsafe { Queue::new_borrowed(QUEUES[port] as _) };

    loop {
        if let Some((event, _)) = queue.recv_front(delay::BLOCK) {
            match event.payload() {
                UartEventPayload::Data { .. }
                | UartEventPayload::RxBufferFull
                | UartEventPayload::RxFifoOverflow => {
                    READ_NOTIFS[port].notify_lsb();
                }
                UartEventPayload::Break | UartEventPayload::DataBreak => {
                    WRITE_NOTIFS[port].notify_lsb();
                    TX_NOTIFS[port].notify_lsb();
                }
                _ => (),
            }
        }
    }
}

fn new_common<'d, UART: Uart + 'd>(
    _uart: UART,
    tx: Option<impl OutputPin + 'd>,
    rx: Option<impl InputPin + 'd>,
    cts: Option<impl InputPin + 'd>,
    rts: Option<impl OutputPin + 'd>,
    config: &config::Config,
    queue: Option<&mut QueueHandle_t>,
) -> Result<(), EspError> {
    let uart_config = config.into();

    esp!(unsafe { uart_param_config(UART::port(), &uart_config) })?;

    #[cfg(esp_idf_version_at_least_6_0_0)]
    {
        esp!(unsafe {
            _uart_set_pin6(
                UART::port(),
                tx.as_ref().map_or(-1, |p| p.pin() as _),
                rx.as_ref().map_or(-1, |p| p.pin() as _),
                rts.as_ref().map_or(-1, |p| p.pin() as _),
                cts.as_ref().map_or(-1, |p| p.pin() as _),
                -1,
                -1,
            )
        })?;
    }

    #[cfg(not(esp_idf_version_at_least_6_0_0))]
    {
        esp!(unsafe {
            uart_set_pin(
                UART::port(),
                tx.as_ref().map_or(-1, |p| p.pin() as _),
                rx.as_ref().map_or(-1, |p| p.pin() as _),
                rts.as_ref().map_or(-1, |p| p.pin() as _),
                cts.as_ref().map_or(-1, |p| p.pin() as _),
            )
        })?;
    }

    esp!(unsafe {
        #[allow(clippy::unwrap_or_default)]
        uart_driver_install(
            UART::port(),
            if rx.is_some() {
                config.rx_fifo_size as _
            } else {
                0
            },
            if tx.is_some() {
                config.tx_fifo_size as _
            } else {
                0
            },
            config.queue_size as _,
            queue.map(|q| q as *mut _).unwrap_or(ptr::null_mut()),
            InterruptType::to_native(config.intr_flags) as i32,
        )
    })?;

    esp!(unsafe { uart_set_mode(UART::port(), config.mode.into()) })?;

    // Configure interrupts after installing the driver
    // so it won't get overwritten.
    let usr_intrs = config.event_config.clone().into();
    esp!(unsafe { uart_intr_config(UART::port(), &usr_intrs as *const _) })?;

    Ok(())
}

fn stop_bits(port: uart_port_t) -> Result<config::StopBits, EspError> {
    let mut stop_bits: uart_stop_bits_t = 0;
    esp_result!(
        unsafe { uart_get_stop_bits(port, &mut stop_bits) },
        stop_bits.into()
    )
}

fn change_stop_bits(port: uart_port_t, stop_bits: config::StopBits) -> Result<(), EspError> {
    esp!(unsafe { uart_set_stop_bits(port, stop_bits.into()) })
}

fn data_bits(port: uart_port_t) -> Result<config::DataBits, EspError> {
    let mut data_bits: uart_word_length_t = 0;
    esp_result!(
        unsafe { uart_get_word_length(port, &mut data_bits) },
        data_bits.into()
    )
}

fn change_data_bits(port: uart_port_t, data_bits: config::DataBits) -> Result<(), EspError> {
    esp!(unsafe { uart_set_word_length(port, data_bits.into()) })
}

fn parity(port: uart_port_t) -> Result<config::Parity, EspError> {
    let mut parity: uart_parity_t = 0;
    esp_result!(unsafe { uart_get_parity(port, &mut parity) }, parity.into())
}

fn change_parity(port: uart_port_t, parity: config::Parity) -> Result<(), EspError> {
    esp!(unsafe { uart_set_parity(port, parity.into()) })
}

fn baudrate(port: uart_port_t) -> Result<Hertz, EspError> {
    let mut baudrate: u32 = 0;
    esp_result!(
        unsafe { uart_get_baudrate(port, &mut baudrate) },
        baudrate.into()
    )
}

fn change_baudrate<T: Into<Hertz> + Copy>(port: uart_port_t, baudrate: T) -> Result<(), EspError> {
    esp!(unsafe { uart_set_baudrate(port, baudrate.into().into()) })
}

fn delete_driver(port: uart_port_t) -> Result<(), EspError> {
    esp!(unsafe { uart_driver_delete(port) })
}

pub fn remaining_unread_bytes(port: uart_port_t) -> Result<usize, EspError> {
    let mut size = 0;
    esp_result!(unsafe { uart_get_buffered_data_len(port, &mut size) }, size)
}

#[cfg(any(
    not(esp_idf_version_major = "4"),
    all(
        esp_idf_version_minor = "4",
        not(any(esp_idf_version_patch = "0", esp_idf_version_patch = "1")),
    ),
))]
pub fn remaining_write_capacity(port: uart_port_t) -> Result<usize, EspError> {
    let mut size = 0;
    esp_result!(
        unsafe { uart_get_tx_buffer_free_size(port, &mut size) },
        size
    )
}

enum Owner {
    Owned,
    Borrowed,
    Shared,
}

impl Owner {
    fn drop_impl(&self, port: uart_port_t) -> Result<(), EspError> {
        let needs_drop = match self {
            Owner::Owned => true,
            Owner::Borrowed => false,
            Owner::Shared => REFS[port as usize].fetch_sub(1, Ordering::SeqCst) == 0,
        };

        if needs_drop {
            delete_driver(port)
        } else {
            Ok(())
        }
    }
}

macro_rules! impl_uart {
    ($uart:ident: $port:expr) => {
        crate::impl_peripheral!($uart);

        impl Uart for $uart<'_> {
            fn port() -> uart_port_t {
                $port
            }
        }
    };
}

fn check_nb<T>(result: Result<usize, EspError>, value: T) -> nb::Result<T, SerialError> {
    match result {
        Ok(len) => {
            if len > 0 {
                Ok(value)
            } else {
                Err(nb::Error::WouldBlock)
            }
        }
        Err(err) if err.code() == ESP_ERR_TIMEOUT => Err(nb::Error::WouldBlock),
        Err(err) => Err(nb::Error::Other(SerialError::new(ErrorKind::Other, err))),
    }
}

fn check_nb_timeout(result: Result<(), EspError>) -> nb::Result<(), SerialError> {
    match result {
        Ok(()) => Ok(()),
        Err(err) if err.code() == ESP_ERR_TIMEOUT => Err(nb::Error::WouldBlock),
        Err(err) => Err(nb::Error::Other(SerialError::new(ErrorKind::Other, err))),
    }
}

impl_uart!(UART0: 0);
impl_uart!(UART1: 1);
#[cfg(any(esp32, esp32s3, esp32p4, esp32s31))]
impl_uart!(UART2: 2);
#[cfg(any(esp32p4, esp32s31))]
impl_uart!(UART3: 3);
#[cfg(esp32p4)]
impl_uart!(UART4: 4);

#[allow(clippy::declare_interior_mutable_const)]
const NO_REFS: AtomicU8 = AtomicU8::new(0);
static REFS: [AtomicU8; SOC_UART_NUM as usize] = [NO_REFS; SOC_UART_NUM as usize];

#[allow(clippy::declare_interior_mutable_const)]
const NOTIF: Notification = Notification::new();
static READ_NOTIFS: [Notification; SOC_UART_NUM as usize] = [NOTIF; SOC_UART_NUM as usize];
static WRITE_NOTIFS: [Notification; SOC_UART_NUM as usize] = [NOTIF; SOC_UART_NUM as usize];
static TX_NOTIFS: [Notification; SOC_UART_NUM as usize] = [NOTIF; SOC_UART_NUM as usize];
static mut QUEUES: [*const core::ffi::c_void; SOC_UART_NUM as usize] =
    [core::ptr::null(); SOC_UART_NUM as usize];
