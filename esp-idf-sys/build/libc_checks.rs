//! The checks of `src/checks/libc.rs` for the `libc` items of `libc_items.rs` which the ESP-IDF
//! bindings have as well

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::Path;
use std::{env, fs};

use anyhow::{bail, Result};
use embuild::{cargo, cmd};

use crate::libc_items::{CONSTS, FNS, STRUCTS, TYPES};

/// Known mismatches, as `(item, cfg)`: the item (`<name>`, or `<struct>::<field>`) is checked only
/// where `cfg` holds, or never if it is `None`. The entry of a struct covers its fields too.
const MISMATCHES: &[(&str, Option<&str>)] = &[
    // On Xtensa, GCC (which builds ESP-IDF) has a 2-byte `unsigned short`, but the bindings, which
    // clang makes, have another type
    ("wchar_t", Some("not(target_arch = \"xtensa\")")),
    // LWIP sizes it for the largest sockaddr, while libc assumes that IPv6 is there
    ("sockaddr_storage", Some("esp_idf_lwip_ipv6")),
    // A 4-byte pointer to a heap object in ESP-IDF and in libc, but libc aligns it to 8, so it pads
    // it to 8 bytes too. Harmless where Rust owns the memory and only passes pointers to ESP-IDF
    ("pthread_cond_t", None),
    ("pthread_mutex_t", None),
    ("pthread_rwlock_t", None),
    // An `int` in ESP-IDF, 12 bytes aligned to 1 in libc. ESP-IDF has no `pthread_rwlockattr_*`
    // functions, and `pthread_rwlock_init` ignores the attributes
    ("pthread_rwlockattr_t", None),
    // 6 in ESP-IDF (LWIP's `NETIF_NAMESIZE`), 16 in libc
    ("IF_NAMESIZE", None),
    ("IFNAMSIZ", None),
    // libc's newlib `termios` has a `c_line` before `c_cc`, which ESP-IDF's does not
    ("termios::c_cc", None),
    // libc's `gethostname` returns nothing, and takes an `ssize_t` rather than a `size_t` length
    ("gethostname", None),
    // libc's `strerror_r` links the GNU one (returning a `char *`), not `__xpg_strerror_r`
    ("strerror_r", None),
];

/// Known signedness mismatches, as `(item, cfg)` like `MISMATCHES`: the signedness of the item (of
/// all its integers, if a function) is checked only where `cfg` holds, while its size always is
const SIGNEDNESS_MISMATCHES: &[(&str, Option<&str>)] = &[
    // `int` in ESP-IDF on RISC-V, `u32` in libc
    ("wchar_t", None),
    // A struct of one `u32` in libc, an `unsigned long` in ESP-IDF: the same layout
    ("sigaction::sa_mask", None),
    // An opaque pointer in libc, an `int` in picolibc: the same size
    ("locale_t", None),
    // libc's `ioctl` takes a `c_ulong` request, ESP-IDF's an `int`: the same bits in a register
    ("ioctl", None),
];

/// The (C) integer types a constant has to resolve to, to be checked by value
const INT_TYPES: &[&str] = &[
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "c_char",
    "c_schar",
    "c_uchar",
    "c_short",
    "c_ushort",
    "c_int",
    "c_uint",
    "c_long",
    "c_ulong",
    "c_longlong",
    "c_ulonglong",
];

/// The top-level items of the (formatted) bindings
#[derive(Default)]
struct Bindings {
    /// Struct name -> its public fields, as field name -> type
    structs: HashMap<String, HashMap<String, String>>,
    /// Type aliases, unions and the other structs
    types: HashSet<String>,
    /// Constant name -> its type
    consts: HashMap<String, String>,
    /// Type alias name -> the aliased type
    aliases: HashMap<String, String>,
    /// Function name -> its declaration
    fns: HashMap<String, Function>,
    /// Function symbol -> the function name
    symbols: HashMap<String, String>,
}

