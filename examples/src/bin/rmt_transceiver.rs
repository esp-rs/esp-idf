//! A simple example that sets up a RMT transmitter and an RMT receiver
//!
//! GPIO pin 2 is input and GPIO pin 4 is output
//!
//!    TYPICAL OUTPUT
//! Tx Loop
//! Rx Loop
//! level0 = High   dur0 = PulseTicks(620)   level1 = Low   dur1 = PulseTicks(620)
//! level0 = High   dur0 = PulseTicks(620)   level1 = Low   dur1 = PulseTicks(620)
//! level0 = High   dur0 = PulseTicks(210)   level1 = Low   dur1 = PulseTicks(410)
//! level0 = High   dur0 = PulseTicks(410)   level1 = Low   dur1 = PulseTicks(210)
//! level0 = High   dur0 = PulseTicks(210)   level1 = Low   dur1 = PulseTicks(0)
//! Rx Loop
//! level0 = High   dur0 = PulseTicks(620)   level1 = Low   dur1 = PulseTicks(620)
//! level0 = High   dur0 = PulseTicks(620)   level1 = Low   dur1 = PulseTicks(620)
//! level0 = High   dur0 = PulseTicks(210)   level1 = Low   dur1 = PulseTicks(410)
//! level0 = High   dur0 = PulseTicks(410)   level1 = Low   dur1 = PulseTicks(210)
//! level0 = High   dur0 = PulseTicks(210)   level1 = Low   dur1 = PulseTicks(0)
//! Tx Loop

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(
    esp_idf_soc_rmt_supported,
    esp_idf_version_at_least_5_0_0,
    not(feature = "rmt-legacy")
))]
mod example {
    // NOTE: The output looks slightly different for this example, but the values are the same.
    use esp_idf_hal::delay::FreeRtos;
    use esp_idf_hal::gpio::{
        AnyIOPin, AnyInputPin, AnyOutputPin, InputPin, OutputPin, PinDriver, Pull,
    };
    use esp_idf_hal::rmt::config::{
        Loop, ReceiveConfig, RxChannelConfig, TransmitConfig, TxChannelConfig,
    };
    use esp_idf_hal::rmt::encoder::CopyEncoder;
    use esp_idf_hal::rmt::{PinState, Pulse, PulseTicks, Symbol};
    use esp_idf_hal::rmt::{RxChannelDriver, TxChannelDriver};
    use esp_idf_hal::units::Hertz;

    use core::mem;
    use core::time::Duration;

    const RMT_RESOLUTION: Hertz = Hertz(10_000_000); // 10MHz resolution, 1 tick = 0.1us
    const DELAY_DURATION: Duration = Duration::from_millis(1000);
    const SHARED_PIN: Option<u8> = Some(2); // Some(n) to use the same pin for input and output, None to use separate pins
    const INPUT_PIN: u8 = 2;
    const OUTPUT_PIN: u8 = 4;

    /// Splits a pin that can be both input and output into separate input and output pins.
    ///
    /// # Safety
    ///
    /// The caller must ensure that none of the returned pins are passed to code that reconfigures
    /// the pin mode. For example `PinDriver` does reconfigure the pin mode.
    unsafe fn split_pin<'d>(
        pin: impl InputPin + OutputPin + 'd,
    ) -> (AnyInputPin<'d>, AnyOutputPin<'d>) {
        let driver = PinDriver::input_output(pin, Pull::Down).unwrap();

        let input = AnyInputPin::steal(driver.pin());
        let output = AnyOutputPin::steal(driver.pin());

        // on drop the driver will reconfigure the pin, so we must forget it
        mem::forget(driver);

        (input, output)
    }

    pub fn run() -> anyhow::Result<()> {
        println!("Create RMT TX channel");

        // SAFETY: You do not have to use the pins like this, instead you can also use
        // `Peripherals::take()`, then get the pins from there.
        //
        // The below is just for demonstration purposes, allowing to have a constant through
        // which one can configure which pins to use.
        let (input_pin, output_pin) = {
            if let Some(pin_num) = SHARED_PIN {
                unsafe { split_pin(AnyIOPin::steal(pin_num)) }
            } else {
                (unsafe { AnyInputPin::steal(INPUT_PIN) }, unsafe {
                    AnyOutputPin::steal(OUTPUT_PIN)
                })
            }
        };

        let mut tx_channel = TxChannelDriver::new(
            output_pin,
            &TxChannelConfig {
                resolution: RMT_RESOLUTION,
                ..Default::default()
            },
        )?;

        let sync_symbol = Symbol::new(
            Pulse::new(PinState::High, PulseTicks::new(620)?),
            Pulse::new(PinState::Low, PulseTicks::new(620)?),
        );
        let zero_symbol = Symbol::new(
            Pulse::new(PinState::High, PulseTicks::new(410)?),
            Pulse::new(PinState::Low, PulseTicks::new(210)?),
        );
        let one_symbol = Symbol::new(
            Pulse::new(PinState::High, PulseTicks::new(210)?),
            Pulse::new(PinState::Low, PulseTicks::new(410)?),
        );

        // Instruct the RMT peripheral to send the given signal endlessly:
        let mut signal = vec![
            sync_symbol,
            sync_symbol,
            one_symbol,
            zero_symbol,
            one_symbol,
        ];

        // Add a delay at the end of each signal:
        signal.extend(
            Symbol::new(
                Pulse::new(PinState::Low, PulseTicks::max()),
                Pulse::new(PinState::Low, PulseTicks::max()),
            )
            .repeat_for(RMT_RESOLUTION, DELAY_DURATION),
        );

        let mut encoder = CopyEncoder::new()?;

        // SAFETY: The encoder and signal are valid until the end of this function, which would drop the channel as well.
        unsafe {
            tx_channel.start_send(
                &mut encoder,
                &signal,
                &TransmitConfig {
                    loop_count: Loop::Endless,
                    ..Default::default()
                },
            )
        }?;

        // Now configure the receiver to receive the data:
        let mut rx_channel = RxChannelDriver::new(
            input_pin,
            &RxChannelConfig {
                resolution: RMT_RESOLUTION,
                ..Default::default()
            },
        )?;

        loop {
            println!("Rx Loop");

            let mut buffer = [Symbol::default(); 250];
            let read = rx_channel.receive(
                &mut buffer,
                &ReceiveConfig {
                    // A pulse whose width is larger than the max will be treated as idle and end the reception.
                    //
                    // In the above declared signals, the largest pulse is 620 ticks long -> 700 should be good.
                    signal_range_max: PulseTicks::new(700)?.duration(RMT_RESOLUTION),
                    ..Default::default()
                },
            )?;

            for symbol in &buffer[..read] {
                println!("0={:?}, 1={:?}", symbol.level0(), symbol.level1());
            }

            FreeRtos::delay_ms(500);
        }
    }
}

