#![allow(non_camel_case_types, non_upper_case_globals)]

// The following is defined to remove a case where bindgen can't handle pcnt_unit_t being defined
// in two different C namespaces (enum vs struct). The struct is opaque (used only as a pointer to an
// opaque type via pcnt_channel_handle_t), so we use the enum definition here, taken from the v4
// bindgen.

/// Selection of all available PCNT units
pub type pcnt_unit_t = core::ffi::c_int;

/// PCNT unit 0
pub const pcnt_unit_t_PCNT_UNIT_0: pcnt_unit_t = 0;

/// PCNT unit 1
pub const pcnt_unit_t_PCNT_UNIT_1: pcnt_unit_t = 1;

/// PCNT unit 2
pub const pcnt_unit_t_PCNT_UNIT_2: pcnt_unit_t = 2;

/// PCNT unit 3
pub const pcnt_unit_t_PCNT_UNIT_3: pcnt_unit_t = 3;

// Ideally, we'd use a conditional off of SOC_PCNT_UNITS_PER_GROUP, but that's not possible in Rust.
// Today, ESP32 is the only chip that has 8 units. All others have 4 (except ESP32-C3, which doesn't
// have PCNT at all). For new chips, check $IDF_PATH/components/soc/$CHIP/include/soc/soc_caps.h for
// for the value of SOC_PCNT_UNITS_PER_GROUP.

#[cfg(esp32)]
/// PCNT unit 4
pub const pcnt_unit_t_PCNT_UNIT_4: pcnt_unit_t = 4;

#[cfg(esp32)]
/// PCNT unit 5
pub const pcnt_unit_t_PCNT_UNIT_5: pcnt_unit_t = 5;

#[cfg(esp32)]
/// PCNT unit 6
pub const pcnt_unit_t_PCNT_UNIT_6: pcnt_unit_t = 6;

#[cfg(esp32)]
/// PCNT unit 7
pub const pcnt_unit_t_PCNT_UNIT_7: pcnt_unit_t = 7;

// For some reason, this isn't defined in soc_caps.h for ESP32-H2 on ESP-IDF v4.x

/// Maximum number of PCNT units
#[cfg(not(all(any(esp32h2, esp32h21), esp_idf_version_major = "4")))]
pub const pcnt_unit_t_PCNT_UNIT_MAX: pcnt_unit_t = crate::SOC_PCNT_UNITS_PER_GROUP as pcnt_unit_t;

/// Maximum number of PCNT units
#[cfg(all(any(esp32h2, esp32h21), esp_idf_version_major = "4"))]
pub const pcnt_unit_t_PCNT_UNIT_MAX: pcnt_unit_t = 4;
