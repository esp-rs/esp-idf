# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.47.0] - 2026-09-15

### Fixed
- Expose I2S2, UART2, UART3 and UART4 on the esp32p4
- Expose SPI3 on the esp32p4
- `PinDriver` now routes the pad to the GPIO function before setting its direction. A pin whose `IO_MUX.MCU_SEL` boots on an alternate function - JTAG, SPI flash, UART - stayed on that function, so `set_high`/`set_low` never reached the pad (#585)
- Fixed issue 502 - some ADC/RTC pins for esp32c5 and esp32c6 had mapping errors
- Fixed async SPI transactions occasionally hanging forever on multi-core chips
- SPI: `SpiBusDriver::new` no longer leaves the device attached to the bus when acquiring the bus lock fails. The device used to stay registered, so freeing the bus later failed with `ESP_ERR_INVALID_STATE` ("not all CSses freed") and panicked in `SpiDriver`'s destructor
- Fix a real, live-hardware-reproduced panic in `rmt::encoder::EncoderState`'s
  `From<rmt_encode_state_t>` conversion, surfacing as an unexplained ESP-IDF `abort()` rather than
  a normal panic message, since the panic occurs inside the RMT ISR where Rust's panic handling
  can't safely lock/print (newlib's own lock code detects the invalid ISR-context lock attempt and
  aborts instead). `rmt_encode_state_t` is a genuine C bitmask (`RMT_ENCODING_RESET = 0`,
  `RMT_ENCODING_COMPLETE = (1 << 0)`, `RMT_ENCODING_MEM_FULL = (1 << 1)`,
  `RMT_ENCODING_WITH_EOF = (1 << 2)`), but `EncoderState` modeled the individual flags as
  mutually-exclusive enum variants, which panicked on any combined value it hadn't special-cased.
  Confirmed on real hardware this isn't an edge case: a small WS2812-strip transmission that
  finishes encoding while also exactly filling the remaining RMT memory block produces
  `COMPLETE | MEM_FULL` on its last encode step - a different combination than the also-valid
  `COMPLETE | WITH_EOF`, and not necessarily the only one.

### Added
- `QueueSet2`, `QueueSet3`, `QueueSet4` for waiting on multiple heterogeneous queues
- Basic compatibility for ESP-IDF release 6.0.
- I2S Standard, PDM and TDM Mode Driver dynamic reconfiguration (change sample rate)
- Support ESP-IDF 6.1 DSI bus flags and continuous high-speed clock-lane configuration.

### Breaking
- I2S: `ClockSource::Apll` and `ClockSource::Xtal` on `esp32p4`, where the XTAL is now the default for this chip
- Dropped support for RGB/BGR666 format on ESP-IDF 6.0.

### Changed (breaking)
- `rmt::encoder::EncoderState` is now a bitmask wrapper type instead of an enum, matching the C
  type it mirrors and able to represent any real flag combination without panicking:
  `EncoderState::EncodingComplete`/`EncodingMemoryFull`/`EncodingReset`/`EncodingWithEof` are
  replaced by associated consts `EncoderState::COMPLETE`/`MEM_FULL`/`RESET`/`WITH_EOF`, combinable
  via `|` (`impl BitOr`), and by query methods `is_complete()`/`is_mem_full()`/`is_reset()`/
  `has_eof()` in place of matching on a variant.

## [0.46.2] - 2026-03-10

### Fixed
- Fix a bug where the GPIO module did not build for esp32c5
- Fix a bug where the LCD driver was not protected with the esp_lcd component

## [0.46.1] - 2026-03-10

### Fixed
- Fix several compilation issues with esp32p4 and esp32h2

## [0.46.0] - 2026-03-09

### Breaking
- MSRV raised to 1.79
- The Timer drivers are not available under ESP-IDF 6+.
  These need to be rewritten against their newer "gptimer" ESP-IDF C equivalents.