impl Bindings {
    fn scan(source: &str) -> Self {
        let mut bindings = Self::default();
        let mut current: Option<(String, HashMap<String, String>)> = None;
        // Within an `extern "C"` block: the `link_name` and the declaration read so far
        let mut externs: Option<(Option<String>, String)> = None;

        for line in source.lines() {
            if let Some((link_name, decl)) = &mut externs {
                if line == "}" {
                    externs = None;
                } else if let Some(symbol) = line
                    .strip_prefix("    #[link_name = \"")
                    .and_then(|rest| rest.strip_suffix("\"]"))
                {
                    *link_name = Some(symbol.trim_start_matches("\\u{1}").to_string());
                } else if !decl.is_empty() || line.starts_with("    pub ") {
                    decl.push_str(line.trim());
                    decl.push(' ');

                    if line.ends_with(';') {
                        bindings.add_fn(decl, link_name.take());
                        decl.clear();
                    }
                }
            } else if let Some((_, fields)) = &mut current {
                if line == "}" {
                    let (name, fields) = current.take().unwrap();
                    bindings.structs.insert(name, fields);
                } else if let Some((field, ty)) = line
                    .strip_prefix("    pub ")
                    .and_then(|rest| rest.split_once(": "))
                {
                    fields.insert(field.to_string(), ty.trim_end_matches(',').to_string());
                }
            } else if line == "unsafe extern \"C\" {" || line == "extern \"C\" {" {
                externs = Some((None, String::new()));
            } else if let Some(rest) = line.strip_prefix("pub struct ") {
                match rest.strip_suffix(" {") {
                    Some(name) => current = Some((name.to_string(), HashMap::new())),
                    None => {
                        bindings.types.insert(ident(rest).to_string());
                    }
                }
            } else if let Some(rest) = line.strip_prefix("pub union ") {
                bindings.types.insert(ident(rest).to_string());
            } else if let Some(rest) = line.strip_prefix("pub type ") {
                let name = ident(rest);

                bindings.types.insert(name.to_string());

                if let Some((_, ty)) = rest.split_once(" = ") {
                    bindings
                        .aliases
                        .insert(name.to_string(), ty.trim_end_matches(';').to_string());
                }
            } else if let Some((name, rest)) = line
                .strip_prefix("pub const ")
                .and_then(|rest| rest.split_once(": "))
            {
                if let Some((ty, _)) = rest.split_once(" =") {
                    bindings.consts.insert(name.to_string(), ty.to_string());
                }
            }
        }

        bindings
    }

    /// Adds the function of `decl` (`pub fn <name>(<args>) -> <ret>;`), if a function
    fn add_fn(&mut self, decl: &str, link_name: Option<String>) {
        let Some((name, rest)) = decl
            .strip_prefix("pub fn ")
            .and_then(|decl| decl.split_once('('))
        else {
            return;
        };

        // The `)` closing the arguments
        let mut depth = 1;
        let Some(end) = rest.find(|c| {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => (),
            }

            depth == 0
        }) else {
            return;
        };

        let args = split_args(&rest[..end])
            .into_iter()
            .map(|arg| arg.split_once(": ").map_or(arg, |(_, ty)| ty).to_string())
            .collect();

        let ret = rest[end + 1..].trim().trim_end_matches(';').trim();
        let ret = ret.strip_prefix("->").unwrap_or(ret).trim().to_string();

        let symbol = link_name.unwrap_or_else(|| name.to_string());

        self.symbols.insert(symbol.clone(), name.to_string());
        self.fns
            .insert(name.to_string(), Function { symbol, args, ret });
    }

    fn has_type(&self, name: &str) -> bool {
        self.structs.contains_key(name) || self.types.contains(name)
    }

    fn has_int_const(&self, name: &str) -> bool {
        self.consts.get(name).is_some_and(|ty| self.is_int(ty, 0))
    }

    fn is_int(&self, ty: &str, depth: usize) -> bool {
        // Only a path can be an integer, not e.g. a pointer or an array of integers
        let path = |c: char| c.is_alphanumeric() || c == '_' || c == ':';

        if ty.is_empty() || !ty.chars().all(path) {
            return false;
        }

        let name = ty.rsplit("::").next().unwrap_or(ty);

        INT_TYPES.contains(&name)
            || depth < 8
                && self
                    .aliases
                    .get(name)
                    .is_some_and(|ty| self.is_int(ty, depth + 1))
    }
}

