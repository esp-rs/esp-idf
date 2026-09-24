# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Support for the esp32s31 (ESP-IDF v6.1+; select it with `MCU=esp32s31` on the `riscv32imafc-esp-espidf` target)

### Fixed
- Sort the extra components' dirs so that we get a predictable build hash by the ESP-IDF Component Manager

## [0.38.1] - 2026-09-16

### Fixed
- Add all Mbedtls 3.x headers
- `checks/libc.rs`: gate IPv6-specific checks on `esp_idf_lwip_ipv6` instead of the coarser `esp_idf_comp_lwip_enabled`, so builds with `CONFIG_LWIP_IPV6=n` no longer fail.
- `checks/libc.rs`: additionally gate termios checks on `esp_idf_vfs_support_termios`, so pre-6.0 builds with `CONFIG_VFS_SUPPORT_TERMIOS=n` no longer fail.

## [0.38.0] - 2026-09-15

### Added
- Basic compatibility for ESP-IDF release 6.0
- Added ESP_IDF_SYS_EXTRA_COMPONENTS_FILE env variable support, allows to select `extra_components` from command line. Probably only useful for CI.
- Added raw bindings to the `esp_heap_task_info.h` API

### Breaking
- Added build-time check for symbol compatibility between `libc` and symbols defined in this library. If you see `libc/esp-idf-sys * mismatch` errors at build time, you might need to update your pinned `libc` version.

### Fixed
- Issue 421: the `libc` compatibility checks failed when `ESP_IDF_COMPONENTS` is trimmed
- Discover the CMake-selected compiler's sysroot for picolibc binding generation in CMake-first builds.
- Fix https://github.com/esp-rs/esp-idf-hal/issues/592 - new API tha does the fix - `restore_posix_stdio_fds` - called automatically form the `binstart` bootstrapping code (#425). Note that the fix ONLY works for ESP-IDF >= v5.3. For earlier ones, use one of the workarounds described in the PR
- Include `soc/gpio_sig_map.h` on ESP-IDF 6 so `SIG_GPIO_OUT_IDX` is available in the bindings
- Fix E0588 compile error in the generated bindings when the TinyUSB CDC class is enabled (`CONFIG_TINYUSB_CDC_ENABLED=y`) by blocklisting TinyUSB's unused `cdc_desc_func_telephone_call_state_reporting_capabilities_t` descriptor

## [0.37.2] - 2026-03-10

### Fixed
- An issue with the sdmmc component and CMake build

## [0.37.1] - 2026-03-10

### Fixed
- Fix several compilation issues with esp32p4

## [0.37.0] - 2026-03-09

### Breaking
- Support for the `esp_app_desc!` macro removed for all versions prior to ESP-IDF V5.1.0
  - Reason: the "patch-ESP-IDF-during-cargo-build" mechanism was retired

### Deprecated
- Support for all ESP-IDF releases < 5.3.0.
  - See https://docs.espressif.com/projects/esp-idf/en/latest/esp32/versions.html#support-periods
  - Note that we are also deprecating 5.1.x and 5.2.x even if the ESP-IDF support for those is not officially over,
    due to significant changes in the ESP-IDF driver code layout since ESP-IDF 5.3.x

### Added
- `esp_idf_version_at_least_X_Y_Z`, `esp_idf_version_patch_at_least_X_Y_Z` and  `esp_idf_version_patch_at_most_X_Y_Z`
  `cfg` constants for easier conditional compilation against various ESP-IDF versions
