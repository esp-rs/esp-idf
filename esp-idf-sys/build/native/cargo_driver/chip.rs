//! ESP32 chip variants support.

use anyhow::{bail, Result};
use strum::{Display, EnumIter, EnumString};

use embuild::espidf::EspIdfVersion;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Display, EnumString, EnumIter)]
#[repr(u32)]
pub enum Chip {
    /// Xtensa LX6 based dual core
    #[strum(serialize = "esp32")]
    ESP32 = 0,
    /// Xtensa LX7 based single core
    #[strum(serialize = "esp32s2")]
    ESP32S2,
    /// Xtensa LX7 based dual core
    #[strum(serialize = "esp32s3")]
    ESP32S3,
    /// RISC-V based single core
    #[strum(serialize = "esp32c2")]
    ESP32C2,
    /// RISC-V based single core
    #[strum(serialize = "esp32c3")]
    ESP32C3,
    /// RISC-V based single core with atomics support
    #[strum(serialize = "esp32h2")]
    ESP32H2,
    /// RISC-V based single core with atomics support
    #[strum(serialize = "esp32c5")]
    ESP32C5,
    /// RISC-V based single core with atomics support
    #[strum(serialize = "esp32c6")]
    ESP32C6,
    /// RISC-V based single core with atomics support
    #[strum(serialize = "esp32c61")]
    ESP32C61,
    /// RISC-V based dual core
    #[strum(serialize = "esp32p4")]
    ESP32P4,
    /// RISC-V based dual core
    #[strum(serialize = "esp32s31")]
    ESP32S31,
}

impl Chip {
    pub fn detect(rust_target_triple: &str) -> Result<&[Chip]> {
        let chips: &[Chip] = match rust_target_triple {
            "xtensa-esp32-espidf" => &[Chip::ESP32],
            "xtensa-esp32s2-espidf" => &[Chip::ESP32S2],
            "xtensa-esp32s3-espidf" => &[Chip::ESP32S3],
            // Keep C3 as the first in the list, so it is picked up by default; as C2 does not work for older ESP IDFs
            "riscv32imc-esp-espidf" => &[Chip::ESP32C3, Chip::ESP32C2],
            // Keep C6 at the first in the list, so it is picked up by default; as H2 does not have a Wifi
            "riscv32imac-esp-espidf" => {
                &[Chip::ESP32C6, Chip::ESP32C61, Chip::ESP32C5, Chip::ESP32H2]
            }
            "riscv32imafc-esp-espidf" => &[Chip::ESP32P4, Chip::ESP32S31],
            _ => bail!("Unsupported target '{}'", rust_target_triple),
        };

        Ok(chips)
    }

    pub fn is_xtensa(&self) -> bool {
        matches!(self, Self::ESP32 | Self::ESP32S2 | Self::ESP32S3)
    }

    /// The name of the gcc toolchain (to compile the `esp-idf`) for `idf_tools.py`.
    pub fn gcc_toolchain(&self, version: Option<&EspIdfVersion>) -> &'static str {
        let new = version
            .map(|version| version.major > 5 || version.major == 5 && version.minor > 1)
            .unwrap_or(true);

        match self {
            Self::ESP32 => {
                if new {
                    "xtensa-esp-elf"
                } else {
                    "xtensa-esp32-elf"
                }
            }
            Self::ESP32S2 => {
                if new {
                    "xtensa-esp-elf"
                } else {
                    "xtensa-esp32s2-elf"
                }
            }
            Self::ESP32S3 => {
                if new {
                    "xtensa-esp-elf"
                } else {
                    "xtensa-esp32s3-elf"
                }
            }
            Self::ESP32C2
            | Self::ESP32C3
            | Self::ESP32H2
            | Self::ESP32C5
            | Self::ESP32C6
            | Self::ESP32C61
            | Self::ESP32P4
            | Self::ESP32S31 => "riscv32-esp-elf",
        }
    }

    /// The name of the clang toolchain for `idf_tools.py`.
    ///
    /// Used for generating the `esp-idf-sys` bindings with `bindgen`
    pub fn clang_toolchain(&self, version: Option<&EspIdfVersion>) -> &'static str {
        let new = version
            .map(|version| version.major > 5 || version.major == 5 && version.minor > 0)
            .unwrap_or(true);

        if new {
            "esp-clang"
        } else {
            // ESP-IDF < 5.1.0 used to have the clang toolchain named `xtensa-clang` even if
            // it actually is still a cross-toolchain
            "xtensa-clang"
        }
    }

    /// The name of the ESP ROM ELF files "toolchain" for `idf_tools.py`.
    ///
    /// Used by recent ESP IDFs during the build process
    pub fn esp_rom_elfs(&self, version: Option<&EspIdfVersion>) -> Option<&'static str> {
        let exists = version
            .map(|version| version.major > 5 || version.major == 5 && version.minor > 0)
            .unwrap_or(true);

        exists.then_some("esp-rom-elfs")
    }

    /// The name of the gcc toolchain for the ultra low-power co-processor for
    /// `idf_tools.py`.
    pub fn ulp_gcc_toolchain(&self, version: Option<&EspIdfVersion>) -> Option<&'static str> {
        match self {
            Self::ESP32 => Some("esp32ulp-elf"),
            Self::ESP32S2 | Self::ESP32S3 | Self::ESP32C5 | Self::ESP32C6 | Self::ESP32P4 => Some(
                if version
                    .map(|version| {
                        version.major > 4
                            || version.major == 4 && version.minor > 4
                            || version.major == 4 && version.minor == 4 && version.patch >= 2
                    })
                    .unwrap_or(true)
                {
                    "esp32ulp-elf"
                } else {
                    "esp32s2ulp-elf"
                },
            ),
            _ => None,
        }
    }

    pub fn cmake_toolchain_file(self) -> String {
        format!("toolchain-{self}.cmake")
    }
}
