use std::collections::HashMap;
use std::fmt::Write;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use cargo_metadata::{Metadata, Package};
use embuild::cargo::IntoWarning;
use embuild::espidf::parse_esp_idf_git_ref;
use embuild::utils::PathExt;
use embuild::{cmake, git};
use serde::Deserialize;

use crate::common::workspace_dir;
use crate::config::utils::{parse_from_env, set_when_none};
use crate::config::EspIdfSys;

pub const ESP_IDF_VERSION_VAR: &str = "ESP_IDF_VERSION";
pub const ESP_IDF_REPOSITORY_VAR: &str = "ESP_IDF_REPOSITORY";
pub const ESP_IDF_EXTRA_COMPONENTS_FILE_VAR: &str = "ESP_IDF_SYS_EXTRA_COMPONENTS_FILE";

pub const DEFAULT_ESP_IDF_VERSION: &str = "v5.5.5";
pub const DEFAULT_CMAKE_GENERATOR: cmake::Generator = {
    // No Ninja builds for linux=aarch64 from Espressif yet
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        cmake::Generator::UnixMakefiles
    }

    #[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
    {
        cmake::Generator::Ninja
    }
};

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct NativeConfig {
    /// The version of the esp-idf to use.
    #[serde(deserialize_with = "parse::git_ref")]
    pub esp_idf_version: Option<git::Ref>,
    /// The URL to the git repository of the `esp-idf`.
    pub esp_idf_repository: Option<String>,
    /// The cmake generator to use when cmake builds the esp-idf.
    #[serde(deserialize_with = "parse::cmake_generator")]
    esp_idf_cmake_generator: Option<cmake::Generator>,

    /// The path to the esp-idf repository.
    pub idf_path: Option<PathBuf>,

    /// Additional components to build and maybe generate bindings for.
    ///
    /// Can be specified in the root crate's `package.metadata.esp-idf-sys`
    /// (or using `ESP_IDF_SYS_EXTRA_COMPONENTS_FILE`) and all direct dependencies'
    /// `package.metadata.esp-idf-sys`.
    #[serde(default, alias = "extra-components")]
    pub extra_components: Option<Vec<ExtraComponent>>,

    /// A list of esp-idf components (names) that should be built. This list is used to
    /// trim the esp-idf build. Any component that is a dependency of a component in this
    /// list will also automatically be built.
    ///
    /// If this option is not specified, all components will be built. Note though that
    /// some components must be explicitly enabled in the sdkconfig.
    #[serde(default, deserialize_with = "parse::list")]
    pub esp_idf_components: Option<Vec<String>>,

    /// Whether the esp-idf component manager (see [`RemoteComponent`]) should be on
    /// (`true`, `y`, `yes`, `on`) or off (`false`, `n`, `no`, `off`)
    #[serde(default, deserialize_with = "parse::toggle_setting")]
    esp_idf_component_manager: Option<bool>,

    /// Whether the clang toolchain of the esp-idf should be installed and used for the
    /// bindings (`true`, `y`, `yes`, `on`) or not (`false`, `n`, `no`, `off`)
    #[serde(default, deserialize_with = "parse::toggle_setting")]
    esp_idf_install_clang: Option<bool>,
}

impl NativeConfig {
    pub fn try_from_env() -> Result<NativeConfig> {
        let mut cfg: NativeConfig = parse_from_env(&["extra_components"])?;

        // `extra_components` cannot easily be expressed as a single env var (it's a
        // complex array), so instead we support pointing to one or more TOML files via
        // this environment variable.
        embuild::cargo::track_env_var(ESP_IDF_EXTRA_COMPONENTS_FILE_VAR);
        if let Ok(val) = std::env::var(ESP_IDF_EXTRA_COMPONENTS_FILE_VAR) {
            let workspace = workspace_dir()?;
            let mut components = Vec::new();
            for file in val.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                // Path::join replaces the base if `file` is absolute.
                let path = workspace.join(file);
                let contents = std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read '{}'", path.display()))?;
                let file_cfg: NativeConfig = toml::from_str(&contents)
                    .with_context(|| format!("failed to parse '{}'", path.display()))?;
                components.extend(file_cfg.extra_components.unwrap_or_default());
            }
            cfg.extra_components = Some(components);
        }

