//! Locating a file's laplace project (nearest `laplace.lock` walking
//! upward) and its installed package cache -- the same
//! `~/.laplace/packages/<name>/<version>/` layout the `laplace` CLI uses.

use std::path::{Path, PathBuf};

pub fn find_lockfile(start_dir: &Path) -> Option<PathBuf> {
    let mut dir = Some(start_dir);
    while let Some(d) = dir {
        let candidate = d.join("laplace.lock");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

pub fn default_cache_root() -> PathBuf {
    let home = std::env::var_os("HOME").expect("HOME must be set for laplace-lsp to locate ~/.laplace");
    PathBuf::from(home).join(".laplace").join("packages")
}

/// Mirrors `laplace`'s own (private) `load_installed_package`: read an
/// installed package's manifest + concatenated `.stan` source and extract
/// its signatures, ready for `laplace::codegen::generate`.
pub fn load_installed_package(
    package_dir: &Path,
    name: &str,
) -> Result<laplace::codegen::InstalledPackage, String> {
    let pkg_manifest = laplace::manifest::read_package_manifest(&package_dir.join("laplace.toml"))
        .map_err(|e| e.to_string())?;
    let source = laplace::manifest::read_package_stan_source(package_dir).map_err(|e| e.to_string())?;
    let signatures = laplace::parser::signatures::extract_signatures(&source);
    Ok(laplace::codegen::InstalledPackage {
        name: name.to_string(),
        source,
        signatures,
        exported: pkg_manifest.exports,
    })
}

/// Best-effort go-to-definition target: the first installed `.stan` file
/// (in the same sorted order `read_package_stan_source` concatenates them)
/// whose extracted signatures include `func`, and a byte offset onto its
/// `func(` header text within that single file.
pub fn find_function_definition(package_dir: &Path, func: &str) -> Option<(PathBuf, usize)> {
    let mut stan_files: Vec<PathBuf> = std::fs::read_dir(package_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("stan"))
        .collect();
    stan_files.sort();

    for file in stan_files {
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        let sigs = laplace::parser::signatures::extract_signatures(&source);
        if !sigs.iter().any(|s| s.name == func) {
            continue;
        }
        if let Some(offset) = find_function_header_offset(&source, func) {
            return Some((file, offset));
        }
    }
    None
}

/// First whole-word occurrence of `name(` in `source`. Good enough given
/// the caller already confirmed `name` is a genuine top-level function in
/// this file via `extract_signatures` -- the only way this can point at the
/// wrong spot is a function whose name is called before its own definition
/// elsewhere in the same file.
fn find_function_header_offset(source: &str, name: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let name_bytes = name.as_bytes();
    let mut i = 0usize;
    while i + name_bytes.len() <= bytes.len() {
        if &bytes[i..i + name_bytes.len()] == name_bytes {
            let before_ok = i == 0 || !laplace::parser::brace_match::is_ident_char(bytes[i - 1]);
            let end = i + name_bytes.len();
            let after_ok = end < bytes.len() && bytes[end] == b'(';
            if before_ok && after_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}