- Compatibility with ESP-IDF v5.4.X, v5.5.x
- Support for `esp32c5` and `esp32c61`
- Add raw bindings for `esp_lcd_mipi_dsi.h` and `esp_ldo_regulator.h` (#398)
- Pulse counter bindings for esp32p4
- `impl core::error::Error for EspError`
- Added bindings for esp_wifi_remote and esp_hosted

### Fixed
- Fix the `esp_app_desc!` macro so the reserv3 field is correct for ESP IDF v5.4
- Fix the `esp_app_desc!` macro so that it properly embeds `CONFIG_ESP_EFUSE_BLOCK_REV_MIN_FULL`/`CONFIG_ESP_EFUSE_BLOCK_REV_MAX_FULL` with recent ESP-IDF versions (5.4+, 5.2.3+, 5.3.2+)
- Make sure that the `esp_app_desc!` macro fails at build-time if some of the fields cannot fit inside the structure
- Re-enable "esp_lcd_panel_rgb.h" after >= v5.3 for targets with SOC_LCD_RGB_SUPPORTED
- Fix typo (commuity -> community) in README.md

## [0.36.1] - 2025-01-10

### Added
- Add raw bindings for `esp_netif_net_stack.h` and `lwip/esp_netif_net_stack.h`. (#360)
- Add raw bindings for `mqtt5_client.h` when `CONFIG_MQTT_PROTOCOL_5` is set.

### Fixed
- Fix the `esp_app_desc!` macro so that it works with recent ESP-IDF versions (5.4+, 5.2.3+, 5.3.2+)

## [0.36.0] - 2025-01-02

### Deprecated

### Breaking
- Updated the `embuild` dependency

### Added
- Compatibility with ESP-IDF v5.3.X
- Add raw bindings for `sdio_slave.h` for all targets that support it and add `onewire_bus.h` (#322)
- Add raw bindings for `onewire_device.h`
- Add raw bindings for `esp_netif_ppp.h`
- Add raw bindings for `esp_efuse_table.h`, `esp_https_ota.h`
- Add raw bindings for OpenThread (#339)
- Add raw bindings to the internal DNS resolver API. (#340)
- Add raw bindings for `esp_littlefs.h`
- Add raw bindings for `esp_netif_br_glue.h`
- Add raw bindings to the `mbedtls` API
- Use clang from ESP-IDF when possible
- Use the `bindgen` crate re-exported from `embuild` rather than depending directly on `bindgen`
- Add raw bindings to the `TinyUSB` API

### Fixed
- Fix rust-analyzer by not using rust libtest harness
- Fix a typo in `BUILD-OPTIONS.md`
- Bugfix: date and time were swapped in the generated esp_app_desc_t
- Update to the latest-released cmake-rs fixing build issues on MacOS
- Raise recommended ESP IDF to 5.2.3; always print recommended build versions; exclude master and release branches from the recommendation
- Fix the build against ESP-IDF master by installing the ROM ELFs
- Support for building on nix (ESP-IDF no longer needs to be a GIT repo) (#353, #356)

## [0.35.0] - 2024-06-23

### Deprecated
**ESP-IDF v4.4** Please start upgrading to ESP-IDF v5.
### Breaking
* removed: ESP-IDF v4.3 support - including code and patches. (#305)
* removed: Unused esp8266 rtos sdk (#305)
* EspError now implements `Debug` and provides a human readable error code. (#294)
* `bindgen` dependancy updated to the latest release version. (#304)
### Added
* Include new dac_oneshot and dac_continous headers in default bindings.h. (#289)
* Include new i2c headers in default bindings.h. (#290)
* Include new rmt headers in default bindings.h.
* Include usb-serial-jtag header. (#317)
### Fixed
* Building against esp-idf >= v5.3 by adding additional conditions on the esp_lcd headers. Note that "esp_lcd_panel_rgb.h" was removed upstream since idf 5.3. (#303)

## [0.34.1] - 2024-02-21
* Fix clippy duplicate imports warnings with latest 1.78 nightly

## [0.34.0] - 2024-01-26
* First `esp-idf-sys` examples:
  * `std_basics`: "Hello world" with `println!` and other types available in the Rust Standard Library like threads, atomics, local storage, collections, etc.
  * `unsafe_call`: Calling an ESP IDF custom API using the unsafe bindings generated by `esp-idf-sys`
* All FreeRTOS headers are now included, so user should get unsafe bindings for all FreeRTOS APIs which are not macros
* Changes to how native vs PlatformIO build is selected, that are supposed to increase the ergonomics of using the crate:
  * Perform PlatformIO build **only** when the `pio` feature IS specified, and the `native` feature is NOT specified
  * In case neither the `native` nor the `pio` feature is specified, perform a native build (this setup used to fail the build)
  * In case both the `native` and `pio` features are specified, perform a native build as well (this setup used to perform a PlatformIO build)
* When both `binstart` and `libstart` features are selected, `binstart` takes precedence over `libstart`
* #264 - Copy the bootloader and partition table binaries to the target folder
* #262 - Bindings for the `esp_lcd` driver component
* #259 - Bindings for the temperature sensor driver
* #261 - Build time optimization - do not download Rust crates not needed for the host platform
* #257 - Make builds utilizing the `esp_app_desc` component reproducible
* (Bugfix) Re-expose raw bindings for the `esp_flash` component on ESP IDF 5+

## [0.33.7] - 2023-11-08
* Workaround for https://github.com/esp-rs/esp-idf-svc/issues/312
* Include gptimer headers (#255)

## [0.33.6] - 2023-11-07
* Export esp_netif_sntp APIs from esp-idf 5.1
* Fix compile error for esp32c6 with NimBLE
* Support for the [symlink to xtensa Clang library](https://github.com/esp-rs/espup/releases/tag/v0.8.0) installed by latest `espup` ([esp-idf-svc issue 319](https://github.com/esp-rs/esp-idf-hal/issues/319))

## [0.33.5] - 2023-10-28
* Support for latest ESP IDF 5.2 dev (master)

## [0.33.4] - 2023-10-27
* The `MCU` environment variable was failing the `pio` build if the MCU was not uppercased
* Better error message for the `native` build in case the MCU was not recognized

## [0.33.3] - 2023-10-17
* Support for ESP IDF Component Manager - check the documentation in BUILD-OPTIONS.md
* ESP32H2 and ESP32C5 now properly assigned to the `riscv32imac-esp-espidf`
* All ESP IDF WPA supplicant APIs exposed
* Build is now checking for the presence of certain environment variables (e.g. CXXFLAGS) that might fail the ESP IDF C build and removing those
* Build is now checking if the project path might fail the ESP IDF C build (i.e. too long on Windows or containing spaces on Unix) and failing if so

## [0.33.2] - 2023-08-18
* Band-aid solution that fixes the build with recent Rust nightlies and ESP IDF < 5.1 (https://github.com/esp-rs/esp-idf-template/issues/149)
* Raw bindings for the continuous ADC driver (ESP IDF >= 5.0)
* Raw bindings for bootloader random functions
* Raw bindings for all available classic BT APIs
* Raw bindings for esp_freertos_hooks.h

## [0.33.1] - 2023-06-11

* Raw bindings for the I2S driver
* Raw bindings for CRC ROM functions

## [0.33.0] - 2023-05-13

* (In theory) no API breakage, yet the minor version is raised just in case
* Support for new chips: esp32c2, esp32h2, esp32c6 and future proofed for esp32c5 and esp32p4
* Support for ESP IDF 5.0, 5.1 and 5.2 (master)
* New raw bindings: esp-transport, himem, psram, esp-dpp, i2s, a2dp, wpa2

## [0.32.1] - 2022-12-13

* Fix an erroneous cast to `u32` in `Esp32Alloc`, causing `no_std` builds using the allocator to fail (#158)
* Apply Niche optimization to `EspError` (`NonZeroI32`), add `from_infallible` associated function, (#159)

## [0.32] - 2022-12-09

* Remove the custom `c_types` module in favor of `core::ffi`
* Switch to `embuild` 0.31 and `bindgen` 0.63. Since 0.61, `bindgen` has the `--size_t-is-usize` flag is enabled by default. This removes a lot of unnecessary casting from `usize` to `u32` and makes the `esp-idf-sys` bindings more ergonomic