        Ok(cfg)
    }

    /// Get the value for the `IDF_COMPONENT_MANAGER` variable passed to cmake. The
    /// component manager is on by default (if [`Self::esp_idf_component_manager`] is [`None`]).
    pub fn idf_component_manager(&self) -> &'static str {
        match self.esp_idf_component_manager {
            Some(true) | None => "1",
            Some(false) => "0",
        }
    }

    /// Whether to install and use the clang toolchain of the esp-idf. On by default (if
    /// [`Self::esp_idf_install_clang`] is [`None`]).
    pub fn esp_idf_install_clang(&self) -> bool {
        self.esp_idf_install_clang.unwrap_or(true)
    }

    /// Get the user-specified esp-idf version or [`DEFAULT_ESP_IDF_VERSION`] if unset.
    pub fn esp_idf_version(&self) -> git::Ref {
        self.esp_idf_version
            .clone()
            .unwrap_or_else(|| parse_esp_idf_git_ref(DEFAULT_ESP_IDF_VERSION))
    }

    /// Get the user-specified cmake generator or [`DEFAULT_CMAKE_GENERATOR`] if unset.
    pub fn esp_idf_cmake_generator(&self) -> cmake::Generator {
        self.esp_idf_cmake_generator
            .unwrap_or(DEFAULT_CMAKE_GENERATOR)
    }

    /// Get all component directories of [`Self::extra_components`].
    pub fn extra_component_dirs(&self) -> Result<Vec<PathBuf>> {
        self.extra_components
            .iter()
            .flatten()
            .flat_map(|extra_comp| {
                extra_comp
                    .component_dirs
                    .iter()
                    .map(move |v| (v.abspath_relative_to(&extra_comp.manifest_dir), extra_comp))
            })
            .try_fold(Vec::new(), |mut results, (comp_dir, comp)| {
                const CMAKELISTS_TXT: &str = "CMakeLists.txt";
                // Same behavior as the esp-idf cmake component discovery.
                // https://github.com/espressif/esp-idf/blob/36f49f361c001b49c538364056bc5d2d04c6f321/tools/cmake/project.cmake#L202
                // https://github.com/espressif/esp-idf/blob/36f49f361c001b49c538364056bc5d2d04c6f321/tools/cmake/component.cmake#L98
                if !comp_dir.is_dir() {
                    bail!(
                        "extra component dir '{}' specified by crate '{}' does not exist",
                        comp_dir.display(),
                        comp.manifest_dir.display()
                    );
                }

                if comp_dir.join(CMAKELISTS_TXT).exists() {
                    results.push(comp_dir)
                } else {
                    // `read_dir` yields entries in an unspecified, filesystem-dependent order.
                    // Sort them so that the resulting `EXTRA_COMPONENT_DIRS` - and thus the
                    // component manager's `manifest_hash` - is the same on every machine.
                    let mut entries = comp_dir.read_dir()?.collect::<std::io::Result<Vec<_>>>()?;
                    entries.sort_by_key(|entry| entry.file_name());

                    for entry in entries {
                        // Ignore dot-folders.
                        if entry.file_name().to_string_lossy().starts_with('.') {
                            continue;
                        }
                        if entry.path().join(CMAKELISTS_TXT).exists() {
                            results.push(entry.path());
                        }
                    }
                }
                Ok(results)
            })
    }

    /// Generate the `idf_component.yml` file contents for the component mananager
    /// containing the specified [`RemoteComponent`]s, but only if there is at least one
    /// remote component.
    pub fn generate_idf_component_yml(&self) -> Option<String> {
        let mut contents = String::from("dependencies:\n");
        let remote_components = self
            .extra_components
            .iter()
            .flatten()
            .filter_map(|c| c.remote_component.as_ref())
            .collect::<Vec<_>>();
        if remote_components.is_empty() {
            return None;
        }
        for remote_comp in remote_components {
            let RemoteComponent {
                name,
                version,
                git,
                path,
                service_url,
            } = remote_comp;

            writeln!(&mut contents, "  {name}:").unwrap();
            writeln!(&mut contents, "    version: '{version}'").unwrap();
            if let Some(git) = git {
                writeln!(&mut contents, "    git: '{git}'").unwrap();
            }
            if let Some(path) = path {
                writeln!(&mut contents, "    path: '{path}'").unwrap();
            }
            if let Some(service_url) = service_url {
                writeln!(&mut contents, "    service_url: '{service_url}'").unwrap();
            }
        }
        Some(contents)
    }

    /// Get all bindings C headers of extra components where the bindings will be
    /// generated combined with the normal `esp-idf` bindings
    /// (all extra components where [`ExtraComponent::bindings_module`] is [`None`]).
    ///
    /// This method will validate that all returned C header files exist.
    #[cfg(any(feature = "native", not(feature = "pio")))]
    pub fn combined_bindings_headers(&self) -> Result<Vec<PathBuf>> {
        let mut results = Vec::new();
        for comp in self.extra_components.iter().flatten() {
            // Skip all extra components with separate bindings.
            if comp.bindings_module.is_some() {
                continue;
            }

            if let Some(header) = &comp.bindings_header {
                let header_path = header.abspath_relative_to(&comp.manifest_dir);

                if !header_path.exists() {
                    bail!(
                        "extra components C header file '{}' specified by crate '{}' does not exist",
                        header_path.display(), comp.manifest_dir.display()
                    );
                }
                results.push(header_path);
            }
        }
        Ok(results)
    }

    /// Get all bindings C headers grouped by the [`ExtraComponent::bindings_module`] name.
    ///
    /// This method will validate that all returned C header files exist and also that the
    /// module name only contains ACII alphanumeric and `_` characters.
    #[cfg(any(feature = "native", not(feature = "pio")))]
    pub fn module_bindings_headers(&self) -> Result<HashMap<&str, Vec<PathBuf>>> {
        let headers = self.extra_components.iter().flatten().filter_map(|comp| {
            match (&comp.bindings_header, &comp.bindings_module) {
                (Some(header), Some(module)) => {
                    Some((header.abspath_relative_to(&comp.manifest_dir), module, comp))
                }
                _ => None,
            }
        });
        let mut map = HashMap::<&str, Vec<PathBuf>>::new();

        for (header_path, module_name, comp) in headers {
            if !header_path.exists() {
                bail!(
                    "extra components C header file '{}' specified by crate '{}' does not exist",
                    header_path.display(),
                    comp.manifest_dir.display()
                );
            }
            validate_module_name(module_name, comp)?;
            map.entry(module_name).or_default().push(header_path);
        }
        Ok(map)
    }

    /// Get the configuration from the `package.metadata.esp-idf-sys` object of the root
    /// crate's manifest, and update all options that are [`None`].
    /// Extend [`Self::extra_components`] with all [`ExtraComponent`]s
    /// specified in the root crate's and all direct dependencies' manifest.
    pub fn with_cargo_metadata(&mut self, root: &Package, metadata: &Metadata) -> Result<()> {
        let EspIdfSys {
            v:
                NativeConfig {
                    esp_idf_version,
                    esp_idf_repository,
                    esp_idf_cmake_generator,
                    idf_path,
                    extra_components,
                    esp_idf_components,
                    esp_idf_component_manager,
                    esp_idf_install_clang,
                },
        } = EspIdfSys::deserialize(&root.metadata)?;

        set_when_none(&mut self.esp_idf_version, esp_idf_version);
        set_when_none(&mut self.esp_idf_repository, esp_idf_repository);
        set_when_none(&mut self.esp_idf_cmake_generator, esp_idf_cmake_generator);
        set_when_none(&mut self.idf_path, idf_path);
        set_when_none(&mut self.esp_idf_components, esp_idf_components);
        set_when_none(
            &mut self.esp_idf_component_manager,
            esp_idf_component_manager,
        );
        set_when_none(&mut self.esp_idf_install_clang, esp_idf_install_clang);

        fn make_processor(
            package: &Package,
        ) -> impl Fn(ExtraComponent) -> Option<ExtraComponent> + '_ {
            // Filter empty extra components and set manifest path.
            |mut comp| {
                if comp.bindings_header.is_none()
                    && comp.component_dirs.is_empty()
                    && comp.remote_component.is_none()
                {
                    return None;
                }
                comp.manifest_dir = package
                    .manifest_path
                    .parent()
                    .expect("manifest_path should always have parent")
                    .into();
                Some(comp)
            }
        }

        // Only load extra_components from the root crate's Cargo.toml if not already set
        // (e.g. via ESP_IDF_SYS_EXTRA_COMPONENTS_FILE), following the set_when_none convention.
        if self.extra_components.is_none() {
            self.extra_components = Some(
                extra_components
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(make_processor(root))
                    .collect(),
            );
        }

        // Always load extra_components from direct dependencies regardless.
        let dependencies = metadata
            .resolve
            .as_ref()
            .and_then(|resolve| {
                resolve
                    .nodes
                    .iter()
                    .find(|n| n.id == root.id)
                    .map(|root_node| &root_node.dependencies)
            })
            .into_iter()
            .flatten()
            .flat_map(|id| metadata.packages.iter().find(|p| p.id == *id));

        for dep_package in dependencies {
            let cfg = EspIdfSys::<NativeConfig>::deserialize(&dep_package.metadata)
                .with_context(|| {
                    anyhow!(
                        "failed to parse `package.metadata.esp-idf-sys` of dependency '{}'",
                        dep_package.name
                    )
                })
                .into_warning();

            if let Some(cfg) = cfg {
                self.extra_components.get_or_insert_with(Vec::new).extend(
                    cfg.v
                        .extra_components
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(make_processor(dep_package)),
                );
            }
        }

        Ok(())
    }
}

