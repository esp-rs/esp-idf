//! Raw Rust bindings for the [ESP-IDF SDK](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/).
//!
//! # Build Prerequisites
//!
//! Follow the [Prerequisites](https://github.com/esp-rs/esp-idf-template#prerequisites) section in the `esp-idf-template` crate.
//!
#![doc = include_str!("../BUILD-OPTIONS.md")]
#![no_std]
#![cfg_attr(
    all(not(feature = "std"), feature = "alloc_handler"),
    feature(alloc_error_handler)
)]
#![allow(unknown_lints)]
#![allow(renamed_and_removed_lints)]
#![allow(unexpected_cfgs)]

pub use bindings::*;
pub use error::*;

// Don't use esp_idf_soc_pcnt_supported; that's only on ESP-IDF v5.x+.
// pcnt_unit_t and friends are only needed for the legacy PCNT API (removed in v6.0).
#[cfg(all(
    not(esp_idf_version_at_least_6_0_0),
    any(esp32, esp32s2, esp32s3, esp32c5, esp32c6, esp32h2, esp32h21, esp32p4)
))]
pub use pcnt::*;

#[doc(hidden)]
pub use build_time;
#[doc(hidden)]
pub use const_format;
#[doc(hidden)]
pub use patches::PatchesRef;

#[cfg(feature = "std")]
#[allow(unused_imports)]
#[macro_use]
extern crate std;

#[cfg(feature = "alloc")]
#[allow(unused_imports)]
#[macro_use]
extern crate alloc;

mod alloc;
#[cfg(esp_idf_version_at_least_5_1_0)]
mod app_desc;
mod error;
mod panic;
mod patches;
#[cfg(all(
    not(esp_idf_version_at_least_6_0_0),
    any(esp32, esp32s2, esp32s3, esp32c5, esp32c6, esp32h2, esp32h21, esp32p4)
))]
mod pcnt;

mod checks;
mod start;
mod stdio;

pub use stdio::restore_posix_stdio_fds;

/// A hack to make sure that a few patches to the ESP-IDF which are implemented in Rust
/// are linked to the final executable
///
/// Call this function once at the beginning of your main function
pub fn link_patches() -> PatchesRef {
    patches::link_patches()
}

#[allow(clippy::all)]
#[allow(unnecessary_transmutes)]
#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(rustdoc::all)]
#[allow(improper_ctypes)]
// TODO: For now, as 5.0 spits out tons of these
// bindgen maps `size_t` to `c_uint`, but std expects `usize` for these runtime
// symbols; on the 32-bit ESP targets `c_uint == usize`, so the signatures are compatible
#[allow(suspicious_runtime_symbol_definitions)]
#[allow(dead_code)]
mod bindings {
    #[cfg(all(
        not(esp_idf_version_at_least_6_0_0),
        any(esp32, esp32s2, esp32s3, esp32c5, esp32c6, esp32h2, esp32h21, esp32p4)
    ))]
    use crate::pcnt::*;

    include!(env!("EMBUILD_GENERATED_BINDINGS_FILE"));
}
