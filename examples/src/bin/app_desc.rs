//! This example just calls the `esp_idf_sys::esp_app_desc! {}` macro
//! sp as to setup the `esp_app_desc` structure with default information.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(esp_idf_version_at_least_5_1_0)]
esp_idf_sys::esp_app_desc! {}

fn main() {
    #[cfg(not(esp_idf_version_at_least_5_1_0))]
    panic!("The `esp_app_desc!` macro is only available for ESP-IDF V5.1.0+");
}