- (#529) `Peripheral` and `PeripheralRef` removed and replaced with a simple pattern similar to the `esp-hal` one.
  - Check https://github.com/esp-rs/esp-idf-hal/pull/529 for details on that change
- The `prelude` module is removed. It was anyway only having a handful of types listed in it. And furthermore, there was no `prelude` module for `esp-idf-svc`.
- `embassy-sync` updated to V0.7
- Implement new pcnt API. The old API will be available through the `pcnt-legacy` feature.
- `ThreadSpawnConfiguration::name` changed from `&'static [u8]` to `&'static core::ffi::CStr` as it is actually a `CStr`
- The `rmt` module is now only exposed on chips that support the rmt feature
- The `onewire` code has been updated for 1.0.4 and the new rmt API
- Implement new gptimer API. The old API will be available through the `timer-legacy` feature.
- Update `embedded-io`, `embedded-io-async`, and `heapless` dependencies to their latest versions
- Raise Rust MSRV to `1.82`

### Deprecated
- `DB_11` ADC attenuation in favor of `DB_12` for ESP-IDF V5.0+
- The old pcnt API (only available with the `pcnt-legacy` feature).
- The old rmt API (only available with the `rmt-legacy` feature)
- The old gptimer API (only available with the `timer-legacy` feature)

### Added
- `Send` for `AsyncCanDriver`
- `DB_12` ADC attenuation
- `fade_with_time`, `fade_with_step`, `fade_stop` for `LedcDriver`
- `write_with_break` for `UartDriver` and `UartTxDriver`
- New RMT API
- New GPTimer API
- esp32p4 pins and core command added.
- Support for esp32c5 and esp32c61
- Made the modem peripheral for esp32p4 dependent on esp_wifi_remote
- LDO support for esp32p4
- Support for `PLL_F48M` UART clock source on ESP32-H2
- LCD driver support; DSI peripheral support with the LCD driver for esp32p4
- New Deep and Light Sleep API (#555)

### Fixed
- Fix pcnt_rotary_encoder example for esp32
- Fix the SDMMC driver for ESP-IDF V5.5+
- Fix UART initialization panic on ESP32-H2 with ESP-IDF 5.x due to unsupported `PLL_F48M` clock source
- Replace Arc with Rc in ledc_threads example (#514)
- Fix outdated task docs
- CAN: fix wrong Alert enum indexing / remove wrong TryFromPrimitive derive (#532)
- GPIO: Allow interoperability with other code that initializes the GPIO ISR service (#537)
- SD card support is no longer behind the `experimental` feature
- Added 6th TX channel config argument for IDF 5.5.2+.
- esp32p4 uses vPortExitCriticalMultiCore instead of vPortExitCritical
- UART: Fix dangling driver state when initialization fails mid-way

## [0.45.2] - 2025-01-15

### Fixed
- Fix ledc example (#511)
- Fix missing sdspi_host_get_dma_info for sdcard driver bug (#509)

## [0.45.1] - 2025-01-10

### Fixed
- Fix RGB to u32 conversion order in rmt_neopixel example (#505)
- Fix missing sdmmc_host_get_dma_info for sdcard driver bug (#507)

## [0.45.0] - 2025-01-02

### Deprecated

### Breaking
- Make the UART config to ESP-IDF  C config reusable outside the UART driver (for OpenThread)
- Re-use the SPI config struct of the SPI driver (for OpenThread)
- Add a `usb_serial` peripheral to the `Peripherals` struct for MCUs that do support the USB-serial-jtag peripheral (#503)
- UART default baud rate changed from 19'200 to 115'200

### Added
- Compatibility with ESP-IDF v5.3.X
- Docs: Add some docstrings for ADC module. (#455)
- Update uart.rs, add rs485 half duplex (#456)
- Update spi.rs, fix half3wire read (#459)
- SPI: Mark DMA as supported in the docu
- SD Card driver; SD Card host drivers (SPI and SDMMC) (#457)
- RMT Onewire Peripheral (#454)
- Add an example of using interrupt directly without async (#474)
- Add support for ESP internal temperature sensor (#337)
- Support the thread modem peripheral on the c6 MCU
- Add dual and quad SPI support (#479)
- Initial USB-serial-jtag driver (#504)

### Fixed
- Fix: UartDriver as embedded_io::Read blocks until buffer is full (#475)
- Allow modem splits even when only HW coex is used
- Fix esp_reset_reason_t constant names (#483)
- Fix drop for AdcDriver in deregister event callbacks (#487)
- Typo fix in task docs (#489)
- Update spi.rs for correct label in gpio_cs capture (#492)
- Fix a crash when dropping `SpiDriver`

## [0.44.1] - 2024-07-09
### Fixed
* The crate now does build with ESP-IDF V4.4.x + esp32c2/esp32c6/esp32h2, yet these MCUs should only be used with ESP-IDF V5+
  as they are not officially supported with ESP-IDF V4.4.x (#450)
* Enum `ResetReason` not dealing with all reset reasons (panics on unknown reset reason) (#443, #444)

## [0.44.0] - 2024-06-23
### Deprecated
**ESP-IDF v4.4** Please start upgrading to ESP-IDF v5.
### Breaking
* **removed**: ESP-IDF v4.3 support, including mostly conditional compilations. (#420)
* GPIO20 is now included on esp32. Notice that not every esp32 has a physical GPIO20. (#370)
* ledc: now it can also be used with high_speed channels on esp32. (#424)
* dependency: updated embassy-sync to 0.6. (#437)
* adc: legacy adc driver is now behind a feature flag `adc-oneshot-legacy` starting with >= ESP-IDF v5.0. For justification, look at https://github.com/espressif/esp-idf/issues/13938. (#433)
* spi: cs_pre/post_trans_delay() config option included. (#266)
### Added
* rmt: `Symbol` now derives `Clone` and `Copy` (#386)
* reset: restart() function (#383)
* pcnt: Can now be used with esp32c6. (#407)
* can: Frame flags enum (#411)
* task: added MallocCap enum - Flag to indicate the capabilities of a Memory Region. (#419)
* i2s: new I2sDriver implementation allowing for splited bidirectional i2s. (#435)
* spi: added new_without_sclk SpiDriver constructor (#440)
### Fixed
* esp32h2 builds: added missing hys_ctrl_mode field (#387)
* rmt: FixedLengthSignal was broken, which led to failures in the neopixel/smartled examples, among other things. (#402)
* e-hal 0.2: implements DelayUs and DelayMs for u8 to be consistent with the other implementations (#414)
* i2c: wrong timeout calculations. (#415)
* task: Compatibility with ESP-iDF v5.3 (pre-release) - adding stack_alloc_caps to `ThreadSpawnConfiguration`. (#419)
* i2s: Compatibility with ESP-IDF v5.3 (pre-release) - small internal adjustments to i2s (#419)
* ledc: max_duty() method miscalulated in certain conditions. (#431)
* timer: fix clock usage in timer driver used with ESP-IDF v5.x (#441)
* i2c: use correct xtal for esp32c2 on timeout calculations. (#438)

## [0.43.1] - 2024-02-21
* Fix - PinDriver state changes and the drop call invoked pull-ups to be enabled. New default behavior on init / state transition / drop is to not enable pull-ups. (#344). If users want to reduce power usage on unused pins, they now need to manually enable pull-ups on a pin. For example, call `core::mem::forget` on the PinDriver instance after setting the pull-ups.
* #354 - breaking change - `rmt` driver now does not directly expose `rmt_item32_t` but rather - wraps it with a `Symbol` newtype
* #374 - Improve delay/timer errors and rounding calculations
* #379 - Remove 4096 limit for SPI dma transfer size
* Fix clippy duplicate imports warnings with latest 1.78 nightly

## [0.43.0] - 2024-01-26
* Breaking change: feature `riscv-ulp-hal` (and consequently, feature `esp-idf-sys`) is now removed. Use the [esp-ulp-riscv-hal](https://github.com/esp-rs/esp-hal/tree/main/esp-ulp-riscv-hal) crate instead
* MSRV 1.75; remove the nightly feature flag from all async trait implementations
* Update public dependencies `e-hal` to 1.0.0 and `embassy-sync` to 0.5 and private dependency `heapless` to 0.8
* Allow `cargo check --all-features` to work correctly (there used to be a build error when both the `esp-idf-sys` and the `riscv-ulp-hal` features were enabled)
* #365 - Async Uart driver is not `Send`
* #362 - Async Uart write hangs forever
* #353 - Use the `PLL_F80M` clock source with ESP IDF 5.1 and esp32c6
* #351 - Remove the `OutputPin` requirement from SPI SDI pin
* #350 - Not checking for ESP_FAIL in AsyncCanDriver::transmit

## [0.42.5] - 2023-11-12
* BREAKING CHANGE IN A PATCH RELEASE DUE TO DISCOVERED UB: The `subscribe` methods in drivers `PinDriver`, `PcntDriver` and `TimerDriver`
no longer accept non-static callbacks, as these lead to UB / crash when the driver is forgotten with e.g. `core::mem::forget`. Since local borrows
are a very useful feature however, these are still allowed via the newly-introduced and even more unsafe `subscribe_nonstatic` method.

## [0.42.4] - 2023-11-02
* Remove dependency on `AtomicU64` which is no longer supported by the upstream `*-espidf` targets
* Fix some Clippy warnings in the `spi` driver

## [0.42.3] - 2023-10-29
* Fix Timer array index bug #331 - prevented the use of TIMER10 on devices that support only 2 Timers
* Fix wrong TIMER11 index definition that declared TIMER11 as TIMER10 #331

## [0.42.2] - 2023-10-28
* Support for latest ESP IDF 5.2 dev (master)

## [0.42.1] - 2023-10-18
* Fix ambiguous name error #325 - a compilation issue when the NimBLE component is enabled in `esp-idf-sys`
* Fix compilation issues of the I2S driver for esp32h2 and esp32c2
* Fix compilation issues of the ADC drivers when the ESP IDF `esp_adc` component is not enabled
* Fix compilation issues of the GPIO driver for esp32c6

## [0.42.0] - 2023-10-17
* MSRV raised to 1.71
* New driver: I2S
* New driver: continuous ADC
* New driver: snapshot ADC, implementing the new ESP-IDF 5.0+ snapshot ADC API
* Async support in the following drivers: Timer, SPI, I2S, continuous ADC, CAN (via an `AsyncCanDriver` wrapper), UART (via an `AsyncUartDriver` wrapper)
* All async drivers can now work out of the box with any executor (`edge-executor` no longer a necessity) via a new ISR-to-task bridge (`esp_idf_hal::interrupt::asynch::IsrReactor`)
* CAN driver: support for pulling alerts in blocking and async mode
* UART driver: support for pulling UART events
* GPIO driver: scoped callback; `subscribe` callback now needs to live only as long as the `PinDriver` instance
* Timer driver: scoped callback; `subscribe` callback now needs to live only as long as the `TimerDriver` instance
* `task` and `interrupt` modules: new submodule in each - `asynch` - featuring a signal/notification-like synchronization primitive
* `task` module: `block_on` method capable of executing a future on the current thread
* `task` module: new sub-module - `queue` for the FreeRTOS `queue` synchonization primitive
* `task` module: new sub-module - `notification` - a more ergonomic API around the FreeRTOS task notification API
   which was already exposed via the `task::notify` and `task::wait_notification` APIs
* Upgraded to `embedded-hal` 1.0.0-rc.1 and `embedded-hal-async` 1.0.0-rc.1
* Dependency `esp-idf-sys` now re-exported as `esp_idf_hal::sys`
* Breaking change: `Peripherals::take` now returns an error when the peripherals are already taken
* Breaking change: `delay::Delay` struct extended with configurable threshold
* Breaking change: `task::wait_any_notification` removed; `task::notify` renamed to `task::notify_and_yield`; new function - `task::notify` - that notifies a task without automatically yielding to the notified task if it is a higher priority than the currently interrupted one
* Breaking change: `task::notify*` and `task::wait` now take/return `NonZeroU32` instead of `u32`
* Breaking change: GPIO - interrupts are now disabled automatically each time an ISR is triggered so as to avoid the IWDT triggering on level interrupts; use has to re-enable - in non-ISR code - via `PinDriver::enable_interrupt()`
* Breaking change: ESP IDF support for `edge-executor` moved to the `edge-executor` crate
* Deprecated: Using ESP-IDF 4.3 is now deprecated and all special cfg flags will be removed in the next release

## [0.41.2] - 2023-06-21

* Do not set the single shot flag in the CAN frame (#263)
* Add safe abstractions to CRC functions in ESP ROM (#261)
* Use same error for all e-hal 0.2 trait impls (#260)
* Add support for WakeupReason from Ext0 events (#259)
* Compilation failed when ESP_IDF_VERSION = "release/v5.1" (#258)
* Fix UART docs (#252)

## [0.41.1] - 2023-05-21

* UART driver was broken - see #250 for more details
* Removed a forgotten `[patch.crates-io]` section that was using `esp-idf-sys` from `master` (low impact, only relevant when building the examples)
* Clippy fixes

## [0.41.0] - 2023-05-13 (Yanked due to #250, see above)

* MSRV 1.66 (but MSRV 1.70 necessary if `nightly` is enabled)
* Support for new chips: esp32c2, esp32h2, esp32c6 and future proofed for esp32c5 and esp32p4
* Support for ESP IDF 5.0, 5.1 and 5.2 (master)
* Support for `embedded-hal` 1.0-alpha.10 and `embedded-hal-async` 0.2.0-alpha.2 (API breakage in i2c and spi)
* Async GPIO native API with support for `embedded-hal-async`
* PCNT driver
* Alerts and Mode support in the TWAI driver
* Task Watchdog API
* Configurable interrupt levels in all drivers (minor API breakage in `SpiDriver`)
* `EspError::from_infallible`
* Auto-reload support in the timer driver

## [0.40.1] - 2022-12-13

Fix the build when the ULP peripheral is enabled.

## [0.40] - 2022-12-09

Rebase on top of `esp-idf-sys` 0.32:
* Retire any usages of `esp-idf-sys::c_types` in favor of `core::ffi`
* Remove casts from `usize` to `u32` and back now that `esp-idf-sys` is compiled with `--size_t-is-usize` enabled

## [0.39.3, 0.39.4] - 2022-12-09

Minor doc fixes; Clippy fixes; `CriticalSection` will panic on FreeRTOS mutex lock/unlock failures.

## [0.39.1, 0.39.2] - 2022-11-21

Patch releases to fix compilation errors under no_std.

## [0.39] - 2022-11-01

Release 0.39 is a backwards-incompatible release where almost all drivers were touched in one way or another.

### Major Changes

The main themes of the 0.39 release are:
* Restore drop semantics for all drivers
* Simpler driver types (little to no generics)
* Unified naming
* Public API
* Completely revamped GPIO metaphor
* Timer driver
* Support for the `critical-section` crate by providing a `CriticalSection` implementation
* Support for the `embassy-sync` crate by providing two types of raw mutexes
* Support for the `edge-executor` crate
* Modem and Mac peripherals
* SPI driver rework

### Major changes elaboration

#### Restore drop semantics for all drivers

* The `release()` method is now retired everywhere
* Just dropping a driver will make it stop functioning
* If peripherals need to be reused after the driver is dropped, this is achieved by passing the peripherals to the driver constructor via `&mut` references, e.g. `I2cMasterDriver::new(&mut peripherals.i2c1, &mut peripherals.pins.gpio10, &mut peripherals.pins.gpio11, &mut peripherals.pins.gpio12)`

### Simpler driver types

* All peripheral generics are removed from all drivers' types; e.g., the I2C master driver now has the following type signature: `I2cMasterDriver<'d>`
* The lifetime - `'d` from above designates the lifetime of the peripheral (e.g. `I2c`) that is used by the driver
* In the most common case where the peripheral is just moved into the driver (and no longer accessible when the driver is dropped), `'d` = `'static`, or in other words, the type signature of the driver becomes `I2cMasterDriver<'static>` and can even be type aliased by the user as in `pub type SimpleI2cMasterDriver = I2cMasterDriver<'static>;`
* When the peripherals are passed to the driver using `&mut` references, `'d` designates the lifetime of the `&mut` reference
* Note that the constructor of each driver is still generic by the peripherals it takes, but these generics are "erased" from the type of the driver once the driver is constructed, as we only need to "remember" whether the peripheral is moved into the driver, or used via a `&mut` reference and can be reused after the driver is dropped. This semantics is captured by the `'d` lifetime

### Unified naming

* Each driver is named `<Peripheral>Driver`, where `<Peripheral>` is the name of the peripheral. E.g. the driver for the `Uart` peripheral is named `UartDriver`, the driver for the `Adc` peripheral is named `AdcDriver` and so on
* The one exception is the GPIO subsystem, where the driver is not named `GpioDriver`, but the simpler `PinDriver` instead, even if the concrete peripheral struct type might be e.g. `Gpio12`
* In case there are multiple drivers per peripheral - as in - say - a master and a slave protocol driver - only the `Slave` contains itself in its name, as in e.g. `SpiDriver` and `SpiSlaveDriver`
* NOTE: We are NOT fond of still using offensive terms like "master" and "slave". Unfortunately, the embedded industry itself has not yet agreed (to our knowledge) on a good replacement for these terms, hence they are still around

### Public API

In addition to implementing the `embedded-hal` traits where possible, all drivers now have public API. While the public API loosely follows the APIs from `embedded-hal`, it deviates where appropriate so that the native underlying ESP IDF driver is better utilized.

These public APIs mean that the user is no longer required to depend on the `embedded-hal` crate so as to consume the `esp-idf-hal` drivers.
Consuming the drivers via the `embedded-hal` traits is now only necessary when the user is targetting cross-platform portability of their application code.

### Completely revamped GPIO metaphor

In previous releases, the GPIO driver was a bit different from the others, in that each *pin peripheral itself* used to have TypeState and methods that configured the peripheral into a certain mode (as in e.g. input-only, input-output, output-only, ADC and so on) that drived the peripheral through a type-state change.

Furthermore, there was no notion of a pin "driver". The driver methods were fused directly into the peripheral, and a subset of these methods were available, depending on the type-state of the peripheral.

This scheme was asymmetric with the rest of the HAL, where peripherals are mostly opaque singleton objects, and it is the driver that exposes a user visible API (including implementation of the corresponding `embedded-hal` traits). It also suffered from less flexibility and code duplication accross the regular GPIO pins, and their degraded `Any*Pin` variants.

The new GPIO subsystem follows the driver-peripheral separation metaphor like everything else:
* Pin peripherals are static objects with no type-state, where the only allowed operation is to degrade a concrete pin instance implementing the `Pin` / `InputPin` / `OutputPin` traits
into a corresponding `Any*Pin` struct which is useful when e.g. passing an array of input or output pins accross the application code, as generics are erased by the degrading operation
* The capabilities of a pin peripheral are expressed by the concrete pin struct type (i.e. `GpioXX`) implementing a set of traits (`InputPin`, `AnalogPin`) and so on
* `PinDriver` is the universal driver for any pin peripheral (e.g. be it a non-degraded `GpioXX` struct, or a degraded `AnyIOPin`, `AnyOutputPin` or `AnyInput` pin struct)
* Providing an ISR handler is possible - via `PinDriver` - on any input pin, including on a degraded one

Finally, `PinDriver` can now work with digital pins by using the RTC digital IO API on chips that support this (ESP32 and the ESP32 S* series)

### Timer driver

The new `timer` module is exposing the hardware timer peripherals on the esp32* MCUs via the ESP-IDF timer driver. Just like the underlying ESP-IDF timer driver, a lot (but not all)
methods are actually safe to call from an ISR routine. Those which are unsafe will panic immediately if called from an ISR.

### Support for the `critical-section` crate

In addition to implementing `embedded-hal` crates, `esp-idf-hal` integrates with the wider Rust embedded ecosystem by providing out of the box critical section implementation for the `critical-section` crate.

To enable this support, you need to compile the crate with the `critical-section` feature enabled.

Note that the provided critical section implementation is based on `esp_idf_hal::task::CriticalSection`, which itself is based on a recursive FreeRtos mutex.
This means that you can `enter()` the critical section for long periods of time, and your code can still be preempted by ISR routines or high priority FreeRtos tasks (threads), thus you should not worry about slowing down the whole RTOS reaction times by using a critical section.
This also means however, that this critical section (and its native `esp_idf_hal::task::CriticalSection` equivalent) CANNOT be used from an ISR context.

If you need a critical section with a warranty that your code will not be interrupted by an ISR or higher priority tasks (i.e. working accross ISRs and tasks), you should be using
`esp_idf_hal::interrupt::IsrCriticalSection` instance, or the `esp_idf_hal::interrupt::free` method. Keep in mind that you should stay in this type of critical section for a very short period of time, because it **disables all interrupts**.

### Support for the `embassy-sync` crate

The HAL provides two mutex implementations that implement the `RawMutex` trait from the `embassy-sync` crate thus integrating the ESP-RS ecosystem better with the Rust embedded async ecosystem:
* `EspRawMutex` - this implementation uses a FreeRtos mutex and behaves similarly to `esp_idf_hal::task::CriticalSection`
* `IsrRawMutex` - this implementation works by disabling all interrupts and thus behaves similarly to `esp_idf_hal::interrupt::IsrCriticalSection`

To enable this support, you need to compile the crate with the `embassy-sync` feature enabled.

While `embassy-sync` itself can use - via its `CriticalSectionRawMutex` mutex bridge - whatever a HAL is providing as a critical section to the `critical-section` crate, the above two implementations provide further benefits:
* `EspRawMutex` is almost the same as the `embassy-sync` native `CriticalSectionRawMutex` with the one major difference that ALL `CriticalSectionRawMutex` instances in fact share a **single** RTOS mutex underneath. This is a limitation which is coming from the `critical-section` crate itself. In contrast, each `EspRawMutex` instance has its own separate RTOS mutex
* `IsrRawMutex` - as the name suggests - allows `embassy-sync` to be utilized in scenarios where the application code should synchronize accross ISRs and regular RTOS task based code

### Support for the `edge-executor` crate

This is another HAL feature that integrates the ESP-RS ecosystem with the Rust embedded async ecosystem.

The [`edge-executor`](https://github.com/ivmarkov/edge-executor) crate is a simple local-only executor that is based on the popular `async-task` crate coming from the Smol async executor (which in turn powers the `async-std` executor).
Without getting into too many details, `edge-executor` is useful in embedded cotexts because
* It is `no_std` compatible (but requires an allocator, unlike [`embassy-executor`](https://github.com/embassy-rs/embassy), so the latter might be a better fit for bare-metal use cases where ESP IDF is not utilized)
* Has a small footprint
* Is ISR-friendly in that it can operate from an ISR context - or - which would be the natural setup when used on top of a RTOS like ESP IDF's FreeRtos - can be **awoken** directly from an ISR routine, this having a minimal latency

To enable this support, you need to compile the crate with the `edge-executor` feature enabled.

The support for `edge-executor` in `esp-idf-hal` is in terms of providing an ISR-friendly `Monitor` trait implementation for `edge-executor` - `FreeRtosMonitor` - that can be awoken directly from an ISR, and is based on the FreeRtos task notification mechanism which is exposed in the `esp_idf_hal::task` crate.

### Modem and Mac peripherals

In previous releases, the Wifi and Ethernet drivers (part of the `esp-idf-svc` crate) did not follow the philosophy of `esp-idf-hal` in that each driver should take a corresponding peripheral (by moving or by a `&mut` reference).

This is now addressed in that the `esp-idf-hal` crate models two new peripherals:
* `modem::Modem`: models the esp32* hardware modem peripheral and implements two new marker traits: `modem::WifiModemPeripheral` and `modem::BluetoothModemPeripheral`
  * By implementing both of the above traits, the `modem::Modem` peripheral should be usable by both the Wifi driver in `esp-idf-svc`, as well as the future Bluetooth driver
  * `modem::Modem` is splittable into two other peripherals: `modem::WifiModem` (implementing only the `modem::WifiModemPeripheral` trait) and `modem::BluetoothModem` (implementing only the `modem::BluetoothModemPeripheral` trait) so that in future the simultaneous operation of the Wifi and Bluetooth drivers to be possible
* `mac::MAC`: models the EMAC hardware available on the original ESP32 chip. The Ethernet driver implementation - just like the Wifi driver implementation - is still in the `esp-idf-svc` crate which better aligns with the underlying ESP-IDF implementations, yet the new Wifi and Ethernet drivers in `esp-idf-svc` now expect their corresponding peripherals to be supplied during construction time

### SPI driver rework

The SPI driver is now split into two structures
* `SpiDriver` - represents an initialized SPI bus and manages access to the underlying SPI hardware
* `SpiDeviceDriver` (new) - an abstraction for rach device connected to the SPI bus

The above split allows for the creation of more than one device per SPI bus. (Up to 6 for the esp32c* variants and 3 for all others).
When creating an `SpiDeviceDriver` instance, user is required to provide a `Borrow` to the `SpiDriver` instance (as in `SpiDriver`, `&SpiDriver`, `&mut SpiDriver`, `Rc(SpiDriver)` or `Arc(SpiDriver)`), so that the SPI bus stays initialized throughout the liftime of all devices.

A second wrapper implementation is now also provided: `SpiSoftCsDeviceDriver` It allows for more devices per SPI bus (i.e. above the 3/6 limit). This is implemented by operating the CS pin in software mode.