/// A function declaration of the bindings
struct Function {
    /// The linked symbol: the name, unless a `link_name` says otherwise
    symbol: String,
    /// The argument types, `...` for the variadic rest
    args: Vec<String>,
    /// The return type, empty if none
    ret: String,
}

/// The generated checks
struct Checks {
    code: String,
    /// Whether the known mismatches are left unchecked, rather than failing the build
    skip_mismatches: bool,
}

impl Checks {
    /// Appends the check of `item`: `code(true)` with the signedness of its integers, if any, and
    /// `code(false)` without it where the signedness is a known mismatch
    fn add_int(&mut self, item: &str, int: bool, code: impl Fn(bool) -> String) {
        match int.then(|| self.mismatch(SIGNEDNESS_MISMATCHES, item)) {
            Some(None) => self.add(item, None, &code(true)),
            Some(Some(Some(cfg))) => {
                self.add(item, Some(cfg), &code(true));
                self.add(item, Some(&format!("not({cfg})")), &code(false));
            }
            None | Some(Some(None)) => self.add(item, None, &code(false)),
        }
    }

    /// Appends `code`, under `cfg` (if any) and the cfg of `item` (or of its struct) if a
    /// known mismatch
    fn add(&mut self, item: &str, cfg: Option<&str>, code: &str) {
        let structure = item.split_once("::").map(|(structure, _)| structure);
        let mut cfgs = Vec::new();

        for item in [Some(item), structure].into_iter().flatten() {
            match self.mismatch(MISMATCHES, item) {
                Some(None) => return,
                Some(Some(item_cfg)) => cfgs.push(item_cfg),
                None => (),
            }
        }

        for cfg in cfgs.into_iter().chain(cfg) {
            writeln!(self.code, "#[cfg({cfg})]").unwrap();
        }

        writeln!(self.code, "{code}").unwrap();
    }

