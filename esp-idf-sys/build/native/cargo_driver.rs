use std::convert::TryFrom;
use std::ffi::OsStr;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{env, fs};

use anyhow::{anyhow, bail, Context, Error, Result};
use config::{ESP_IDF_REPOSITORY_VAR, ESP_IDF_VERSION_VAR};
use embuild::cargo::IntoWarning;
use embuild::cmake::file_api::codemodel::Language;
use embuild::cmake::file_api::ObjKind;
use embuild::espidf::{
    EspIdfOrigin, EspIdfRemote, EspIdfVersion, FromEnvError, NotActivatedError, SourceTree,
};
use embuild::fs::copy_file_if_different;
use embuild::utils::{OsStrExt, PathExt};
use embuild::{bindgen, build, cargo, cmake, espidf, git, kconfig, path_buf};

use strum::IntoEnumIterator;

use self::chip::Chip;
use crate::common::{
    self, list_specific_sdkconfigs, manifest_dir, sanitize_c_env_vars, sanitize_project_path,
    setup_clang_env, workspace_dir, EspIdfBuildOutput, EspIdfComponents, InstallDir,
};
use crate::config::{BuildConfig, ESP_IDF_GLOB_VAR_PREFIX, ESP_IDF_TOOLS_INSTALL_DIR_VAR};

pub mod chip;
pub mod config;