/// An extra component to be built, bindings to generate.
///
/// An [`ExtraComponent`] may be used to:
/// - build an extra esp-idf component with [`Self::component_dirs`];
/// - add a remote component to the build with [`Self::remote_component`];
/// - generate the bindings of the header specified by [`Self::bindings_header`].
///
/// Note that it is also possible to only build a component, or only generate bindings.
/// This can be used to generate extra bindings of esp-idf headers.
///
/// ## Example
/// ```toml
/// [[package.metadata.esp-idf-sys.extra_components]]
/// component_dirs = ["rainmaker/components/esp-insights/components", "rainmaker/components"]
/// remote_component = { name = "espressif/mdns", version = "1.2" }
/// bindings_header = "bindings.h"
/// bindings_module = "module_name"
/// ```
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(default)]
pub struct ExtraComponent {
    /// A single path or a list of paths to a component directory or directory containing components.
    ///
    /// Each path can be absolute or relative. Relative paths will be relative to the
    /// folder containing the defining `Cargo.toml`.
    ///
    /// **This field is optional.** No component will be built if this field is absent, though
    /// the bindings of the `[Self::bindings_header`] will still be generated.
    #[serde(default, deserialize_with = "parse::value_or_list")]
    pub component_dirs: Vec<PathBuf>,