#[cfg(all(
    esp_idf_soc_rmt_supported,
    esp_idf_version_at_least_5_0_0,
    not(feature = "rmt-legacy")
))]
fn main() -> anyhow::Result<()> {
    // Sometimes the default stack size is not enough -> it is increased here.
    std::thread::Builder::new()
        .stack_size(10_000)
        .spawn(example::run)?
        .join()
        .unwrap()
}

#[cfg(any(feature = "rmt-legacy", esp_idf_version_major = "4"))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(any(feature = "rmt-legacy", esp_idf_version_major = "4"))]
mod example {
    use esp_idf_hal::{
        delay::FreeRtos,
        peripherals::Peripherals,
        rmt::{
            FixedLengthSignal, PinState, Pulse, PulseTicks, Receive, RmtReceiveConfig,
            RmtTransmitConfig, RxRmtDriver, TxRmtDriver,
        },
    };

    pub fn main() -> anyhow::Result<()> {
        println!("Starting APP!");

        let peripherals = Peripherals::take()?;

        /*
         *********************** SET UP RMT RECEIVER ******************************
         */
        let mut rx = RxRmtDriver::new(
            peripherals.rmt.channel2,
            peripherals.pins.gpio2,
            &RmtReceiveConfig::new().idle_threshold(700u16),
            250,
        )?;

        rx.start()?;

        let _ = std::thread::Builder::new()
            .stack_size(10000)
            .spawn(move || loop {
                println!("Rx Loop");

                let mut pulses = [(Pulse::zero(), Pulse::zero()); 250];

                // See sdkconfig.defaults to determine the tick time value ( default is one tick = 10 milliseconds)
                // Set ticks_to_wait to 0 for non-blocking
                let receive = rx.receive(&mut pulses, 0).unwrap();

                if let Receive::Read(length) = receive {
                    let pulses = &pulses[..length];

                    for (pulse0, pulse1) in pulses {
                        println!("0={pulse0:?}, 1={pulse1:?}");
                    }
                }

                FreeRtos::delay_ms(500);
            });

        /*
         *********************** SET UP RMT TRANSMITTER ******************************
         */

        // Prepare the tx_config
        // The default uses one memory block or 64 signals and clock divider set to 80 (1us tick)
        let mut tx = TxRmtDriver::new(
            peripherals.rmt.channel0,
            peripherals.pins.gpio4,
            &RmtTransmitConfig::new(),
        )?;

        // Prepare signal pulse signal to be sent.
        let one_low = Pulse::new(PinState::Low, PulseTicks::new(410)?);
        let one_high = Pulse::new(PinState::High, PulseTicks::new(210)?);
        let zero_low = Pulse::new(PinState::Low, PulseTicks::new(210)?);
        let zero_high = Pulse::new(PinState::High, PulseTicks::new(410)?);
        let sync_low = Pulse::new(PinState::Low, PulseTicks::new(620)?);
        let sync_high = Pulse::new(PinState::High, PulseTicks::new(620)?);

        let _ = std::thread::spawn(move || loop {
            println!("Tx Loop");

            // Create a sequence
            let mut signal = FixedLengthSignal::<5>::new();
            signal.set(0, &(sync_high, sync_low)).unwrap();
            signal.set(1, &(sync_high, sync_low)).unwrap();
            signal.set(2, &(one_high, one_low)).unwrap();
            signal.set(3, &(zero_high, zero_low)).unwrap();
            signal.set(4, &(one_high, one_low)).unwrap();

            // Transmit the signal (send sequence)
            tx.start(signal).unwrap();

            FreeRtos::delay_ms(1000);
        });

        loop {
            FreeRtos::delay_ms(3000);
        }
    }
}
