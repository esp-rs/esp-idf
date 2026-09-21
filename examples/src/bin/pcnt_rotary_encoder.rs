//! PCNT decoding a rotary encoder
//!
//! To try this out, connect a rotary encoder to pins 5 and 6, the common should be grounded

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(not(feature = "pcnt-legacy"), esp_idf_soc_pcnt_supported))]
mod example {
    use std::time::Duration;

    use anyhow::Context;
    use esp_idf_hal::delay::FreeRtos;
    use esp_idf_hal::gpio::{AnyInputPin, InputPin};
    use esp_idf_hal::pcnt::config::{
        ChannelEdgeAction, ChannelLevelAction, GlitchFilterConfig, UnitConfig,
    };
    use esp_idf_hal::pcnt::PcntUnitDriver;
    use esp_idf_hal::peripherals::Peripherals;
    use esp_idf_sys::EspError;

    const LOW_LIMIT: i32 = i16::MIN as i32;
    const HIGH_LIMIT: i32 = -LOW_LIMIT;

    pub struct Encoder<'d> {
        unit: PcntUnitDriver<'d>,
    }

    impl<'d> Encoder<'d> {
        pub fn new(pin_a: impl InputPin + 'd, pin_b: impl InputPin + 'd) -> Result<Self, EspError> {
            let mut unit = PcntUnitDriver::new(&UnitConfig {
                low_limit: LOW_LIMIT,
                high_limit: HIGH_LIMIT,
                accum_count: true,
                ..Default::default()
            })?;

            unit.set_glitch_filter(Some(&GlitchFilterConfig {
                max_glitch: Duration::from_nanos(1000),
                ..Default::default()
            }))?;

            let (duplicate_pin_a, duplicate_pin_b) = unsafe {
                (
                    AnyInputPin::steal(pin_a.pin()),
                    AnyInputPin::steal(pin_b.pin()),
                )
            };

            unit.add_channel(Some(pin_a), Some(pin_b), &Default::default())?
                .set_edge_action(ChannelEdgeAction::Decrease, ChannelEdgeAction::Increase)?
                .set_level_action(ChannelLevelAction::Keep, ChannelLevelAction::Inverse)?;

            unit.add_channel(
                Some(duplicate_pin_b),
                Some(duplicate_pin_a),
                &Default::default(),
            )?
            .set_edge_action(ChannelEdgeAction::Increase, ChannelEdgeAction::Decrease)?
            .set_level_action(ChannelLevelAction::Keep, ChannelLevelAction::Inverse)?;

            unit.enable()?;

            // If accum_count is true, the LOW and HIGH limits have to be added to the watch points
            unit.add_watch_points_and_clear([LOW_LIMIT, HIGH_LIMIT])?;

            // Start counting:
            unit.start()?;

            Ok(Self { unit })
        }

        pub fn get_value(&self) -> Result<i32, EspError> {
            self.unit.get_count()
        }
    }

    pub fn run() -> anyhow::Result<()> {
        // Temporary. Will disappear once ESP-IDF 4.4 is released, but for now it is necessary to call this function once,
        // or else some patches to the runtime implemented by esp-idf-sys might not link properly.
        esp_idf_hal::sys::link_patches();

        println!("setup pins");
        let peripherals = Peripherals::take().context("failed to take Peripherals")?;
        let pin_a = peripherals.pins.gpio4;
        let pin_b = peripherals.pins.gpio5;
        println!("setup encoder");
        let encoder = Encoder::new(pin_a, pin_b)?;

        let mut last_value = 0i32;
        loop {
            let value = encoder.get_value()?;
            if value != last_value {
                println!("value: {value}");
                last_value = value;
            }
            FreeRtos::delay_ms(100u32);
        }
    }
}

#[cfg(all(not(feature = "pcnt-legacy"), esp_idf_soc_pcnt_supported))]
fn main() -> anyhow::Result<()> {
    example::run()
}

#[cfg(all(
    not(esp_idf_version_at_least_6_0_0),
    any(esp32, esp32s2, esp32s3),
    feature = "pcnt-legacy"
))]
fn main() -> anyhow::Result<()> {
    example::run()
}

#[cfg(any(
    all(
        not(esp_idf_version_at_least_6_0_0),
        not(any(esp32, esp32s2, esp32s3)),
        feature = "pcnt-legacy"
    ),
    not(esp_idf_soc_pcnt_supported)
))]
fn main() {
    use esp_idf_hal::delay::FreeRtos;

    #[cfg(not(any(esp32, esp32s2, esp32s3)))]
    println!("PCNT is not supported on this device");

    #[cfg(esp_idf_version_at_least_6_0_0)]
    println!("PCNT is not yet available when building against ESP-IDF 6.0+");

    loop {
        FreeRtos::delay_ms(100u32);
    }
}

