//! This module contains compile-time checks only, and will generate no code.

/// If any of the two constants below do not compile, the `time_t` of the `libc` crate
/// does not match the one of ESP-IDF.
///
/// ESP-IDF 5.0 and later use a 64-bit `time_t`, which is also the default of the `libc` crate.
/// When compiling against ESP-IDF 4.x, add the rustc cfg flag `espidf_time32` in the `.cargo/config.toml` file
/// of your binary crate, for example:
/// ```toml
/// [target.'cfg(target_os = "espidf")']
/// rustflags = ["--cfg", "espidf_time32"]
/// ```
///
/// When compiling against ESP-IDF 5.0 or later, remove that flag.
#[allow(deprecated)]
#[allow(unused)]
#[cfg(feature = "std")]
const ESP_IDF_TIME64_CHECK: ::std::os::espidf::raw::time_t = 0 as crate::time_t;
#[allow(unused)]
const ESP_IDF_TIME64_CHECK_LIBC: ::libc::time_t = 0 as crate::time_t;

// Check for libc/esp-idf-sys type and constant mismatches.
#[cfg(feature = "std")]
mod libc;
