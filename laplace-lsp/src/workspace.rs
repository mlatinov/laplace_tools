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

/// Load an installed package into the form `laplace::codegen` wants.
///
/// This delegates to `laplace::package::load` rather than reading the
/// directory itself, so a package written in the `.laplacelib` dialect
/// resolves here exactly as it does in a real `laplace build`: its
/// `library { }` block stripped, any `functions { }` wrapper unwrapped, and
/// its own imports reported as dependencies. Reading only `*.stan` (as this
/// did before the library dialect existed) made every `.laplacelib` package
/// look empty, so calls into one were wrongly flagged as unresolved.
pub fn load_installed_package(
    package_dir: &Path,
    name: &str,
) -> Result<laplace::codegen::InstalledPackage, String> {
    laplace::package::load(package_dir, name).map_err(|e| e.to_string())
}

/// Best-effort go-to-definition target: the first installed source file
/// (in the same sorted order `laplace::package::read_package_sources`
/// concatenates them) whose extracted signatures include `func`, and a byte
/// offset onto its `func(` header text within that single file.
///
/// Both of a package's source kinds are searched: plain `.stan` files and
/// `.laplacelib` files. A `.laplacelib` is read verbatim rather than through
/// `laplacelib::parse`, because the offset returned has to point into the
/// file the editor will actually open, not into a stripped copy of it.
pub fn find_function_definition(package_dir: &Path, func: &str) -> Option<(PathBuf, usize)> {
    let mut stan_files: Vec<PathBuf> = std::fs::read_dir(package_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("stan") | Some(laplace::parser::laplacelib::LAPLACELIB_EXTENSION)
            )
        })
        .collect();
    stan_files.sort();

    for file in stan_files {
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        if !defines_function(&file, &source, func) {
            continue;
        }
        if let Some(offset) = find_function_header_offset(&source, func) {
            return Some((file, offset));
        }
    }
    None
}

/// Does this package source file define a top-level function called `func`?
///
/// A `.laplacelib` is checked through `laplacelib::parse` first, because
/// `extract_signatures` reads a flat sequence of definitions: run directly
/// on a file whose definitions sit inside a `functions { }` wrapper, it
/// treats that block as one headerless definition and skips right over its
/// contents. Parsing unwraps the block (and drops any `library { }`) exactly
/// as a real build does. The offset is still taken from the raw file, since
/// that is what the editor opens.
fn defines_function(path: &Path, source: &str, func: &str) -> bool {
    let flat;
    let text = if path.extension().and_then(|e| e.to_str())
        == Some(laplace::parser::laplacelib::LAPLACELIB_EXTENSION)
    {
        let Ok(parsed) = laplace::parser::laplacelib::parse(path, source) else {
            return false;
        };
        flat = parsed.body;
        flat.as_str()
    } else {
        source
    };

    laplace::parser::signatures::extract_signatures(text)
        .iter()
        .any(|s| s.name == func)
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


#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_plain_stan_package_files_definitions_are_found() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), "gps.stan", "real f(real x) {\n  return x;\n}\n");
        assert!(defines_function(&path, &fs::read_to_string(&path).unwrap(), "f"));
        assert!(!defines_function(&path, &fs::read_to_string(&path).unwrap(), "g"));
    }

    #[test]
    fn definitions_inside_a_laplacelib_functions_wrapper_are_found() {
        // The wrapper is why this needs `laplacelib::parse`: run straight
        // at the raw file, `extract_signatures` reads `functions {` as one
        // headerless definition and never looks inside it.
        let tmp = tempfile::tempdir().unwrap();
        let source = "library {\n  import other\n}\n\nfunctions {\n  real f(real x) {\n    return x;\n  }\n}\n";
        let path = write(tmp.path(), "lib.laplacelib", source);
        assert!(defines_function(&path, source, "f"));
        assert!(!defines_function(&path, source, "g"));
    }

    #[test]
    fn bare_definitions_in_a_laplacelib_are_found_too() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "library {\n  import other\n}\n\nreal f(real x) {\n  return x;\n}\n";
        let path = write(tmp.path(), "lib.laplacelib", source);
        assert!(defines_function(&path, source, "f"));
    }

    #[test]
    fn go_to_definition_points_into_the_laplacelib_file_itself() {
        let tmp = tempfile::tempdir().unwrap();
        // Offsets must land in the file the editor opens, not in the
        // stripped copy `laplacelib::parse` produced to check the name.
        let source = "library {\n  import other\n}\n\nfunctions {\n  real f(real x) {\n    return x;\n  }\n}\n";
        write(tmp.path(), "lib.laplacelib", source);

        let (file, offset) = find_function_definition(tmp.path(), "f").unwrap();
        assert_eq!(file.file_name().unwrap(), "lib.laplacelib");
        assert_eq!(&source[offset..offset + 1], "f");
        assert!(source[..offset].ends_with("real "));
    }
}