#[cfg(all(
    not(esp_idf_version_at_least_6_0_0),
    any(esp32, esp32s2, esp32s3),
    feature = "pcnt-legacy"
))]
/// esp-idf encoder implementation using v4 pcnt api
///
/// Note that PCNT only track a signed 16bit value.  We use interrupts to detect a LOW and HIGH
/// threshold and track how much that accounts for and provide an i32 value result
mod example {
    use std::cmp::min;
    use std::sync::atomic::AtomicI32;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    use anyhow::Context;
    use esp_idf_hal::delay::FreeRtos;
    use esp_idf_hal::gpio::AnyInputPin;
    use esp_idf_hal::gpio::InputPin;
    use esp_idf_hal::pcnt::*;
    use esp_idf_hal::peripherals::Peripherals;
    use esp_idf_sys::EspError;

    const LOW_LIMIT: i16 = -100;
    const HIGH_LIMIT: i16 = 100;

    pub struct Encoder<'d> {
        unit: PcntDriver<'d>,
        approx_value: Arc<AtomicI32>,
    }

    impl<'d> Encoder<'d> {
        pub fn new(
            pcnt: impl Pcnt + 'd,
            pin_a: impl InputPin + 'd,
            pin_b: impl InputPin + 'd,
        ) -> Result<Self, EspError> {
            let mut unit = PcntDriver::new(
                pcnt,
                Some(pin_a),
                Some(pin_b),
                Option::<AnyInputPin>::None,
                Option::<AnyInputPin>::None,
            )?;
            unit.channel_config(
                PcntChannel::Channel0,
                PinIndex::Pin0,
                PinIndex::Pin1,
                &PcntChannelConfig {
                    lctrl_mode: PcntControlMode::Reverse,
                    hctrl_mode: PcntControlMode::Keep,
                    pos_mode: PcntCountMode::Decrement,
                    neg_mode: PcntCountMode::Increment,
                    counter_h_lim: HIGH_LIMIT,
                    counter_l_lim: LOW_LIMIT,
                },
            )?;
            unit.channel_config(
                PcntChannel::Channel1,
                PinIndex::Pin1,
                PinIndex::Pin0,
                &PcntChannelConfig {
                    lctrl_mode: PcntControlMode::Reverse,
                    hctrl_mode: PcntControlMode::Keep,
                    pos_mode: PcntCountMode::Increment,
                    neg_mode: PcntCountMode::Decrement,
                    counter_h_lim: HIGH_LIMIT,
                    counter_l_lim: LOW_LIMIT,
                },
            )?;

            unit.set_filter_value(min(10 * 80, 1023))?;
            unit.filter_enable()?;

            let approx_value = Arc::new(AtomicI32::new(0));
            // unsafe interrupt code to catch the upper and lower limits from the encoder
            // and track the overflow in `value: Arc<AtomicI32>` - I plan to use this for
            // a wheeled robot's odomerty
            unsafe {
                let approx_value = approx_value.clone();
                unit.subscribe(move |status| {
                    let status = PcntEventType::from_repr_truncated(status);
                    if status.contains(PcntEvent::HighLimit) {
                        approx_value.fetch_add(HIGH_LIMIT as i32, Ordering::SeqCst);
                    }
                    if status.contains(PcntEvent::LowLimit) {
                        approx_value.fetch_add(LOW_LIMIT as i32, Ordering::SeqCst);
                    }
                })?;
            }
            unit.event_enable(PcntEvent::HighLimit)?;
            unit.event_enable(PcntEvent::LowLimit)?;
            unit.counter_pause()?;
            unit.counter_clear()?;
            unit.counter_resume()?;

            Ok(Self { unit, approx_value })
        }

        pub fn get_value(&self) -> Result<i32, EspError> {
            let value =
                self.approx_value.load(Ordering::Relaxed) + self.unit.get_counter_value()? as i32;
            Ok(value)
        }
    }

    pub fn run() -> anyhow::Result<()> {
        // Temporary. Will disappear once ESP-IDF 4.4 is released, but for now it is necessary to call this function once,
        // or else some patches to the runtime implemented by esp-idf-sys might not link properly.
        esp_idf_hal::sys::link_patches();

        println!("setup pins");
        let peripherals = Peripherals::take().context("failed to take Peripherals")?;
        let pin_a = peripherals.pins.gpio4;
        let pin_b = peripherals.pins.gpio5;
        println!("setup encoder");
        let encoder = Encoder::new(peripherals.pcnt0, pin_a, pin_b)?;

        let mut last_value = 0i32;
        loop {
            let value = encoder.get_value()?;
            if value != last_value {
                println!("value: {value}");
                last_value = value;
            }
            FreeRtos::delay_ms(100u32);
        }

        Ok(())
    }
}