    /// A remote component to be included in the build. For multiple remote components
    /// consider declaring multiple [`ExtraComponent`]s.
    ///
    /// The components will be managed by the [esp-idf component manager]. Each remote
    /// component will correspond to an `idf_component.yml` `dependencies` entry.
    /// See [`RemoteComponent`] as to what options are available.
    ///
    /// **This field is optional.**
    ///
    /// [esp-idf component manager]: https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-guides/tools/idf-component-manager.html
    #[serde(default)]
    pub remote_component: Option<RemoteComponent>,

    /// The path to the C header to generate the bindings with. If this option is absent,
    /// **no** bindings will be generated.
    ///
    /// The path can be absolute or relative. A relative path will be relative to the
    /// folder containing the defining `Cargo.toml`.
    ///
    /// **This field is optional.**
    #[serde(default)]
    pub bindings_header: Option<PathBuf>,

    /// If this field is present, the component bindings will be generated separately from
    /// the `esp-idf` bindings and put into their own module inside the `esp-idf-sys` crate.
    /// Otherwise, if absent, the component bindings will be added to the existing
    /// `esp-idf` bindings (which are available in the crate root).
    ///
    /// To put the bindings into its own module, a separate bindgen instance will generate
    /// the bindings. Note that this will result in duplicate `esp-idf` bindings if the
    /// same `esp-idf` headers that were already processed for the `esp-idf` bindings are
    /// included by the component(s).
    ///
    /// **This field is optional.**
    #[serde(default)]
    pub bindings_module: Option<String>,

    /// Internal field; the path of the directory containing the manifest (`Cargo.toml`)
    /// that defined this [`ExtraComponent`].
    #[serde(skip)]
    pub manifest_dir: PathBuf,
}

/// A remote component to be managed by the [esp-idf component manager]. Each
/// [`RemoteComponent`] corresponds to an entry in the `dependencies:` section of the `idf_component.yml`.
#[derive(Debug, Deserialize, Clone)]
pub struct RemoteComponent {
    /// The name of the remote component. Corrensponds to a key in the dependencies of
    /// `idf_component.yml`.
    pub name: String,
    /// The version of the remote component. Corresponds to the `version` field of the
    /// `idf_component.yml`.
    pub version: String,
    /// An optional git url that contains this remote component. Corresponds to the `git`
    /// field of the `idf_component.yml`.
    #[serde(default)]
    pub git: Option<String>,
    /// An optional path to the component in case [`RemoteComponent::git`] is used.
    /// Corresponds to the `path` field of the `idf_component.yml`.
    ///
    /// Note: This should not be used for local components, use
    /// [`ExtraComponent::component_dirs`] instead.
    #[serde(default)]
    pub path: Option<String>,
    /// An optional url to a custom component registry. Corresponds to the `service_url`
    /// field of the `idf_component.yml`.
    #[serde(default)]
    pub service_url: Option<String>,
}