pub fn build() -> Result<EspIdfBuildOutput> {
    sanitize_project_path()?;
    sanitize_c_env_vars()?;

    let out_dir = cargo::out_dir();
    let target = env::var("TARGET")?;
    let workspace_dir = workspace_dir()?;
    let manifest_dir = manifest_dir()?;

    let config = BuildConfig::try_from_env().map(|mut config| {
        config.with_cargo_metadata().into_warning();
        config
    })?;
    config.print();

    let supported_chips = Chip::detect(&target)?;

    let chip = if let Some(mcu) = &config.mcu {
        if let Ok(chip) = Chip::from_str(mcu) {
            if !supported_chips.contains(&chip) {
                bail!(
                    "Specified MCU '{chip}' is not amongst the MCUs ([{}]) supported by the build target ('{target}')",
                    supported_chips.iter().map(|chip| format!("{chip}")).collect::<Vec<_>>().join(", ")
                );
            }

            chip
        } else {
            bail!(
                "Specified MCU '{mcu}' is not recognized as a valid Espressif MCU amongst [{}]",
                Chip::iter()
                    .map(|chip| chip.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    } else {
        if supported_chips.len() > 1 {
            println!(
                "cargo:warning=Configuring first supported MCU '{}' derived from the build target '{}' supporting MCUs [{}]; explicitly specify an MCU to resolve this ambiguity",
                supported_chips[0],
                target,
                supported_chips.iter().map(|chip| format!("{chip}")).collect::<Vec<_>>().join(", ")
            );
        }

        supported_chips[0]
    };

    let chip_name = chip.to_string();
    let profile = common::build_profile();
    let cmake_generator = config.native.esp_idf_cmake_generator();

    // A closure to specify which tools `idf-tools.py` should install.
    let make_tools = move |tree: &SourceTree,
                           version: &Result<espidf::EspIdfVersion>|
          -> Result<Vec<espidf::Tools>> {
        eprintln!(
            "Using esp-idf {} at '{}'",
            espidf::EspIdfVersion::format(version),
            tree.path().display(),
        );

        let mut tools = vec![];
        let mut subtools = vec![
            chip.gcc_toolchain(version.as_ref().ok()),
            chip.clang_toolchain(version.as_ref().ok()),
        ];

        subtools.extend(chip.esp_rom_elfs(version.as_ref().ok()));

        // Use custom cmake for esp-idf<4.4, because we need at least cmake-3.20
        match version.as_ref().map(|v| (v.major, v.minor, v.patch)) {
            Ok((major, minor, _)) if major > 4 || (major == 4 && minor >= 4) => {
                subtools.push("cmake")
            }
            _ => {
                tools.push(espidf::Tools::cmake()?);
            }
        }

        if cmake_generator == cmake::Generator::Ninja {
            subtools.push("ninja")
        }

        if !cfg!(target_os = "linux") || !cfg!(target_arch = "aarch64") {
            subtools.extend(chip.ulp_gcc_toolchain(version.as_ref().ok()));
        }

        tools.push(espidf::Tools::new(subtools));

        Ok(tools)
    };

    // The `kconfig.cmake` script looks at this variable if it should compile `mconf` on
    // windows. `idf_tools.py` also fails when this variable is set because it thinks
    // we're using msys/cygwin.
    // But this variable is also present when using git-bash.
    env::remove_var("MSYSTEM");
    // Install the esp-idf and its tools.
    let (idf, tools_install_dir) = {
        // Get the install dir location from the build config, or use
        // [`crate::config::DEFAULT_TOOLS_INSTALL_DIR`] if unset.
        let (idf_tools_install_dir, is_default_install_dir) = config.esp_idf_tools_install_dir()?;
        // EspIdf must come from the environment if `esp_idf_tools_install_dir` == `fromenv`.
        let require_from_env = idf_tools_install_dir.is_from_env();
        let maybe_from_env = require_from_env || is_default_install_dir;

        // Closure to install the esp-idf using `embuild::espidf::Installer`.
        let install = |esp_idf_origin: EspIdfOrigin| -> Result<(espidf::EspIdf, InstallDir)> {
            match &esp_idf_origin {
                EspIdfOrigin::Custom(repo) => {
                    eprintln!(
                        "Using custom user-supplied esp-idf repository at '{}' (detected from env variable `{}`)",
                        repo.path().display(),
                        espidf::IDF_PATH_VAR
                    );
                    if let Some(custom_url) = &config.native.esp_idf_repository {
                        cargo::print_warning(format_args!(
                            "Ignoring configuration setting `{ESP_IDF_REPOSITORY_VAR}=\"{custom_url}\"`: \
                             custom esp-idf repository detected via ${}",
                            espidf::IDF_PATH_VAR
                        ));
                    }
                    if let Some(custom_version) = &config.native.esp_idf_version {
                        cargo::print_warning(format_args!(
                            "Ignoring configuration setting `{ESP_IDF_VERSION_VAR}` ({custom_version}): \
                             custom esp-idf repository detected via ${}",
                            espidf::IDF_PATH_VAR
                        ));
                    }
                }
                EspIdfOrigin::Managed(remote) => {
                    eprintln!("Using managed esp-idf repository: {remote:?}");
                }
            };

            let idf = espidf::Installer::new(esp_idf_origin)
                .install_dir(idf_tools_install_dir.path().map(Into::into))
                .with_tools(make_tools)
                .install()
                .context("Could not install esp-idf")?;
            Ok((idf, idf_tools_install_dir.clone()))
        };

        // TODO: This is a bit of a mess.
        // We should probably refactor this and the lines below to make it more readable.
        //
        // For one, it is still unclear to me when this path should be used and when not.
        // It seems an option is to also completely retire specifying the IDF PATH in the cargo-metadata config,
        // see: https://github.com/esp-rs/esp-idf-sys/pull/353#issuecomment-2543179482
        let idf_path = config.native.idf_path.as_deref();

        // 1. Try to use the activated esp-idf environment if `esp_idf_tools_install_dir`
        //    is `fromenv` or unset.
        // 2. Use a custom esp-idf repository specified by `$IDF_PATH`/`idf_path` if
        //    available and install the tools using `embuild::espidf::Installer` in
        //    `install_dir`.
        // 3. Install the esp-idf and its tools in `install_dir`.
        match (espidf::EspIdf::try_from_env(), maybe_from_env) {
            (Ok(idf), true) => {
                eprintln!(
                    "Using activated esp-idf {} environment at '{}'",
                    espidf::EspIdfVersion::format(&idf.version),
                    idf.esp_idf_dir.path().display()
                );

                (idf, InstallDir::FromEnv)
            },
            (Ok(idf), false) => {
                cargo::print_warning(format_args!(
                    "Ignoring activated esp-idf environment: {ESP_IDF_TOOLS_INSTALL_DIR_VAR} != {}", InstallDir::FromEnv
                ));
                install(EspIdfOrigin::Custom(idf.esp_idf_dir))?
            },
            (Err(FromEnvError::NotActivated(NotActivatedError { source: err, .. })), true) |
            (Err(FromEnvError::NoRepo(err)), true) if require_from_env => {
                return Err(err.context(
                    format!("activated esp-idf environment not found but required by {ESP_IDF_TOOLS_INSTALL_DIR_VAR} == {idf_tools_install_dir}")
                ))
            }
            (Err(FromEnvError::NotActivated(NotActivatedError { esp_idf_dir, .. })), _) => {
                install(EspIdfOrigin::Custom(esp_idf_dir))?
            },
            (Err(FromEnvError::NoRepo(_)), _) => {
                let origin = match idf_path {
                    Some(idf_path) => EspIdfOrigin::Custom(SourceTree::Plain(idf_path.to_path_buf())),
                    None => EspIdfOrigin::Managed(EspIdfRemote {
                        git_ref: config.native.esp_idf_version(),
                        repo_url: config.native.esp_idf_repository.clone()
                    })
                };
                install(origin)?
            },
        }
    };

    let gcc12 = match idf.version.as_ref().map(|v| (v.major, v.minor, v.patch)) {
        Ok((major, minor, _)) => major > 5 || major == 5 && minor >= 1,
        Err(err) => {
            cargo::print_warning(err);
            false
        }
    };

    let version = idf.version.as_ref().ok().cloned();

    let custom_linker = if !gcc12 && !chip.is_xtensa() {
        // An annoying issue with the riscv targets is that since Rust nightly-2023-08-08
        // and the introduction of LLVM-17, rustc (and LLVM) claim to support RISCV ISA 2.1 spec
        // (via the "attributes" section in elf object files)
        //
        // However, older versions of GCC like these prior to ESP-IDF 5.1 only support the earlier ISA 2.0 spec
        // What is happening is that they do error out - at link time - when they link together object files
        // generated with ISA 2.0 (ESP IDF < 5.1) with ones generated with ISA 2.1 (rustc) - even though that should be OK
        // - at least for riscv32imc and riscv32imac targets - assuming they both support the zicsr and zifencei extensions
        // as is actually the case with the ESP32 riscv MCUs
        //
        // For now, the only feasible workaround is to use a newer GCC specifically for linking
        // (Another workaround would've been to strip the riscv arch attributes either from the ESP IDF object files,
        // or from the object files generated by rustc, but I've yet to find a way to do this reliably withoput breaking the build process)
        let linker_origin = EspIdfOrigin::Managed(EspIdfRemote {
            git_ref: git::Ref::Tag("v5.1".into()),
            repo_url: None, // Use the default remote repository
        });

        // Get the install dir location from the build config, or use
        // [`crate::config::DEFAULT_TOOLS_INSTALL_DIR`] if unset.
        let (linker_install_dir, _) = config.esp_idf_tools_install_dir()?;

        let version_for_installer = Some(EspIdfVersion {
            major: 5,
            minor: 1,
            patch: 0,
        });

        let installer = espidf::Installer::new(linker_origin)
            .install_dir(linker_install_dir.path().map(Into::into))
            .with_tools(move |_, _| {
                Ok(vec![espidf::Tools::new(vec![
                    chip.gcc_toolchain(version_for_installer.as_ref())
                ])])
            })
            .install()
            .context("Could not install GCC linker")?;

        let linker_name = format!("{}-gcc", chip.gcc_toolchain(version.as_ref()));

        let linker =
            which::which_in_global(linker_name.clone(), Some(installer.exported_path.clone()))?
                .next()
                .ok_or_else(|| {
                    Error::msg(format!(
                        "Could not locate GCC linker {} in {:?}",
                        linker_name, installer.exported_path
                    ))
                })?;

        Some(linker)
    } else {
        None
    };

    let (default_branch, curr_branch) = if let SourceTree::Git(repository) = &idf.esp_idf_dir {
        let default_branch = repository.get_default_branch()?;
        let curr_branch = repository.get_branch_name()?;

        (default_branch, curr_branch)
    } else {
        (None, None)
    };

    // Issue warnings when building against an ESP-IDF branch/tag which does not designate an official ESP-IDF release
    if let Some(curr_branch) = curr_branch {
        if default_branch.as_ref() == Some(&curr_branch) {
            cargo::print_warning("Building against ESP-IDF `master` is not officially supported, because it is considered bleeding edge and might get backwards-incompatible changes.");
        } else if curr_branch.starts_with("release/v") {
            cargo::print_warning(format_args!(
                "Building against ESP-IDF branch `{curr_branch}` is not officially supported, because `release/vX.Y.Z` branches do not have a stable ESP-IDF version.",
            ));
        } else if !curr_branch.starts_with("v") {
            cargo::print_warning(format_args!(
                "Building against ESP-IDF branch `{curr_branch}` is not officially supported, because it does not follow the `vX.Y.Z` ESP-IDF release tag naming convention.",
            ));
        }
    }

    // Issue warnings when building against a deprecated ESP-IDF version
    if let Ok((major, minor, patch)) = idf.version.as_ref().map(|v| (v.major, v.minor, v.patch)) {
        if major < 5 || major == 5 && minor < 3 {
            cargo::print_warning(format_args!(
                "Building against ESP-IDF version {major}.{minor}.{patch} is deprecated and not officially supported. \
                 Please consider upgrading to ESP-IDF V5.3.0 or newer, because support for older ESP-IDF versions will be removed in newer releases."
            ));
        }

        if chip == Chip::ESP32S31 && (major < 6 || major == 6 && minor < 1) {
            bail!(
                "MCU '{chip}' is only supported by ESP-IDF V6.1.0 or newer, but the build is against ESP-IDF version {major}.{minor}.{patch}"
            );
        }
    } else {
        cargo::print_warning(format_args!(
            "Could not extract ESP-IDF version from {:?}",
            idf.version
        ));
    }

    // "PATH" is anyway passed to the CMake generator, but if we don't set it here, we get the following warnings from CMake:
    // ```
    // Compiler family detection failed due to error: ToolNotFound: Failed to find tool. Is `riscv32-esp-elf-gcc` installed?
    // Compiler family detection failed due to error: ToolNotFound: Failed to find tool. Is `riscv32-esp-elf-g++` installed?
    // Compiler family detection failed due to error: ToolNotFound: Failed to find tool. Is `riscv32-esp-elf-gcc` installed?
    // ```
    env::set_var("PATH", &idf.exported_path);

    // Remove the sdkconfig file generated by the esp-idf so that potential changes
    // in the user provided sdkconfig and sdkconfig.defaults don't get ignored.
    // TODO: I'm really not sure why we have to do this.
    let _ = fs::remove_file(path_buf![&out_dir, "sdkconfig"]);

    // Create cmake project.
    copy_file_if_different(
        manifest_dir.join(path_buf!("resources", "cmake_project", "CMakeLists.txt")),
        &out_dir,
    )?;
    let main_comp = out_dir.join("main");
    fs::create_dir_all(&main_comp)?;
    copy_file_if_different(
        manifest_dir.join(path_buf!("resources", "cmake_project", "main", "main.c")),
        &main_comp,
    )?;
    copy_file_if_different(
        manifest_dir.join(path_buf!(
            "resources",
            "cmake_project",
            "main",
            "CMakeLists.txt"
        )),
        &main_comp,
    )?;

    // Generate the `idf_component.yml` for the main component if there is at least one
    // remote component.
    let idf_comp_yml = main_comp.join("idf_component.yml");
    let idf_comp_yml_contents = config.native.generate_idf_component_yml();
    if let Some(idf_comp_yml_contents) = idf_comp_yml_contents {
        // Only write it when the generated contents differ or it doesn't exist.
        match fs::read_to_string(&idf_comp_yml) {
            Ok(file_contents) if file_contents == idf_comp_yml_contents => (),
            Ok(_) | Err(_) => {
                fs::write(&idf_comp_yml, idf_comp_yml_contents).with_context(|| {
                    anyhow!("could not write file '{}'", idf_comp_yml.display())
                })?;
            }
        }
    } else {
        let _ = fs::remove_file(&idf_comp_yml);
    }
    let idf_comp_manager = config.native.idf_component_manager();

    // Copy additional globbed files specified by user env variables
    for file in build::tracked_env_globs_iter(ESP_IDF_GLOB_VAR_PREFIX)? {
        let dest_path = out_dir.join(file.1);
        fs::create_dir_all(dest_path.parent().unwrap())?;
        // TODO: Maybe warn if this overwrites a critical file (e.g. CMakeLists.txt).
        // It could be useful for the user to explicitly overwrite our files.
        copy_file_if_different(&file.0, &dest_path)?;
    }

    // Resolve the `sdkconfig` and all `sdkconfig.defaults` files specified in the build
    // config.
    let sdkconfig = {
        let file = config.esp_idf_sdkconfig();
        let path = Path::new(&file).abspath_relative_to(&workspace_dir);
        let cfg = list_specific_sdkconfigs(path, &profile, &chip_name).next();
        if let Some(ref file) = cfg {
            cargo::track_file(file);
        }
        cfg
    };

    let sdkconfig_defaults = {
        let gen_defaults_path = out_dir.join("gen-sdkconfig.defaults");
        fs::write(&gen_defaults_path, generate_sdkconfig_defaults()?)?;

        let mut result = vec![gen_defaults_path];
        result.extend(
            config
                .esp_idf_sdkconfig_defaults()
                .into_iter()
                .flat_map(|v| {
                    list_specific_sdkconfigs(
                        v.abspath_relative_to(&workspace_dir),
                        &profile,
                        &chip_name,
                    )
                    // We need to reverse the order here so that the more
                    // specific defaults come last.
                    .rev()
                    .inspect(|p| cargo::track_file(p))
                }),
        );
        result
    };

    let defaults_files = to_cmake_path_list(
        sdkconfig_defaults
            .iter()
            // Use the `sdkconfig` as a defaults file to prevent it from being changed by the
            // build. It must be the last defaults file so that its options have precendence
            // over any actual defaults from files before it.
            .chain(sdkconfig.as_ref()),
    )?;

    let cmake_toolchain_file = path_buf![
        &idf.esp_idf_dir.path(),
        "tools",
        "cmake",
        chip.cmake_toolchain_file()
    ];

    // Get the directories of all extra components to build.
    let extra_component_dirs = to_cmake_path_list(config.native.extra_component_dirs()?)?;

    // `cmake::Config` automatically uses `<out_dir>/build` and there is no way to query
    // what build directory it sets, so we hard-code it.
    let cmake_build_dir = out_dir.join("build");

    let query = cmake::Query::new(
        &cmake_build_dir,
        "cargo",
        &[ObjKind::Codemodel, ObjKind::Toolchains, ObjKind::Cache],
    )?;

    let mut cmake_config = cmake::Config::new(&out_dir);

    cmake_config
        .generator(cmake_generator.name())
        .out_dir(&out_dir)
        .no_build_target(true)
        .no_default_flags(true)
        .define("CMAKE_TOOLCHAIN_FILE", &cmake_toolchain_file)
        .define("CMAKE_BUILD_TYPE", "")
        .define("PYTHON", to_cmake_path_list([&idf.venv_python])?)
        .always_configure(true)
        .pic(false)
        .env("IDF_COMPONENT_MANAGER", idf_comp_manager)
        .env("EXTRA_COMPONENT_DIRS", extra_component_dirs)
        .env("IDF_PATH", idf.esp_idf_dir.path())
        .env("PATH", &idf.exported_path)
        .env("SDKCONFIG_DEFAULTS", defaults_files)
        .env("IDF_TARGET", &chip_name)
        .env("PROJECT_DIR", to_cmake_path_list([&workspace_dir])?);

    // Export all installed tools environment vars necessary for the ESP-IDF CMake build.
    for (var, value) in &idf.exported_env_vars {
        cmake_config.env(var, value);
    }

    match &tools_install_dir {
        InstallDir::Custom(dir) | InstallDir::Out(dir) | InstallDir::Workspace(dir) => {
            cmake_config.env(espidf::IDF_TOOLS_PATH_VAR, dir);
        }
        InstallDir::Global => {
            cmake_config.env(
                espidf::IDF_TOOLS_PATH_VAR,
                espidf::Installer::global_install_dir(),
            );
        }
        // Not setting it will forward the environment variable.
        InstallDir::FromEnv => (),
    }

    // Specify the components that should be built.
    if let Some(components) = &config.native.esp_idf_components {
        cmake_config.env("ESP_IDF_COMPONENTS", components.join(";"));
    }

    // Build the esp-idf.
    cmake_config.build();

    let replies = query.get_replies()?;
    let target = replies
        .get_codemodel()?
        .into_first_conf()
        .get_target("libespidf.elf")
        .unwrap_or_else(|| {
            bail!("Could not read build information from cmake: Target 'libespidf.elf' not found")
        })?;

    let compiler = replies
        .get_toolchains()
        .and_then(|mut t| {
            t.take(Language::C)
                .ok_or_else(|| Error::msg("No C toolchain"))
        })
        .and_then(|t| {
            t.compiler
                .path
                .ok_or_else(|| Error::msg("No compiler path set"))
        })
        .context("Could not determine the compiler from cmake")?;

    let build_info = espidf::EspIdfBuildInfo {
        esp_idf_dir: idf.esp_idf_dir.path().to_owned(),
        exported_path_var: idf.exported_path.try_to_str()?.to_owned(),
        venv_python: idf.venv_python,
        build_dir: cmake_build_dir.clone(),
        project_dir: out_dir.clone(),
        compiler: compiler.clone(),
        mcu: chip_name,
        sdkconfig,
        sdkconfig_defaults: Some(sdkconfig_defaults),
    };

    // Save information about the esp-idf build to the out dir so that it can be
    // easily retrieved by tools that need it.
    build_info.save_json(out_dir.join(espidf::BUILD_INFO_FILENAME))?;

    // Get all component names built by the esp-idf (cached by `CMakeLists.txt`).
    let components = replies
        .get_cache()?
        .entries
        .iter()
        .find(|e| e.name == "BUILD_COMPONENTS")
        .ok_or_else(|| Error::msg("could not get built esp-idf components from cmake"))?
        .value
        .split(';')
        .filter_map(|comp| match comp.trim() {
            "" => None,
            c => Some(c.to_string()),
        })
        .collect::<Vec<_>>();

    eprintln!("Built components: {}", components.join(", "));

    copy_binaries_to_target_folder()?;

    if let Some(clang) = which::which_in_global("clang", Some(idf.exported_path.clone()))?.next() {
        // Found a `clang` binary on the path
        // Check if its `lib` directory contains `libclang`
        //
        // Why is this check necessary? See:
        // https://github.com/espressif/llvm-project/issues/108

        let lib_path = clang
            .parent()
            .and_then(|clang| clang.parent())
            .map(|clang| clang.join("lib"));

        let has_libclang = lib_path
            .as_deref()
            .and_then(|lib_path| fs::read_dir(lib_path).ok())
            .map(|dir| {
                dir.filter_map(|entry| entry.ok())
                    .any(|entry| entry.file_name().to_string_lossy().starts_with("libclang."))
            })
            .unwrap_or(false);

        setup_clang_env(has_libclang.then_some(lib_path.as_deref().unwrap()))?;
    }

    let gcc_ld_name = format!("{}-ld", chip.gcc_toolchain(version.as_ref()));
    let gcc_sysroot = which::which_in_global(&gcc_ld_name, Some(idf.exported_path.clone()))
        .ok()
        .and_then(|mut it| it.next())
        .and_then(|ld| {
            embuild::cmd!(&ld, "--print-sysroot")
                .stdout()
                .ok()
                .map(|s| PathBuf::from(s.trim()))
        });

    let sdkconfig_json = path_buf![&cmake_build_dir, "config", "sdkconfig.json"];
    let build_output = EspIdfBuildOutput {
        cincl_args: build::CInclArgs::try_from(&target.compile_groups[0])?,
        link_args: Some(
            build::LinkArgsBuilder::try_from(&target.link.unwrap())?
                .linker(custom_linker.as_ref().unwrap_or(&compiler))
                .working_directory(&cmake_build_dir)
                .force_ldproxy(true)
                .build()?,
        ),
        bindgen: bindgen::Factory::from_cmake(&target.compile_groups[0])?.with_linker(&compiler),
        components: EspIdfComponents::from(components),
        kconfig_args: Box::new(
            kconfig::try_from_json_file(sdkconfig_json.clone())
                .with_context(|| anyhow!("Failed to read '{:?}'", sdkconfig_json))?,
        ),
        env_path: Some(idf.exported_path.try_to_str()?.to_owned()),
        esp_idf: build_info.esp_idf_dir,
        gcc_sysroot,
        config,
    };

    Ok(build_output)
}

// Generate `sdkconfig.defaults` content based on the crate manifest (`Cargo.toml`).
//
// This is currently only used to forward the optimization options to the esp-idf.
fn generate_sdkconfig_defaults() -> Result<String> {
    const OPT_VARS: [&str; 4] = [
        "CONFIG_COMPILER_OPTIMIZATION_NONE",
        "CONFIG_COMPILER_OPTIMIZATION_DEFAULT",
        "CONFIG_COMPILER_OPTIMIZATION_PERF",
        "CONFIG_COMPILER_OPTIMIZATION_SIZE",
    ];

    let opt_level = env::var("OPT_LEVEL")?;
    let debug = env::var("DEBUG")?;
    let opt_index = match (opt_level.as_str(), debug.as_str()) {
        ("s" | "z", _) => 3,               // -Os
        ("1", _) | (_, "2" | "true") => 1, // -Og
        ("0", _) => 0,                     // -O0
        ("2" | "3", _) => 2,               // -O2
        _ => unreachable!("Invalid DEBUG or OPT_LEVEL"),
    };

    Ok(OPT_VARS
        .iter()
        .enumerate()
        .fold(String::new(), |mut out, (i, s)| {
            writeln!(out, "{}={}", s, if i == opt_index { 'y' } else { 'n' }).unwrap();
            out
        }))
}

/// Create a cmake list (`;`-separated strings), escape all `;` and on Windows make sure
/// paths don't contain `\`.
pub fn to_cmake_path_list(iter: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Result<String> {
    let mut accu = String::new();
    for p in iter {
        let p: &str = p.as_ref().try_to_str()?;
        if !accu.is_empty() {
            accu.push(';');
        }

        // Escape all `;` since cmake uses them as separators.
        let p = p.replace(';', "\\;");

        accu.push_str(
            // Windows uses `\` as directory separators which cmake can't deal with, so we
            // convert all back-slashes to forward-slashes here.
            &if cfg!(windows) {
                p.replace('\\', "/")
            } else {
                p
            },
        );
    }
    Ok(accu)
}

// The bootloader binary gets stored in the build folder of esp-idf-sys. Since this build
// folder is tagged with a fingerprint, it is not easily usable for tools such as espflash
// (see issue https://github.com/esp-rs/esp-idf-sys/issues/97).
//
// This function moves the bootloader.bin file to the regular rust build folder
// (e.g. `target/xtensa-esp32-espidf/release`) so that it can be accessed more easily.
//
// Ditto for the partition table binary.
fn copy_binaries_to_target_folder() -> Result<()> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target_dir = out_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| anyhow!("Cannot locate target dir of {}", out_dir.display()))?
        .canonicalize()?;

    let build_dir = out_dir.join("build");

    let bootloader_src = build_dir.join("bootloader").join("bootloader.bin");
    let bootloader_target = target_dir.join("bootloader.bin");

    fs::copy(&bootloader_src, bootloader_target).with_context(|| {
        format!(
            "Failed to copy bootloader binary {} to target folder {}",
            bootloader_src.display(),
            target_dir.display()
        )
    })?;

    let part_table_src = build_dir
        .join("partition_table")
        .join("partition-table.bin");
    let part_table_target = target_dir.join("partition-table.bin");

    fs::copy(&part_table_src, part_table_target).with_context(|| {
        format!(
            "Failed to copy partition table binary {} to target folder {}",
            part_table_src.display(),
            target_dir.display()
        )
    })?;

    Ok(())
}
