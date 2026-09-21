//! A super-simple example of calling an unsafe API provided by `esp-idf-sys` / ESP IDF
//! and not otherwise available via the Rust Standard Library

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

use esp_idf_sys::esp_get_free_heap_size;

fn main() {
    let free_memory = unsafe { esp_get_free_heap_size() } / 1024;

    println!("Free memory: {free_memory}KB");
}