mod parse {
    use std::str::FromStr;

    use serde::Deserializer;
    use strum::IntoEnumIterator;

    use super::*;
    pub use crate::config::parse::*;
    use crate::config::utils::ValueOrVec;

    /// Deserialize a toggle setting as a boolean or string with `true` (`"true"`, `"y"`, `"yes"` or `"on"`),
    /// or `false` (`"false"`, `"n"`, `"no"` or `"off"`).
    pub fn toggle_setting<'d, D>(de: D) -> Result<Option<bool>, D::Error>
    where
        D: Deserializer<'d>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum BoolOrString {
            Bool(bool),
            String(String),
        }

        match Option::<BoolOrString>::deserialize(de)? {
            Some(BoolOrString::Bool(b)) => Ok(Some(b)),
            None => Ok(None),
            Some(BoolOrString::String(s)) => {
                const VALUES_ON: [&str; 4] = ["true", "y", "yes", "on"];
                const VALUES_OFF: [&str; 4] = ["false", "n", "no", "off"];
                if VALUES_ON.iter().any(|e| *e == s) {
                    Ok(Some(true))
                } else if VALUES_OFF.iter().any(|e| *e == s) {
                    Ok(Some(false))
                } else {
                    Err(serde::de::Error::custom(format!(
                        "invalid option, should be one of {VALUES_ON:?} for true or {VALUES_OFF:?} for false",
                    )))
                }
            }
        }
    }

    /// Parse a cmake generator, either `default` or one of [`cmake::Generator`].
    pub fn cmake_generator<'d, D: Deserializer<'d>>(
        de: D,
    ) -> Result<Option<cmake::Generator>, D::Error> {
        let gen = Option::<String>::deserialize(de)?
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        let gen = match gen {
            Some(val) => val,
            None => return Ok(None),
        };

        match gen.as_str() {
            "default" => Ok(DEFAULT_CMAKE_GENERATOR),
            other => cmake::Generator::from_str(other).map_err(|_| {
                serde::de::Error::custom(format!(
                    "invalid cmake generator: should be either `default`, or one of [{}]",
                    cmake::Generator::iter()
                        .map(|e| e.into())
                        .collect::<Vec<&'static str>>()
                        .join(", ")
                ))
            }),
        }
        .map(Some)
    }

    pub fn git_ref<'d, D: Deserializer<'d>>(de: D) -> Result<Option<git::Ref>, D::Error> {
        Ok(Option::<String>::deserialize(de)?
            .map(|val| embuild::espidf::parse_esp_idf_git_ref(val.trim())))
    }

    pub fn value_or_list<'d, T, D>(de: D) -> Result<Vec<T>, D::Error>
    where
        D: Deserializer<'d>,
        T: for<'de> Deserialize<'de>,
    {
        Ok(match ValueOrVec::<T>::deserialize(de)? {
            ValueOrVec::Val(val) => vec![val],
            ValueOrVec::Vec(vec) => vec,
        })
    }
}

/// A extra component module name can only contain ASCII alphanumeric and `_` characters.
/// Additionally it must not start with a digit.
#[cfg(any(feature = "native", not(feature = "pio")))]
pub fn validate_module_name(module_name: &str, comp: &ExtraComponent) -> Result<()> {
    if module_name.is_empty() {
        bail!(
            "extra component module name '{}' specified by crate '{}' cannot be empty",
            module_name,
            comp.manifest_dir.display()
        );
    }

    let mut chars = module_name.chars();
    let first_char = chars.next().unwrap();

    let first_char_valid = first_char.is_ascii_alphabetic() || first_char == '_';
    let other_valid = chars.all(|c| c.is_ascii_alphanumeric() || c == '_');

    if !first_char_valid || !other_valid {
        bail!(
            "extra component module name '{}' specified by crate '{}' can only contain \
             ASCII alphanumeric or `_` characters and must be a valid Rust module name",
            module_name,
            comp.manifest_dir.display()
        );
    }
    Ok(())
}
