# Customizing the Build

**Table of contents**
- [Customizing the Build](#customizing-the-build)
  - [Rust configuration flags](#rust-configuration-flags)
  - [Features](#features)
  - [sdkconfig](#sdkconfig)
    - [(*native* builder only) Using cargo-idf to interactively modify ESP-IDF's `sdkconfig` file](#native-builder-only-using-cargo-idf-to-interactively-modify-esp-idfs-sdkconfig-file)
    - [(*pio* builder only) Using cargo-pio to interactively modify ESP-IDF's `sdkconfig` file](#pio-builder-only-using-cargo-pio-to-interactively-modify-esp-idfs-sdkconfig-file)
  - [ESP-IDF configuration](#esp-idf-configuration)
    - [Example](#example)
  - [Extra ESP-IDF components](#extra-esp-idf-components)
  - [Remote components (idf component registry)](#remote-components-idf-component-registry)
  - [Conditional compilation](#conditional-compilation)
  - [More info](#more-info)

## Rust configuration flags
The following are flags passed to `rustc` that influence the build.

- ### `--cfg espidf_time64`
  This is a flag for the [libc crate](https://docs.rs/libc/) that uses 64-bits (instead of 32-bits) for
  `time_t`. This *must be set* for ESP-IDF 5.0 and above and *must be unset* for lesser versions.

- ### `-Zbuild-std=std,panic_abort`
  Required for `std` support. Rust does not provide `std` libraries for ESP32 targets since they are tier-2/-3.

## Features
- ### `native`
  This is the default feature for downloading all tools and building the ESP-IDF framework using the framework's "native" (own) tooling.
  It relies on build and installation utilities available in the [embuild](https://github.com/ivmarkov/embuild) crate.

  The `native` builder installs all needed tools to compile this crate as well as the ESP-IDF framework itself. 

- ### `pio`

  This is a backup feature for installing all build tools and building the ESP-IDF framework. It uses [PlatformIO](https://platformio.org/) via the
  [embuild](https://github.com/ivmarkov/embuild) crate.

  Similarly to the `native` builder, the `pio` builder also automatically installs all needed tools (PlatformIO packages and frameworks in this case) to compile this crate as well as the ESP-IDF framework itself. 

  > [!WARNING]
  > The `pio` builder is less flexible than the default `native` builder in that it can work with only **one, specific** version of ESP-IDF. At the time of writing, this is V4.3.2.
  
- ### `binstart`

  Defines the esp-idf entry-point for when the root crate is a [binary
  crate](https://doc.rust-lang.org/cargo/reference/cargo-targets.html#binaries) that
  defines a
  [`main`](https://doc.rust-lang.org/reference/crates-and-source-files.html?highlight=main#main-functions)
  function.

- ### `libstart`

  Defines the esp-idf entry-point for when the root crate is a [library crate](https://doc.rust-lang.org/cargo/reference/cargo-targets.html#library).
  The root crate is expected to provide a
  ```ignore
  #[no_mangle]
  fn main() {}
  ```
  function.

## sdkconfig

The esp-idf makes use of an [`sdkconfig`](#espidfsdkconfig-espidfsdkconfig) file for its
compile-time component configuration (see the [esp-idf
docs](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/kconfig.html#project-configuration)
for more information). This config is separate from the [build configuration](#build-configuration).

### (*native* builder only) Using cargo-idf to interactively modify ESP-IDF's `sdkconfig` file

TBD: Upcoming

### (*pio* builder only) Using cargo-pio to interactively modify ESP-IDF's `sdkconfig` file

To enable Bluetooth, or do other configurations to the ESP-IDF sdkconfig you might take advantage of the cargo-pio Cargo subcommand:
- To install it, issue `cargo install cargo-pio --git https://github.com/ivmarkov/cargo-pio`
- To open the ESP-IDF interactive menuconfig system, issue `cargo pio espidf menuconfig` in the root of your **binary crate** project
- To use the generated/updated `sdkconfig` file, follow the steps described in the "Bluetooth Support" section

## ESP-IDF configuration

There are two ways to configure how the ESP-IDF framework is compiled:
1. Environment variables, denoted by `$VARIABLE`;

   > The environment variables can be passed on the command line, or put into the `[env]`
   > section of a `.cargo/config.toml` file (see [cargo reference](https://doc.rust-lang.org/cargo/reference/config.html#env)).

2. The `[package.metadata.esp-idf-sys]` section of the `Cargo.toml`, denoted by *`field`*.

   > [!NOTE]
   > Configuration can only come from the **root crate's** `Cargo.toml`. The root crate
   > is the package in the *workspace directory*. If there is no root crate in case of a
   > [virtual
   > workspace](https://doc.rust-lang.org/cargo/reference/workspaces.html#virtual-manifest),
   > its *name* can be specified with the `ESP_IDF_SYS_ROOT_CRATE` environment variable.

   > [!WARNING]
   > Environment variables always take precedence over `Cargo.toml` metadata.

> [!NOTE]
> ***workspace directory***
>
> The workspace directory mentioned here is always the directory containing the
> `Cargo.lock` file and the `target` directory (unless configured otherwise, see the note
> below about `CARGO_TARGET_DIR`) where the build artifacts are stored. It can be
> overridden with the `CARGO_WORKSPACE_DIR` environment variable, should this not be the
> right directory.  
> (See
> [`embuild::cargo::workspace_dir`](https://docs.rs/embuild/latest/embuild/cargo/fn.workspace_dir.html)
> for more information).
>
> There is no need to explicitly add a 
> [`[workspace]`](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-workspace-section)
> section to the `Cargo.toml` of the workspace directory.
>
> Please note that if you have set `CARGO_TARGET_DIR` and moved your `target` directory out of
> the crate root, then embuild is not able to locate the crate root. This will result in it
> among other things ignoring your local `sdkconfig.defaults`. In this case you must
> declare:
> ```
> [env]
> CARGO_WORKSPACE_DIR = { value = "", relative = true }
> ```
> in the `.cargo/config.toml` file, to force it to look in the current directory.
       

The following configuration options are available:

- ### *`esp_idf_sdkconfig_defaults`*, `$ESP_IDF_SDKCONFIG_DEFAULTS`

    A single path or a list of paths to `sdkconfig.defaults` files to be used as base
    values for the [`sdkconfig`](#espidfsdkconfig-espidfsdkconfig). If such a path is
    relative, it will be relative to the *workspace directory*.
    
    Defaults to `sdkconfig.defaults`.
    
    In case of the environment variable, multiple elements should be `;`-separated.
    
    > [!NOTE]
    > For each defaults file in this list, a more specific file will also be searched and
    > used. This happens with the following patterns and order (least to most specific):
    >
    > 1. `<path>`
    > 2. `<path>.<profile>`
    > 3. `<path>.<mcu>`
    > 4. `<path>.<profile>.<mcu>`
    > 
    > where `<profile>` is the current cargo profile used (`debug`/`release`) and `<mcu>`
    > specifies the mcu for which this is currently compiled for (see the [*`mcu`*](#mcu-mcu)
    > configuration option below).

    > [!WARNING]
    > A setting contained in a more specific defaults file will override the
    > same setting specified in a less specific one. For example, in a debug
    > build, flags in `sdkconfig.debug` override those in `sdkconfig.defaults`.
    

- ### *`esp_idf_sdkconfig`*, `$ESP_IDF_SDKCONFIG`

  The `sdkconfig` file used to [configure the
  `esp-idf`](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/kconfig.html).
  If this is a relative path, it is relative to the *workspace directory*.
  
  Defaults to `sdkconfig`.
  
  > [!NOTE]
  > Similar to the `sdkconfig.defaults`-file a more specific `sdkconfig`-file will be
  > selected if available. This happens with the following patterns and precedence:
  >
  > 1. `<path>.<profile>.<mcu>`
  > 2. `<path>.<mcu>`
  > 3. `<path>.<profile>`
  > 4. `<path>`
  >
  > &nbsp;

  > [!NOTE]
  > ***native* builder only:**  
  > The cargo optimization options (`debug` and `opt-level`) are used by default to
  > determine the compiler optimizations of the `esp-idf`, **however** if the compiler
  > optimization options are already set in the `sdkconfig` **they will be used instead.**

- ### *`esp_idf_tools_install_dir`*, `$ESP_IDF_TOOLS_INSTALL_DIR`

  The install location for the ESP-IDF framework tooling.

  > [!NOTE]
  > The framework tooling is either [PlatformIO](https://platformio.org/) when the `pio` builder is used, or the ESP-IDF
  > native toolset when the `native` builder is used (default).

  This option can take one of the following values:
    - `workspace` (default) - the tooling will be installed or used in
      `<crate-workspace-dir>/.embuild/platformio` for `pio`, and
      `<crate-workspace-dir>/.embuild/espressif` for the `native` builder;
    - `out` - the tooling will be installed or used inside *esp-idf-sys*'s [build output
      directory](https://doc.rust-lang.org/cargo/reference/build-scripts.html#outputs-of-the-build-script),
      and will be deleted when `cargo clean` is invoked;
    - `global` - the tooling will be installed or used in its standard directory
      (`~/.platformio` for PlatformIO, and `~/.espressif` for the native ESP-IDF toolset);
    - `custom:<dir>` -  the tooling will be installed or used in the directory specified by
      `<dir>`. If this directory is a relative location, it is assumed to be relative to
      the *workspace directory*;
    - `fromenv` - use the build framework from the environment 
        - *native* builder: use activated esp-idf environment (see esp-idf docs
      [unix](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/get-started/linux-macos-setup.html#step-4-set-up-the-environment-variables)
      /
      [windows](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/get-started/windows-setup.html#using-the-command-prompt))
        - *pio* builder: use `platformio` from the environment (i.e. `$PATH`)

      and error if this is not possible.
      
   > [!WARNING]
   > Please be extra careful with the `custom:<dir>` setting when switching
   > from `pio` to `native` and the other way around, because the builder will install the
   > tooling in `<dir>` without using any additional `platformio` or `espressif`
   > subdirectories, so if you are not careful, you might end up with both PlatformIO, as
   > well as the ESP-IDF native tooling intermingled together in a single folder.

   > [!WARNING]
   > The [ESP-IDF git repository](https://github.com/espressif/esp-idf) will be cloned
   > *inside* the tooling directory. The *native* builder will use the esp-idf at
   > [*`idf_path`*](#idfpath-idfpath-native-builder-only) if available.
      
- ### *`idf_path`*, `$IDF_PATH` (*native* builder only)
  A path to a user-provided local clone of the [esp-idf](https://github.com/espressif/esp-idf),
  that will be used instead of the one downloaded by the build script.

- ### *`esp_idf_version`*, `$ESP_IDF_VERSION` (*native* builder only)

  The version used for the `esp-idf`, can be one of the following:
  - `commit:<hash>`: Uses the commit `<hash>` of the `esp-idf` repository.
    Note that this will clone the whole `esp-idf` not just one commit.
  - `tag:<tag>`: Uses the tag `<tag>` of the `esp-idf` repository.
  - `branch:<branch>`: Uses the branch `<branch>` of the `esp-idf` repository.
  - `v<major>.<minor>` or `<major>.<minor>`: Uses the tag `v<major>.<minor>` of the `esp-idf` repository.
  - `<branch>`: Uses the branch `<branch>` of the `esp-idf` repository.

  Defaults to `v4.4.1`.

- ### *`esp_idf_repository`*, `$ESP_IDF_REPOSITORY` (*native* builder only)

  The URL to the git repository of the `esp-idf`, defaults to <https://github.com/espressif/esp-idf.git>.
  
  > [!NOTE]
  > When the `pio` builder is used, it is possible to achieve something similar to
  > `ESP_IDF_VERSION` and `ESP_IDF_REPOSITORY` by using the
  > [`platform_packages`](https://docs.platformio.org/en/latest/projectconf/section_env_platform.html#platform-packages)
  > PlatformIO option as follows:
  >
  > `ESP_IDF_PIO_CONF="platform_packages = framework-espidf @ <git-url> [@ <git-branch>]"`
  >
  > The above approach however has the restriction that PlatformIO will always use the ESP-IDF build tooling from
  > its own ESP-IDF distribution, so the user-provided ESP-IDF branch may or may not compile. The current
  > PlatformIO tooling is suitable for compiling ESP-IDF branches derived from versions 4.3.X and 4.4.X.

- ### `$ESP_IDF_GLOB[_XXX]_BASE` and `$ESP_IDF_GLOB[_XXX]_YYY`

  A pair of environment variable prefixes that enable copying files and directory trees that match a certain glob mask into the native C project used for building the ESP-IDF framework:
  - `ESP_IDF_GLOB[_XXX]_BASE` specifies the base directory which will be glob-ed for resources to be copied
  - `ESP_IDF_GLOB[_XXX]_BASE_YYY` specifies one or more environment variables that represent the glob masks of resources to be searched for and copied, using the directory designated by the `ESP_IDF_GLOB[_XXX]_BASE` environment variable as the root. For example, if the following variables are specified:
    - `ESP_IDF_GLOB_HOMEDIR_BASE=/home/someuser`
    - `ESP_IDF_GLOB_HOMEDIR_FOO=foo*`
    - `ESP_IDF_GLOB_HOMEDIR_BAR=bar*`
    ... then all files and directories matching 'foo*' or 'bar*' from the home directory of the user will be copied into the ESP-IDF C project.

    Note also that `_HOMEDIR` in the above example is optional, and is just a mechanism allowing the user to specify more than one base directory and its glob patterns.


- ### `$ESP_IDF_PIO_CONF_XXX` (*pio* builder only)

  A PlatformIO setting (or multiple settings separated by a newline) that will be passed
  as-is to the `platformio.ini` file of the C project that compiles the ESP-IDF.
  
  Check [the PlatformIO
  documentation](https://docs.platformio.org/en/latest/projectconf/index.html) for more
  information as to what settings you can pass via this variable.
  
  > [!NOTE]
  > This is not one variable, but rather a family of variables all
  > starting with `ESP_IDF_PIO_CONF_`. For example, passing `ESP_IDF_PIO_CONF_1` as well as
  > `ESP_IDF_PIO_CONF_FOO` is valid and all such variables will be honored.

- ### *`esp_idf_cmake_generator`*, `$ESP_IDF_CMAKE_GENERATOR` (*native* builder only)

  The CMake generator to be used when building the ESP-IDF.
  
  If not specified or set to `default`, Ninja will be used on all platforms except
  Linux/aarch64, where (for now) the Unix Makefiles generator will be used, as there are
  no Ninja builds for that platform provided by Espressif yet.
  
  Possible values for this environment variable are [the names of all command-line
  generators that CMake
  supports](https://cmake.org/cmake/help/latest/manual/cmake-generators.7.html#cmake-generators)
  with **spaces and hyphens removed**.

- ### *`esp_idf_path_issues`*, `$ESP_IDF_PATH_ISSUES`

  What should happen to the build process if the Rust project path does not meet certain criteria (i.e. path too long on Windows or path contains spaces on Linux). Possible values:
  * `err` (default) - Fail the build
  * `warn` - Issue a warning but continue the build
  * `ignore` - Continue the build and do not issue a warning
  

- ### *`esp_idf_c_env_vars_issues`*, `$ESP_IDF_C_ENV_VARS_ISSUES` (non-CMake build only)

  What should happen to the build process if certain environment variables that might fail the ESP IDF C build are detected. Possible values:
  * `warnremove` (default) - Do not pass these variables to the ESP IDF C build, and issue a build warning
  * `remove` - Same as above but do not issue a warning
  * `err` - Fail the build
  * `warn` - Issue a warning but do not remove the variables and continue the build
  * `ignore` - Continue the build and do not issue a warning

  The currently detected environment variables that might be problematic are as follows: `CC`, `CXX`, `CFLAGS`, `CCFLAGS`, `CXXFLAGS`, `CPPFLAGS`, `LDFLAGS`, `GCC_EXEC_PREFIX`, `COMPILER_PATH`, `C_INCLUDE_PATH`, `CPLUS_INCLUDE_PATH`.

- ### *`esp_idf_espup_clang_symlink`*, `$ESP_IDF_ESPUP_CLANG_SYMLINK`

  Background:

  As part of installing the `esp` Rust toolchain, the `espup` utility - on Unix-like systems - configures a hidden symlink in its private folder that points to the Clang compiler library that is also distributed with the `esp` Rust toolchain (and which - just like the `esp` Rust toolchain itself - does support the `xtensa` architecture). 

  Since `esp-idf-sys` uses `bindgen` to generate raw bindings for the C ESP IDF APIs, it needs to have the `LIBCLANG_PATH` env var configured to point to the CLang library.
  
  `esp-idf-sys` does this automatically, by using the symlink provided by `espup`.

  Following options are available:
  * `try` (default) - Check if the symlink is available and use it; continue the build expecting a user-defined `LIBCLANG_PATH` env var otherwise
  * `warn` - Same as `try` but report a warning if the symlink is not available
  * `err` - Fail the build if the symlink is not available or broken
  * `ignore` - Do not use the symlink at all

- ### *`esp_idf_component_manager`*, `$ESP_IDF_COMPONENT_MANAGER`

    Whether the [esp-idf component manager](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-guides/tools/idf-component-manager.html)
    should be on or off.

    Can be any of `true`, `y`, `yes`, `on` for on, and `false`, `n`,
    `no`, `off` for off.
    
    If not specified, it is *on by default*.

- ### *`mcu`*, `$MCU`

   The MCU name (i.e. `esp32`, `esp32s2`, `esp32s3` `esp32c3`, `esp32c2`, `esp32h2`, `esp32c5`, `esp32c6`, `esp32c61`, `esp32p4`, `esp32s31`). 
   
   If not set this will be automatically detected from the cargo target.

   > [!WARNING]
   > [Older ESP-IDF versions might not support all MCUs from above.](https://github.com/espressif/esp-idf#esp-idf-release-and-soc-compatibility)
   
- ### `$ESP_IDF_SYS_EXTRA_COMPONENTS_FILE` (*native* builder only)

    A comma-separated list of paths to TOML files that each specify additional
    [`extra_components`](#extra-esp-idf-components) entries. This is useful for
    conditionally enabling extra components without modifying `Cargo.toml`,
    for example in CI.

    Each file uses the same syntax as `Cargo.toml` metadata, but without the
    `[package.metadata.esp-idf-sys]` wrapper:

    ```toml
    [[extra_components]]
    remote_component = { name = "espressif/mqtt", version = "1.*" }
    ```

    Multiple files can be specified with comma separation:

    ```bash
    ESP_IDF_SYS_EXTRA_COMPONENTS_FILE="extra_mqtt.toml,extra_eth.toml" cargo build ...
    ```

    Relative paths are resolved relative to the *workspace directory*. Use absolute paths
    if unsure.

    When set, this takes precedence over `extra_components` in the root crate's `Cargo.toml`.
    Extra components from direct dependencies are still merged in regardless.

    > [!NOTE]
    > This option is only available as an environment variable; there is no `Cargo.toml`
    > equivalent (use `extra_components` directly instead).

- ### *`esp_idf_components`*, `$ESP_IDF_COMPONENTS` (*native* builder only)

    > **Note**  
    > The esp-idf is split into components, where one component is essentially a
    > library with its name typically being the name of the containing directory (for more
    > information see the [esp-idf build system
    > docs](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-guides/build-system.html)).
    >
    > To see which components `esp-idf-sys` compiled, run the build with the `-vv` flag
    > (to display build script output), and look for `[esp-idf-sys <version>] Built
    > components: ...` in the output.

    The (`;`-separated for the environment variable) list of esp-idf component names that
    should be built. This list is used to trim the esp-idf build. Any component that is a
    dependency of a component in this list will also automatically be built.
    
    Defaults to all components being built.
    
    > [!NOTE]  
    > Some components must be explicitly enabled in the sdkconfig.  
    > [Extra components](#extra-esp-idf-components) must also be added to this list if
    > they are to be built.

### Example

An example of the `[package.metadata.esp-idf-sys]` section of the `Cargo.toml`.
```toml
[package.metadata.esp-idf-sys]
esp_idf_tools_install_dir = "global"
esp_idf_sdkconfig = "sdkconfig"
esp_idf_sdkconfig_defaults = ["sdkconfig.defaults", "sdkconfig.defaults.ble"]
# native builder only
esp_idf_version = "branch:release/v4.4"
esp_idf_components = ["pthread"]
```

## Extra ESP-IDF components

It is possible to let *esp-idf-sys* compile extra [ESP-IDF
components](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-guides/build-system.html#concepts)
and generate bindings for them.

This is possible by adding an object to the
`package.metadata.esp-idf-sys.extra_components` array of the `Cargo.toml`. *esp-idf-sys*
will honor all such extra components in the *root crate*'s and all **direct**
dependencies' `Cargo.toml`.

Alternatively, extra components can be specified via the `$ESP_IDF_SYS_EXTRA_COMPONENTS_FILE`
environment variable (see above).

> [!NOTE]
> By only specifying the `bindings_header` field, one can extend the set of *esp-idf*
> bindings that were generated from
> [src/include/esp-idf/bindings.h](src/include/esp-idf/bindings.h). To do this you need to create
> a `*.h` header file in your project source, and reference that in the `bindings_header` variable. You can
> then include extra esp-idf header files from there.

An extra component can be specified like this:

```toml
[[package.metadata.esp-idf-sys.extra_components]]
# A single path or a list of paths to a component directory or directory 
# containing components.
# 
# Each path can be absolute or relative. Relative paths will be relative to the
# folder containing the defining `Cargo.toml`.
# 
# **This field is optional.** No component will be built if this field is absent, though
# the bindings of the `[Self::bindings_header]` will still be generated.
component_dirs = ["dir1", "dir2"] # or "dir"


# A remote component to be included in the build. For multiple remote components
# consider declaring multiple extra components.
#
# The components will be managed by the esp-idf component manager. Each remote
# component will correspond to an `idf_component.yml` `dependencies` entry.
# See the Remote component section as to what options are available.
#
# **This field is optional.**
remote_component = { ... }

# The path to the C header to generate the bindings with. If this option is absent,
# **no** bindings will be generated.
#
# The path can be absolute or relative. A relative path will be relative to the
# folder containing the defining `Cargo.toml`.
#
# This field is optional.
bindings_header = "bindings.h"

# If this field is present, the component bindings will be generated separately from
# the `esp-idf` bindings and put into their own module inside the `esp-idf-sys` crate.
# Otherwise, if absent, the component bindings will be added to the existing
# `esp-idf` bindings (which are available in the crate root).
#
# To put the bindings into its own module, a separate bindgen instance will generate
# the bindings. Note that this will result in duplicate `esp-idf` bindings if the
# same `esp-idf` headers that were already processed for the `esp-idf` bindings are
# included by the component(s).
#
# This field is optional.
bindings_module = "name"
```

and is equivalent to
```toml
[package.metadata.esp-idf-sys]
extra_components = [
    { component_dirs = [ "dir1", "dir2" ], bindings_header = "bindings.h", bindings_module = "name" }
]
```

## Remote components (idf component registry)

The esp-idf build systems supports remote components managed by the [esp-idf component
manager](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-guides/tools/idf-component-manager.html).
All remote component dependencies can be specified using the [extra esp-idf
components](#extra-esp-idf-components) `remote_component` field. Every such dependency
maps exactly to a dependency entry in the `idf_component.yml` manifest.

The component manager will resolve all such dependencies, in addition to those of the C
esp-idf components, and download them to the `managed_components` directory in the
esp-idf-sys `out` (build output) directory.

A lock file (`components_<chip>.lock`) will be generated in the *workspace directory* if
there is at least one such dependency.

See the [esp_idf_component_manager](#esp_idf_component_manager-esp_idf_component_manager)
options to turn the component manager off.

A remote component can be specified by:
```toml
[package.metadata.esp-idf-sys.extra_components.0.remote_component]
# The name of the remote component. Corresponds to a key in the dependencies of
# `idf_component.yml`.
name = "component_name"
# The version of the remote component. Corresponds to the `version` field of the
# `idf_component.yml`.
version = "1.2"
# A git url that contains this remote component. Corresponds to the `git`
# field of the `idf_component.yml`.
#
# This field is optional.
git = "https://github.com/espressif/esp32-camera.git"
# A path to the component.
# Corresponds to the `path` field of the `idf_component.yml`.
#
# Note: This should not be used for local components, use
# `component_dirs` of extra components instead.
#
# This field is optional.
path = "path/to/component"
# A url to a custom component registry. Corresponds to the `service_url`
# field of the `idf_component.yml`.
#
# This field is optional.
service_url = "https://componentregistry.company.com"
```

For example, to add a dependency the `1.2.x` version of the `espressif/mdns` component,
add the following to your `Cargo.toml`:
```toml
[[package.metadata.esp-idf-sys.extra_components]]
remote_component = { name = "espressif/mdns", version = "1.2" }
```

> **Note**  
> Slashes (`/`) in a remote component's name will be replaced with two underscores `__`
> for the name of the compiled component (e.g. `espressif/mdns` will become
> `espressif__mdns` with the [`cfg`](#conditional-compilation)
> `esp_idf_comp_espressif__mdns_enabled`).

Remote components that are **not** already included inside the [esp-idf-sys bindings.h](https://github.com/esp-rs/esp-idf/blob/master/esp-idf-sys/src/include/esp-idf/bindings.h) file must be manually added, in its own C header file.

For example, the `espressif/esp32-camera` component could be included in the following way:
```toml
[[package.metadata.esp-idf-sys.extra_components]]
remote_component = { name = "espressif/esp32-camera", version = "2.0.7" }
bindings_header = "your_bindings.h"
bindings_module = "camera"
```

and the `your_bindings.h` file could look like this:
```c
#if defined(ESP_IDF_COMP_ESPRESSIF__ESP32_CAMERA_ENABLED)
#include "esp_camera.h"
#endif
```

In this case the bindings would be generated in the `esp_idf_sys::camera` module.

## Conditional compilation

The *esp-idf-sys* build script will set [rustc *cfg*s](https://doc.rust-lang.org/reference/conditional-compilation.html)
available for its sources.

> [!IMPORTANT]
> If an upstream crate also wants to have access to the *cfg*s it must:
> - have `esp-idf-sys` as a dependency, and
> - output the *cfg*s in its [build
>   script](https://doc.rust-lang.org/cargo/reference/build-scripts.html) with
> 
>   ```rust
>   embuild::espidf::sysenv::output();
>   ```
>   using the [embuild](https://crates.io/crates/embuild) crate.

The list of available *cfg*s:
- `esp_idf_comp_{component}_enabled` for each [component](#espidfcomponents-espidfcomponents-native-builder-only)
- `esp_idf_version="{major}.{minor}"`
- `esp_idf_version_full="{major}.{minor}.{patch}"`
- `esp_idf_version_major="{major}"`
- `esp_idf_version_minor="{minor}"`
- `esp_idf_version_patch="{patch}"`
- `esp_idf_{sdkconfig_option}`

  Each [sdkconfig
  setting](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/kconfig.html#configuration-options-reference)
  where `{sdkconfig_option}` corresponds to the option set in the sdkconfig **lowercased**,
  **without** the `CONFIG_` prefix and **with** a lowercase `esp_idf_` prefix. Only options set to `y` will get a *cfg*.

- `{mcu}`

  Corresponds to the [mcu](#mcu-mcu) for which the esp-idf is compiled for.

## More info

If you are interested in how it all works under the hood, check the [build.rs](build/build.rs)
build script of this crate.
