//! Checks the POSIX binding of the standard file descriptors 0, 1 and 2.
//!
//! The Rust Standard Library reads `stdin` from descriptor 0 and writes
//! `println!`/`eprintln!` output to descriptors 1 and 2, so these have to be
//! bound to the console for its standard streams to work. Binding them is what
//! `esp_idf_sys::restore_posix_stdio_fds` - called from the `app_main` glue
//! before `main` - does.
//!
//! Expected output (the "raw write" lines and the `println!`/`eprintln!` lines
//! must all appear, and the raw writes must report the number of bytes they
//! were given):
//!
//! ```text
//! --- POSIX stdio descriptor check ---
//! restore_posix_stdio_fds() -> 1
//! raw write to descriptor 1 (Rust stdout) returned 44
//! raw write to descriptor 2 (Rust stderr) returned 44
//! Rust println! works
//! Rust eprintln! works
//! --- done ---
//! ```
//!
//! Note that the diagnostics above are printed with the C `printf`, which goes
//! through the C `stdout` stream rather than through descriptor 1, so they show
//! up even when the descriptors are *not* bound.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

use core::ffi::c_void;

use std::ffi::CString;

use esp_idf_sys::{restore_posix_stdio_fds, write, STDERR_FILENO, STDOUT_FILENO};

fn main() {
    cprintln("--- POSIX stdio descriptor check ---");

    // The `app_main` glue has already called this before `main`; calling it
    // again only reports the state, as the call is idempotent
    let bound = restore_posix_stdio_fds();
    cprintln(&format!("restore_posix_stdio_fds() -> {}", bound as u8));

    // Write to the raw descriptors, which is what the Rust Standard Library
    // does. A negative result means the descriptor is not bound to the console
    let out = raw_write(
        STDOUT_FILENO as _,
        "raw write to descriptor 1 (Rust stdout)\r\n",
    );
    let err = raw_write(
        STDERR_FILENO as _,
        "raw write to descriptor 2 (Rust stderr)\r\n",
    );

    cprintln(&format!(
        "raw write to descriptor 1 (Rust stdout) returned {out}"
    ));
    cprintln(&format!(
        "raw write to descriptor 2 (Rust stderr) returned {err}"
    ));

    println!("Rust println! works");
    eprintln!("Rust eprintln! works");

    cprintln("--- done ---");
}

/// Prints via the C `stdout` stream, which - unlike `println!` - does not go
/// through descriptor 1
fn cprintln(msg: &str) {
    let msg = CString::new(msg).unwrap();

    unsafe {
        esp_idf_sys::printf(c"%s\n".as_ptr(), msg.as_ptr());
    }
}

/// Writes to a raw descriptor, returning the result of the `write` syscall
/// (`ssize_t` is not the same Rust type on all ESP-IDF targets, hence the cast)
fn raw_write(fd: i32, msg: &str) -> i64 {
    unsafe { write(fd, msg.as_ptr() as *const c_void, msg.len()) as i64 }
}
