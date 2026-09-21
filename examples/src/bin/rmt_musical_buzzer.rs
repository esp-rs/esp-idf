//! Play a song using a piezo buzzer.
//!
//! Should play "Ode to Joy" on pin 17.
//!
//! Based off the ESP-IDF rmt musical buzzer example:
//! https://github.com/espressif/esp-idf/blob/b092fa073047c957545a0ae9504f04972a8c6d74/examples/peripherals/rmt/musical_buzzer/main/musical_buzzer_example_main.c

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(any(feature = "rmt-legacy", esp_idf_version_major = "4"))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(not(any(feature = "rmt-legacy", esp_idf_version_major = "4")))]
fn main() -> anyhow::Result<()> {
    println!("This example requires feature `rmt-legacy` enabled or using ESP-IDF v4.4.X");

    loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}

#[cfg(any(feature = "rmt-legacy", esp_idf_version_major = "4"))]
mod example {
    use std::time::Duration;

    use esp_idf_hal::delay::Ets;
    use esp_idf_hal::{
        peripherals::Peripherals,
        rmt::{self, config::TransmitConfig, TxRmtDriver},
        units::Hertz,
    };
    use notes::*;

    pub fn main() -> anyhow::Result<()> {
        esp_idf_hal::sys::link_patches();

        let peripherals = Peripherals::take()?;
        let led = peripherals.pins.gpio17;
        let channel = peripherals.rmt.channel0;
        let config = TransmitConfig::new();
        let mut tx: TxRmtDriver<'static> = TxRmtDriver::new(channel, led, &config)?;

        loop {
            play_song(&mut tx, ODE_TO_JOY)?;
            Ets::delay_ms(3000);
        }
    }

    pub fn play_song(tx: &mut TxRmtDriver<'static>, song: &[NoteValue]) -> anyhow::Result<()> {
        for note_value in song {
            note_value.play(tx)?;
        }
        Ok(())
    }

    pub struct NoteValueIter {
        ticks: rmt::PulseTicks,
        tone_cycles: u32,
        pause_cycles: u32,
    }

    impl NoteValueIter {
        fn new(ticks_per_sec: Hertz, note: &NoteValue) -> Self {
            // Calculate the frequency for a piezo buzzer.
            let dur_ms = note.duration.as_millis();
            let cycles_per_sec = note.note.0; // pitch
            let ticks_per_cycle = ticks_per_sec.0 as u128 / cycles_per_sec as u128;
            let ticks_per_half = (ticks_per_cycle / 2_u128) as u16;
            let ticks = rmt::PulseTicks::new(ticks_per_half).unwrap();

            let total_cycles = (cycles_per_sec as u128 * dur_ms / 1000_u128) as u32;
            // Pause for the last 40ms of every note
            let pause_cycles = (cycles_per_sec as u128 * 40_u128 / 1000_u128) as u32;
            let tone_cycles = total_cycles - pause_cycles;

            Self {
                ticks,
                tone_cycles,
                pause_cycles,
            }
        }
    }

    impl std::iter::Iterator for NoteValueIter {
        type Item = rmt::Symbol;

        // runs in ISR
        fn next(&mut self) -> Option<Self::Item> {
            if self.tone_cycles + self.pause_cycles > 0 {
                let high_state = if self.tone_cycles > 0 {
                    self.tone_cycles -= 1;
                    rmt::PinState::High
                } else {
                    self.pause_cycles -= 1;
                    rmt::PinState::Low
                };
                let level0 = rmt::Pulse::new(high_state, self.ticks);
                let level1 = rmt::Pulse::new(rmt::PinState::Low, self.ticks);
                Some(rmt::Symbol::new(level0, level1))
            } else {
                None
            }
        }
    }

    #[derive(Debug)]
    pub struct Note(u16);

    #[allow(dead_code)]
    pub mod notes {
        use super::Note;

        pub const A4: Note = Note(440);
        pub const AS4: Note = Note(466);
        pub const B4: Note = Note(494);
        pub const C5: Note = Note(523);
        pub const CS5: Note = Note(554);
        pub const D5: Note = Note(587);
        pub const DS5: Note = Note(622);
        pub const E5: Note = Note(659);
        pub const F5: Note = Note(698);
        pub const FS5: Note = Note(740);
        pub const G5: Note = Note(784);
        pub const GS5: Note = Note(831);
        pub const A5: Note = Note(880);
    }

    #[derive(Debug)]
    pub struct NoteValue {
        note: Note,
        duration: Duration,
    }

    impl NoteValue {
        pub fn play(&self, tx: &mut TxRmtDriver<'static>) -> anyhow::Result<()> {
            let ticks_hz = tx.counter_clock()?;
            tx.start_iter_blocking(self.iter(ticks_hz))?;
            Ok(())
        }

        pub fn iter(&self, ticks_hz: Hertz) -> NoteValueIter {
            NoteValueIter::new(ticks_hz, self)
        }
    }

    macro_rules! n {
        ($n: expr, $duration: expr) => {
            NoteValue {
                note: $n,
                duration: Duration::from_millis($duration),
            }
        };
    }

    const ODE_TO_JOY: &[NoteValue] = &[
        n!(FS5, 400),
        n!(FS5, 600),
        n!(G5, 400),
        n!(A5, 400),
        n!(A5, 400),
        n!(G5, 400),
        n!(FS5, 400),
        n!(E5, 400),
        n!(D5, 400),
        n!(D5, 400),
        n!(E5, 400),
        n!(FS5, 400),
        n!(FS5, 400),
        n!(FS5, 200),
        n!(E5, 200),
        n!(E5, 800),
        n!(FS5, 400),
        n!(FS5, 600),
        n!(G5, 400),
        n!(A5, 400),
        n!(A5, 400),
        n!(G5, 400),
        n!(FS5, 400),
        n!(E5, 400),
        n!(D5, 400),
        n!(D5, 400),
        n!(E5, 400),
        n!(FS5, 400),
        n!(E5, 400),
        n!(E5, 200),
        n!(D5, 200),
        n!(D5, 800),
        n!(E5, 400),
        n!(E5, 400),
        n!(FS5, 400),
        n!(D5, 400),
        n!(E5, 400),
        n!(FS5, 200),
        n!(G5, 200),
        n!(FS5, 400),
        n!(D5, 400),
        n!(E5, 400),
        n!(FS5, 200),
        n!(G5, 200),
        n!(FS5, 400),
        n!(E5, 400),
        n!(D5, 400),
        n!(E5, 400),
        n!(A4, 400),
        n!(A4, 400),
        n!(FS5, 400),
        n!(FS5, 600),
        n!(G5, 400),
        n!(A5, 400),
        n!(A5, 400),
        n!(G5, 400),
        n!(FS5, 400),
        n!(E5, 400),
        n!(D5, 400),
        n!(D5, 400),
        n!(E5, 400),
        n!(FS5, 400),
        n!(E5, 400),
        n!(E5, 200),
        n!(D5, 200),
        n!(D5, 800),
    ];
}

// #[cfg(any(feature = "rmt-legacy", esp_idf_version_major = "4"))]
// #[cfg(any(feature = "rmt-legacy", esp_idf_version_major = "4"))]
// #[cfg(any(feature = "rmt-legacy", esp_idf_version_major = "4"))]
// #[cfg(any(feature = "rmt-legacy", esp_idf_version_major = "4"))]
// #[allow(dead_code)]
// #[cfg(any(feature = "rmt-legacy", esp_idf_version_major = "4"))]
