# Rust on ESP-IDF

[![Matrix](https://img.shields.io/matrix/esp-rs:matrix.org?label=join%20matrix&color=BEC5C9&logo=matrix)](https://matrix.to/#/#esp-rs:matrix.org)

The Rust crates that build on top of Espressif's [ESP-IDF](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/) framework, developed together in this repository:

| Crate | Description | CI |
|-------|-------------|----|
| [`esp-idf-sys`](esp-idf-sys) [![crates.io](https://img.shields.io/crates/v/esp-idf-sys.svg)](https://crates.io/crates/esp-idf-sys) | Raw Rust bindings for ESP-IDF, plus the `cargo`-driven build of ESP-IDF itself | [![CI](https://github.com/esp-rs/esp-idf/actions/workflows/ci.yml/badge.svg)](https://github.com/esp-rs/esp-idf/actions/workflows/ci.yml) |
| [`esp-idf-hal`](esp-idf-hal) [![crates.io](https://img.shields.io/crates/v/esp-idf-hal.svg)](https://crates.io/crates/esp-idf-hal) | Safe Rust wrappers for the ESP-IDF drivers, implementing the [`embedded-hal`](https://github.com/rust-embedded/embedded-hal) traits | [![CI](https://github.com/esp-rs/esp-idf/actions/workflows/ci.yml/badge.svg)](https://github.com/esp-rs/esp-idf/actions/workflows/ci.yml) |
| [`esp-idf-svc`](esp-idf-svc) [![crates.io](https://img.shields.io/crates/v/esp-idf-svc.svg)](https://crates.io/crates/esp-idf-svc) | Safe Rust wrappers for the ESP-IDF services, implementing the [`embedded-svc`](https://github.com/esp-rs/embedded-svc) traits | [![CI](https://github.com/esp-rs/esp-idf/actions/workflows/ci.yml/badge.svg)](https://github.com/esp-rs/esp-idf/actions/workflows/ci.yml) |

The crates form a single Cargo workspace. Each crate keeps its own `README.md` and `CHANGELOG.md` in its directory, while the build configuration (`.cargo/config.toml`, `sdkconfig.defaults`, the partition table) and the [examples](examples/src/bin) of all crates (in the `examples` crate) are shared at the repository root. Build and run the examples from the repository root:

```sh
MCU=esp32c3 cargo espflash flash --target riscv32imc-esp-espidf --bin wifi --monitor
```

## Getting started

- The [esp-idf-template](https://github.com/esp-rs/esp-idf-template) project template
- The `README.md` of each crate for its features and ESP-IDF version support
- The [esp-rs Matrix channel](https://matrix.to/#/#esp-rs:matrix.org) for questions

## Community Effort

Please note that **all `esp-idf-*` crates are a community effort**, in that Espressif puts little to no paid developer time in these.
So while ESP-IDF itself is very popular and well tested, the `esp-idf-*` crates:
- Might be a bit lagging behind the latest stable ESP-IDF version
- Are (currently) missing HIL tests
- Need more documentation

For a HAL which is officially supported by Espressif (as in - with paid developer time), please look at [`esp-hal`](https://github.com/esp-rs/esp-hal). Keep in mind that `esp-hal` is `no_std`-only, does not use ESP-IDF and requires async programming.

## Repository layout and history

This repository continues the former [esp-idf-hal](https://github.com/esp-rs/esp-idf-hal) repository, whose history is unchanged: for `esp-idf-hal` files, `git log --follow` walks past the commit that moved them into the `esp-idf-hal/` directory.

The histories of the former [esp-idf-sys](https://github.com/esp-rs/esp-idf-sys) and [esp-idf-svc](https://github.com/esp-rs/esp-idf-svc) repositories were merged in with their paths rewritten under `esp-idf-sys/` and `esp-idf-svc/`, so plain `git log` works for their files all the way back.

Release tags are prefixed with the crate name: `esp-idf-sys-v0.38.1`, `esp-idf-hal-v0.47.0`, `esp-idf-svc-v0.53.0`. The pre-monorepo `esp-idf-hal` tags keep their unprefixed `vX.Y.Z` names as well.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
