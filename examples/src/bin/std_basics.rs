//! This example does not use anything from the `esp-idf-sys` unsafe API
//! but demonstrates, that *linking* with the `esp-idf-sys` library artefacts (and with the Rust Standard Library)
//! does provide the Rust STD layer on top of ESP IDF!

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]
#![allow(renamed_and_removed_lints)]
#![allow(clippy::thread_local_initializer_can_be_made_const)]

use core::cell::RefCell;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use std::io;
use std::thread;

thread_local! {
    static TLS: RefCell<u32> = RefCell::new(13);
}

fn main() -> Result<(), io::Error> {
    // Get backtraces from anyhow; only works for Xtensa arch currently
    #[cfg(target_arch = "xtensa")]
    std::env::set_var("RUST_BACKTRACE", "1");

    test_print();

    test_atomics();

    test_threads()?;

    #[cfg(not(esp_idf_version = "4.3"))]
    test_fs()?;

    loop {
        println!("Sleeping for 2 seconds...");
        thread::sleep(Duration::from_secs(2));
    }
}

#[allow(clippy::vec_init_then_push)]
fn test_print() {
    // Start simple
    println!("Hello from Rust!");

    // Check collections
    let mut children = vec![];

    children.push("foo");
    children.push("bar");
    println!("More complex print {children:?}");
}

#[allow(deprecated)]
fn test_atomics() {
    let a = AtomicUsize::new(0);
    let v1 = a.compare_and_swap(0, 1, Ordering::SeqCst);
    let v2 = a.swap(2, Ordering::SeqCst);

    let (r1, r2) = unsafe {
        // don't optimize our atomics out
        let r1 = ptr::read_volatile(&v1);
        let r2 = ptr::read_volatile(&v2);

        (r1, r2)
    };

    println!("Result: {r1}, {r2}");
}

fn test_threads() -> Result<(), io::Error> {
    let mut children = vec![];

    println!("Rust main thread: {:?}", thread::current());

    TLS.with(|tls| {
        println!("Main TLS before change: {}", *tls.borrow());
    });

    TLS.with(|tls| *tls.borrow_mut() = 42);

    TLS.with(|tls| {
        println!("Main TLS after change: {}", *tls.borrow());
    });

    for i in 0..5 {
        // Spin up another thread
        children.push(thread::spawn(move || {
            println!("This is thread number {}, {:?}", i, thread::current());

            TLS.with(|tls| *tls.borrow_mut() = i);

            TLS.with(|tls| {
                println!("Inner TLS: {}", *tls.borrow());
            });
        }));
    }

    println!("About to join the threads.");

    for child in children {
        // Wait for the thread to finish. Returns a result.
        let _ = child.join();
    }

    TLS.with(|tls| {
        println!("Main TLS after threads: {}", *tls.borrow());
    });

    thread::sleep(Duration::from_secs(2));

    println!("Joins were successful.");

    Ok(())
}

#[cfg(not(esp_idf_version = "4.3"))]
fn test_fs() -> Result<(), io::Error> {
    use std::{fs, path::PathBuf};

    assert_eq!(fs::canonicalize(PathBuf::from("."))?, PathBuf::from("/"));
    assert_eq!(
        fs::canonicalize(
            PathBuf::from("/")
                .join("foo")
                .join("bar")
                .join(".")
                .join("..")
                .join("baz")
        )?,
        PathBuf::from("/foo/baz")
    );

    Ok(())
}
