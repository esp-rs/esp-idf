use std::iter::once;
use std::path::PathBuf;

use anyhow::*;
use common::*;
use embuild::bindgen::types::callbacks::{IntKind, ParseCallbacks};
use embuild::bindgen::BindgenExt;
use embuild::utils::OsStrExt;
use embuild::{bindgen as bindgen_utils, build, cargo, kconfig, path_buf};

mod common;
mod config;
mod libc_checks;
mod libc_items;

// Features `native` and `pio` control whether the build is performed using the "native" ESP IDF CMake-based build,
// or via the PlatformIO `espressif32` module. They work as follows:
// - If neither the `native` nor the `pio` feature is specified, native build would be used
// - If boththe  `native` and `pio` features are specified, native build would be used as well
// - Otherwise, either native or PlatformIO build would be used, depending on which feature is specified
//
// The sole reason why the `native` feature exists in the first place is so that if somebody uses `cargo check --all-features`
// (might happen due to VSCode Rust Analyzer default settings) native build to still be used in that case.
#[cfg(any(feature = "native", not(feature = "pio")))]
mod native;
#[cfg(all(not(feature = "native"), feature = "pio"))]
mod pio;

#[cfg(any(feature = "native", not(feature = "pio")))]
use native as build_driver;
#[cfg(all(not(feature = "native"), feature = "pio"))]
use pio as build_driver;

#[derive(Debug)]
struct BindgenCallbacks;

impl ParseCallbacks for BindgenCallbacks {
    fn int_macro(&self, name: &str, _value: i64) -> Option<IntKind> {
        // Make sure the ESP_ERR_*, ESP_OK and ESP_FAIL macros are all i32.
        const PREFIX: &str = "ESP_";
        const SUFFIX: &str = "ERR_";
        const SUFFIX_SPECIAL: [&str; 2] = ["OK", "FAIL"];

        let name = name.strip_prefix(PREFIX)?;
        if name.starts_with(SUFFIX) || SUFFIX_SPECIAL.contains(&name) {
            Some(IntKind::I32)
        } else {
            None
        }
    }
}

