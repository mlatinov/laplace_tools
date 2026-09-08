//! Live diagnostics: wires `laplace::codegen` (which already validates
//! imports/exports while compiling) plus direct lockfile/cache checks, so
//! editing an import that doesn't resolve is flagged without running
//! `laplace build`.
//!
//! Full Stan-level syntax/type checking (missing semicolons, unknown types,
//! incompatible operand types, ...) is deliberately NOT run here: it
//! requires shelling out to `stanc`, an external binary that isn't
//! guaranteed to be installed. That part is best-effort and lives in
//! `crate::stanc` instead, which silently contributes no diagnostics when
//! `stanc` can't be found. This module's checks are always-on because they
//! only need `codegen` + the lockfile, never an external process.

use std::ops::Range;
use std::path::Path;

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range as LspRange, Url};

use laplace::codegen::{self, CodegenError};
use laplace::parser::library_block::{parse_library_block, ImportStatement, LibraryBlock};
use laplace::resolve::lockfile::{self, Lockfile};

use crate::position::{line_starts, offset_to_position};
use crate::workspace::{default_cache_root, find_lockfile, load_installed_package};

pub fn compute_diagnostics_for_uri(uri: &Url, source: &str) -> Vec<Diagnostic> {
    let Ok(path) = uri.to_file_path() else {
        return Vec::new();
    };
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let lock = find_lockfile(dir)
        .and_then(|p| lockfile::read_lockfile(&p).ok())
        .unwrap_or_default();
    let cache_root = default_cache_root();

    compute_diagnostics(source, &lock, &cache_root)
}

pub fn compute_diagnostics(source: &str, lock: &Lockfile, cache_root: &Path) -> Vec<Diagnostic> {
    let starts = line_starts(source);
    let mut diags = Vec::new();

    let library_block = match parse_library_block(source) {
        Ok(b) => b,
        Err(e) => {
            diags.push(diagnostic(source, &starts, 0..0, e.to_string()));
            return diags;
        }
    };

    let imports: &[ImportStatement] = library_block
        .as_ref()
        .map(|b| b.imports.as_slice())
        .unwrap_or(&[]);

    let mut installed = Vec::new();
    // Imports already flagged as unavailable below (not in the lock, or not
    // installed) -- suppress `codegen`'s redundant `MissingImport` for the
    // same package once we've already explained *why* it's missing.
    let mut unavailable = std::collections::HashSet::new();

    for import in imports {
        let range = import_range(source, library_block.as_ref().unwrap(), import);

        let Some(locked) = lock.packages.iter().find(|p| p.name == import.name) else {
            diags.push(diagnostic(
                source,
                &starts,
                range,
                format!(
                    "`{}` is imported but not recorded in laplace.lock -- run `laplace add {}`",
                    import.name, import.name
                ),
            ));
            unavailable.insert(import.name.clone());
            continue;
        };

        if let Some(pin) = &import.version {
            if pin != &locked.version {
                diags.push(diagnostic(
                    source,
                    &starts,
                    range.clone(),
                    format!(
                        "`{}` is pinned to {pin} here, but laplace.lock has {} -- run `laplace update {}`",
                        import.name, locked.version, import.name
                    ),
                ));
            }
        }

        let package_dir = cache_root.join(&locked.name).join(&locked.version);
        if !package_dir.is_dir() {
            diags.push(diagnostic(
                source,
                &starts,
                range,
                format!(
                    "`{}@{}` is locked but not installed -- run `laplace install`",
                    locked.name, locked.version
                ),
            ));
            unavailable.insert(import.name.clone());
            continue;
        }

        match load_installed_package(&package_dir, &locked.name) {
            Ok(pkg) => installed.push(pkg),
            Err(msg) => {
                diags.push(diagnostic(source, &starts, range, msg));
                unavailable.insert(import.name.clone());
            }
        }
    }

    if let Err(err) = codegen::generate(source, library_block.as_ref(), &installed) {
        let already_reported = matches!(&err, CodegenError::MissingImport { package } if unavailable.contains(package));
        if !already_reported {
            let range = codegen_error_range(source, library_block.as_ref(), &err);
            diags.push(diagnostic(source, &starts, range, err.to_string()));
        }
    }

    diags
}