    /// The cfg of `item` in `mismatches`, if there and the known mismatches are skipped
    fn mismatch(
        &self,
        mismatches: &[(&str, Option<&'static str>)],
        item: &str,
    ) -> Option<Option<&'static str>> {
        if !self.skip_mismatches {
            return None;
        }

        mismatches
            .iter()
            .find(|(name, _)| *name == item)
            .map(|(_, cfg)| *cfg)
    }
}

/// Formats `bindings_file` with the `rustfmt` of the current toolchain, of stable or of nightly
///
/// `generate` scans the formatted bindings, and finds nothing in unformatted ones. So when the
/// checks are compiled (the `std` feature), not being able to format them is an error.
pub fn format_bindings(bindings_file: &Path) -> Result<()> {
    let formatted = cmd!("rustfmt", bindings_file).run().is_ok()
        || cmd!("rustup", "run", "stable", "rustfmt", bindings_file)
            .run()
            .is_ok()
        || cmd!("rustup", "run", "nightly", "rustfmt", bindings_file)
            .run()
            .is_ok();

    if !formatted {
        if env::var_os("CARGO_FEATURE_STD").is_some() {
            bail!(
                "Could not format the generated bindings with `rustfmt` (tried the one of the current \
                 toolchain, of stable and of nightly). The checks against `libc` of the `std` feature \
                 need the formatted bindings. Is `rustfmt` installed (`rustup component add rustfmt`)?"
            );
        }

        cargo::print_warning(
            "rustfmt not found in the current toolchain, nor in stable or nightly. \
             The generated bindings will not be properly formatted.",
        );
    }

    Ok(())
}

/// Writes to `out_file` the checks of the items which both `libc_items.rs` and `bindings_file` have
pub fn generate(bindings_file: &Path, out_file: &Path) -> Result<()> {
    const POLICY_VAR: &str = "ESP_IDF_LIBC_MISMATCHES";

    cargo::track_env_var(POLICY_VAR);

    let skip_mismatches = match std::env::var(POLICY_VAR).as_deref().unwrap_or("ignore") {
        "ignore" => true,
        "err" => false,
        policy => bail!("Unsupported value for {POLICY_VAR} env var: {policy}"),
    };

    let bindings = Bindings::scan(&fs::read_to_string(bindings_file)?);
    let mut checks = Checks {
        code: String::new(),
        skip_mismatches,
    };

    for (name, fields) in STRUCTS {
        if !bindings.has_type(name) {
            continue;
        }

        checks.add(name, None, &format!("check_types!({name});"));

        let Some(sys_fields) = bindings.structs.get(*name) else {
            continue;
        };

        // The fields without known mismatches: the non-integer ones, and the integer ones
        let mut unconditional = (Vec::new(), Vec::new());

        for (field, ty) in fields
            .iter()
            .filter_map(|field| sys_fields.get(*field).map(|ty| (*field, ty)))
        {
            let item = format!("{name}::{field}");
            let int = bindings.is_int(ty, 0);

            if checks.mismatch(MISMATCHES, &item).is_some()
                || int && checks.mismatch(SIGNEDNESS_MISMATCHES, &item).is_some()
            {
                checks.add_int(&item, int, |int| {
                    let check_fields = if int {
                        "check_int_fields"
                    } else {
                        "check_fields"
                    };
                    format!("{check_fields}!({name}, {field});")
                });
            } else if int {
                unconditional.1.push(field);
            } else {
                unconditional.0.push(field);
            }
        }

        for (check_fields, fields) in [
            ("check_fields", unconditional.0),
            ("check_int_fields", unconditional.1),
        ] {
            if !fields.is_empty() {
                let fields = fields.join(", ");
                checks.add(name, None, &format!("{check_fields}!({name}, {fields});"));
            }
        }
    }

    for name in TYPES.iter().filter(|name| bindings.has_type(name)) {
        checks.add_int(name, bindings.is_int(name, 0), |int| {
            let check_types = if int {
                "check_int_types"
            } else {
                "check_types"
            };
            format!("{check_types}!({name});")
        });
    }

    for name in CONSTS.iter().filter(|name| bindings.has_int_const(name)) {
        checks.add(name, None, &format!("check_constants!({name});"));
    }

    for (name, symbol) in FNS {
        // The declaration of the same name, or else the one linking the same symbol
        let Some((sys_name, function)) = bindings.fns.get_key_value(*name).or_else(|| {
            let sys_name = bindings.symbols.get(*symbol)?;
            bindings.fns.get_key_value(sys_name)
        }) else {
            continue;
        };

        if function.symbol != *symbol {
            let sys_symbol = &function.symbol;
            checks.add(
                name,
                None,
                &format!("check_symbols!({name}, {symbol:?}, {sys_symbol:?});"),
            );
        }

        let args = function
            .args
            .iter()
            .map(|arg| if arg == "..." { "..." } else { "_" })
            .collect::<Vec<_>>()
            .join(", ");
        let ints = function
            .args
            .iter()
            .enumerate()
            .filter(|(_, ty)| bindings.is_int(ty, 0))
            .map(|(index, _)| index.to_string())
            .chain(bindings.is_int(&function.ret, 0).then(|| "ret".to_string()))
            .collect::<Vec<_>>()
            .join(", ");

        checks.add_int(name, !ints.is_empty(), |int| {
            let ints = if int { ints.as_str() } else { "" };
            format!("check_functions!({name}, {sys_name}, ({args}), [{ints}]);")
        });
    }

    fs::write(out_file, checks.code)?;

    Ok(())
}

/// The identifier `item` starts with
fn ident(item: &str) -> &str {
    let end = item
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(item.len());

    &item[..end]
}

/// The comma-separated `args`, without the commas within the argument types
fn split_args(args: &str) -> Vec<&str> {
    let mut split = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let mut prev = ' ';

    for (index, c) in args.char_indices() {
        match c {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' => depth -= 1,
            // Not the `>` of a `->`
            '>' if prev != '-' => depth -= 1,
            ',' if depth == 0 => {
                split.push(args[start..index].trim());
                start = index + 1;
            }
            _ => (),
        }

        prev = c;
    }

    split.push(args[start..].trim());
    split.retain(|arg| !arg.is_empty());

    split
}