fn main() -> anyhow::Result<()> {
    let build_output = build_driver::build()?;

    // We need to restrict the kconfig parameters which are turned into rustc cfg items
    // because otherwise we would be hitting rustc command line restrictions on Windows
    //
    // For now, we take all tristate parameters which are set to true, as well as a few
    // selected string / numeric ones, as per below
    //
    // Numeric kconfig values (e.g. `BT_NIMBLE_L2CAP_COC_MAX_NUM`) are emitted as value cfgs
    // (`esp_idf_bt_nimble_l2cap_coc_max_num="<n>"`), so downstream can match on them with e.g.
    // `#[cfg(not(esp_idf_bt_nimble_l2cap_coc_max_num = "0"))]` (the same technique used for the
    // ESP-IDF version cfgs, since `cfg` only supports `=` equality).
    //
    // This might change in future
    let kconfig_str_allow = regex::Regex::new(r"IDF_TARGET|BT_NIMBLE_L2CAP_COC_MAX_NUM")?;

    let cfg_args = build::CfgArgs {
        args: build_output
            .kconfig_args
            .filter(|(key, value)| {
                matches!(value, kconfig::Value::Tristate(kconfig::Tristate::True))
                    || kconfig_str_allow.is_match(key)
            })
            .filter_map(|(key, value)| value.to_rustc_cfg("esp_idf", key))
            .collect(),
    };

    let mcu = cfg_args
        .get("esp_idf_idf_target")
        .ok_or_else(|| {
            anyhow!(
                "Failed to get IDF_TARGET from kconfig. cfgs:\n{:?}",
                cfg_args.args
            )
        })?
        .to_lowercase();

    // We need the IDF version to configure bindgen blocklist, but normally
    // the version is parsed from the bindgen themselves, so extract it from
    // the headers manually here.
    // For now, only major version is needed.
    let idf_version_header = path_buf![
        &build_output.esp_idf,
        "components",
        "esp_common",
        "include",
        "esp_idf_version.h"
    ];
    let idf_version_major: u32 = std::fs::read_to_string(&idf_version_header)
        .ok()
        .and_then(|s| {
            regex::Regex::new(r"#define\s+ESP_IDF_VERSION_MAJOR\s+(\d+)")
                .ok()?
                .captures(&s)?
                .get(1)?
                .as_str()
                .parse()
                .ok()
        })
        .ok_or_else(|| {
            anyhow!(
                "Failed to parse ESP_IDF_VERSION_MAJOR from '{}'",
                idf_version_header.display()
            )
        })?;

    let manifest_dir = manifest_dir()?;

    let header_file = path_buf![
        &manifest_dir,
        "src",
        "include",
        if mcu == "esp8266" {
            "esp-8266-rtos-sdk"
        } else {
            "esp-idf"
        },
        "bindings.h"
    ];

    cargo::track_file(&header_file);

    // CONFIG_LIBC_PICOLIBC=y is normally only supported with GCC toolchain. This is a problem
    // as we use clang/bindgen to generate the bindings (even if a GCC toolchain is selected).
    // clang/bindgen doesn't understand GCC's -specs=picolibc.specs and falls back to the
    // newlib sysroot headers, causing errors like 'unknown type name __FILE'.
    // Detect the picolibc include dir (relative to the GCC sysroot) and inject it via the
    // Factory's clang args so it is searched before the sysroot -I path added by embuild.
    // gcc_sysroot is e.g. <toolchain>/riscv32-esp-elf/riscv32-esp-elf/;
    // picolibc is at <toolchain>/riscv32-esp-elf/picolibc/include (one level up from sysroot).
    let picolibc_include: Option<PathBuf> = if cfg_args.get("esp_idf_libc_picolibc").is_some() {
        let sysroot = build_output
            .gcc_sysroot
            .as_deref()
            .ok_or_else(|| anyhow!("CONFIG_LIBC_PICOLIBC=y but GCC sysroot could not be found"))?;
        let picolibc = sysroot.parent().unwrap().join("picolibc").join("include");
        if !picolibc.exists() {
            bail!("picolibc include dir not found at '{}'", picolibc.display());
        }
        Some(picolibc)
    } else {
        None
    };

    // Because we have multiple bindgen invocations and we can't clone a bindgen::Builder,
    // we have to set the options every time.
    let configure_bindgen = |bindgen: embuild::bindgen::types::Builder| {
        let bindgen = bindgen
            .parse_callbacks(Box::new(BindgenCallbacks))
            .use_core()
            .enable_function_attribute_detection()
            .clang_arg("-DESP_PLATFORM")
            .blocklist_function("strtold")
            .blocklist_function("_strtold_r")
            .blocklist_function("v.*printf")
            .blocklist_function("v.*scanf")
            .blocklist_function("_v.*printf_r")
            .blocklist_function("_v.*scanf_r")
            .blocklist_function("esp_log_writev")
            .blocklist_type("cdc_desc_func_telephone_call_state_reporting_capabilities_t");
        // In ESP-IDF < v6.0, pcnt_unit_t exists as both a struct and an enum; blocklist the
        // type so we can provide the enum definition manually in src/pcnt.rs. In v6.0+ the
        // legacy enum is gone, so bindgen can handle the struct fine on its own.
        let bindgen = if idf_version_major < 6 {
            bindgen.blocklist_type("pcnt_unit_t")
        } else {
            bindgen
        };
        // If picolibc is active, inject its include path before the sysroot headers so
        // bindgen picks up the right stdlib headers (clang ignores -specs=picolibc.specs).
        let bindgen = if let Some(ref picolibc) = picolibc_include {
            bindgen.clang_arg(format!("-I{}", picolibc.display()))
        } else {
            bindgen
        };
        let bindgen = bindgen
            .clang_args(build_output.components.clang_args())
            .clang_args(vec![
                "-target",
                if mcu != "esp32" && mcu != "esp32s2" && mcu != "esp32s3" {
                    // Necessary to pass explicitly, because of https://github.com/rust-lang/rust-bindgen/issues/1555
                    "riscv32"
                } else {
                    // We don't really have a similar issue with Xtensa, but we pass it explicitly as well just in case
                    "xtensa"
                },
            ]);
        Ok(bindgen)
    };

    let bindings_file = bindgen_utils::default_bindings_file()?;
    let bindgen_err = || {
        anyhow!(
            "failed to generate bindings in file '{}'",
            bindings_file.display()
        )
    };

    #[allow(unused_mut)]
    let mut headers = vec![header_file];

    #[cfg(any(feature = "native", not(feature = "pio")))]
    // Add additional headers from extra components.
    headers.extend(
        build_output
            .config
            .native
            .combined_bindings_headers()?
            .into_iter()
            .inspect(|h| cargo::track_file(h)),
    );

    configure_bindgen(build_output.bindgen.clone().builder()?)?
        .path_headers(headers)?
        .generate()
        .with_context(bindgen_err)?
        .write_to_file(&bindings_file)
        .with_context(bindgen_err)?;

    // Generate bindings separately for each unique module name.
    #[cfg(any(feature = "native", not(feature = "pio")))]
    (|| {
        use std::fs;
        use std::io::{BufWriter, Write};

        let mut output_file =
            BufWriter::new(fs::File::options().append(true).open(&bindings_file)?);

        for (module_name, headers) in build_output.config.native.module_bindings_headers()? {
            let bindings = configure_bindgen(build_output.bindgen.clone().builder()?)?
                .path_headers(headers.into_iter().inspect(|h| cargo::track_file(h)))?
                .generate()?;

            writeln!(
                &mut output_file,
                "pub mod {module_name} {{\
                     {bindings}\
                 }}"
            )?;
        }
        Ok(())
    })()
    .with_context(bindgen_err)?;

    // Cargo fmt generated bindings.
    libc_checks::format_bindings(&bindings_file)?;

    // The `libc` compatibility checks of `src/checks/libc.rs`
    libc_checks::generate(
        &bindings_file,
        &bindings_file.with_file_name("libc_checks.rs"),
    )?;

    let cfg_args = build::CfgArgs {
        args: cfg_args
            .args
            .into_iter()
            .chain(EspIdfVersion::parse(bindings_file)?.cfg_args())
            .chain(build_output.components.cfg_args())
            .chain(once(mcu))
            .collect(),
    };
    cfg_args.propagate();
    cfg_args.output();

    // In case other crates need to have access to the ESP-IDF C headers
    build_output.cincl_args.propagate();

    // In case other crates need to have access to the ESP-IDF toolchains
    if let Some(env_path) = build_output.env_path {
        cargo::set_metadata(embuild::build::ENV_PATH_VAR, env_path);
    }

    // In case other crates need access to the ESP-IDF SDK
    cargo::set_metadata(
        embuild::build::ESP_IDF_PATH_VAR,
        build_output.esp_idf.try_to_str()?,
    );

    if let Some(link_args) = build_output.link_args {
        link_args.propagate();

        // Only necessary for building the examples
        link_args.output();
    }

    Ok(())
}