pub(crate) fn import_range(source: &str, block: &LibraryBlock, import: &ImportStatement) -> Range<usize> {
    let body = &source[block.byte_range.clone()];
    let needle = format!("import {}", import.name);
    if let Some(rel) = body.find(&needle) {
        let start = block.byte_range.start + rel;
        return start..start + needle.len();
    }
    block.byte_range.clone()
}

fn codegen_error_range(source: &str, library_block: Option<&LibraryBlock>, err: &CodegenError) -> Range<usize> {
    let calls = laplace::codegen::rename::find_qualified_calls(source);
    match err {
        CodegenError::MissingImport { package } | CodegenError::ExportedFunctionMissing { package, .. } => {
            library_block
                .and_then(|b| {
                    b.imports
                        .iter()
                        .find(|i| &i.name == package)
                        .map(|i| import_range(source, b, i))
                })
                .unwrap_or(0..0)
        }
        CodegenError::FunctionNotExported { package, func }
        | CodegenError::UnknownPackageReference { package, func } => calls
            .iter()
            .find(|c| &c.package == package && &c.func == func)
            .map(|c| c.range.clone())
            .unwrap_or(0..0),
        CodegenError::DuplicateMangledName { .. } => 0..0,
    }
}

fn diagnostic(source: &str, starts: &[usize], range: Range<usize>, message: String) -> Diagnostic {
    let start = offset_to_position(source, starts, range.start.min(source.len()));
    let end = offset_to_position(source, starts, range.end.min(source.len()));
    Diagnostic {
        range: LspRange { start, end },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("laplace".to_string()),
        message,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use laplace::resolve::lockfile::LockedPackage;

    use super::*;

    /// Installs a minimal `gps` package (one exported function, `rbf_cov`)
    /// into `cache_root/gps/1.0.0/` and returns a matching `Lockfile`.
    fn install_gps(cache_root: &Path) -> Lockfile {
        let dir = cache_root.join("gps").join("1.0.0");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("laplace.toml"),
            "name = \"gps\"\nversion = \"1.0.0\"\nexports = [\"rbf_cov\"]\n",
        )
        .unwrap();
        fs::write(
            dir.join("gps.stan"),
            "matrix rbf_cov(vector x, real alpha, real rho) {\n  return x[1] * alpha * rho;\n}\n",
        )
        .unwrap();

        Lockfile {
            packages: vec![LockedPackage {
                name: "gps".to_string(),
                version: "1.0.0".to_string(),
                checksum: "sha256:whatever".to_string(),
                source: "registry".to_string(),
            }],
        }
    }

    #[test]
    fn clean_file_with_installed_package_has_no_diagnostics() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_gps(tmp.path());
        let source = "library {\n  import gps\n}\nmodel {\n  real y = gps::rbf_cov([1.0], 1.0, 1.0)[1, 1];\n}\n";
        assert!(compute_diagnostics(source, &lock, tmp.path()).is_empty());
    }

    #[test]
    fn import_missing_from_lockfile_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "library {\n  import gps\n}\nmodel {\n}\n";
        let diags = compute_diagnostics(source, &Lockfile::default(), tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("laplace add gps"));
    }

    #[test]
    fn locked_but_not_installed_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = Lockfile {
            packages: vec![LockedPackage {
                name: "gps".to_string(),
                version: "1.0.0".to_string(),
                checksum: "sha256:whatever".to_string(),
                source: "registry".to_string(),
            }],
        };
        let source = "library {\n  import gps\n}\nmodel {\n}\n";
        let diags = compute_diagnostics(source, &lock, tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("laplace install"));
    }

    #[test]
    fn pinned_version_mismatch_against_lockfile_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_gps(tmp.path());
        let source = "library {\n  import gps@2.0.0\n}\nmodel {\n}\n";
        let diags = compute_diagnostics(source, &lock, tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("pinned to 2.0.0"));
        assert!(diags[0].message.contains("laplace.lock has 1.0.0"));
    }

    #[test]
    fn unresolved_pkg_func_call_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_gps(tmp.path());
        let source = "library {\n  import gps\n}\nmodel {\n  real y = gps::matern_cov(1.0);\n}\n";
        let diags = compute_diagnostics(source, &lock, tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("matern_cov"));
        assert!(diags[0].message.contains("not in `gps`'s exports"));
    }

    #[test]
    fn call_to_an_unimported_package_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "model {\n  real y = other::helper(1.0);\n}\n";
        let diags = compute_diagnostics(source, &Lockfile::default(), tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("was not imported"));
    }
}
