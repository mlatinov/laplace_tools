//! Best-effort live Stan-level syntax/type checking: if a `stanc` binary can
//! be found, run it against the generated `.stan` text on every debounced
//! diagnostics pass and relocate its errors back onto the original
//! `.laplace` source via `codegen`'s `SourceMap`. This is a bonus on top of
//! `diagnostics`'s always-on import/lockfile checks -- if `stanc` can't be
//! found, this module silently contributes no diagnostics rather than
//! failing; nothing here is required for the LSP to work.

use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range as LspRange, Url};

use laplace::codegen::{self, GeneratedStan, SourceMap};
use laplace::parser::library_block::{parse_library_block, LibraryBlock};
use laplace::resolve::lockfile::Lockfile;
use laplace::validate::{self, StancDiagnostic, StancSeverity};

use crate::diagnostics::import_range;
use crate::position::{line_starts, offset_to_position};
use crate::workspace::{default_cache_root, find_lockfile, load_installed_package};

pub fn compute_stanc_diagnostics_for_uri(uri: &Url, source: &str) -> Vec<Diagnostic> {
    let Some(stanc) = stanc_path() else {
        return Vec::new();
    };

    let Ok(path) = uri.to_file_path() else {
        return Vec::new();
    };
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let lock = find_lockfile(dir)
        .and_then(|p| laplace::resolve::lockfile::read_lockfile(&p).ok())
        .unwrap_or_default();
    let cache_root = default_cache_root();

    compute_stanc_diagnostics(stanc, source, &lock, &cache_root)
}

fn compute_stanc_diagnostics(stanc: &Path, source: &str, lock: &Lockfile, cache_root: &Path) -> Vec<Diagnostic> {
    let Ok(library_block) = parse_library_block(source) else {
        return Vec::new(); // diagnostics::compute_diagnostics already reports this
    };
    let imports = library_block.as_ref().map(|b| b.imports.as_slice()).unwrap_or(&[]);

    let mut installed = Vec::new();
    for import in imports {
        let Some(locked) = lock.packages.iter().find(|p| p.name == import.name) else {
            return Vec::new(); // unresolved import; diagnostics.rs already flags it
        };
        let package_dir = cache_root.join(&locked.name).join(&locked.version);
        let Ok(pkg) = load_installed_package(&package_dir, &locked.name) else {
            return Vec::new();
        };
        installed.push(pkg);
    }

    let Ok((generated, source_map)) = codegen::generate_with_source_map(source, library_block.as_ref(), &installed)
    else {
        return Vec::new(); // codegen error; diagnostics.rs already flags it
    };

    let Some(raw) = run_stanc(stanc, &generated.source) else {
        return Vec::new();
    };

    let starts = line_starts(source);
    let gen_starts = line_starts(&generated.source);

    validate::parse_stanc_output(&raw)
        .into_iter()
        .map(|d| to_lsp_diagnostic(source, &starts, &generated, &gen_starts, &source_map, library_block.as_ref(), d))
        .collect()
}

/// Runs `stanc` against `stan_source` over stdin, routing the C++ it would
/// generate on success to a throwaway tempfile -- we only want the
/// syntax/type-checking, never actual codegen. `None` if `stanc` couldn't be
/// spawned/piped to at all, or if it succeeded (nothing to report).
fn run_stanc(stanc: &Path, stan_source: &str) -> Option<String> {
    let out_file = tempfile::NamedTempFile::new().ok()?;

    let mut child = Command::new(stanc)
        .arg("--filename-in-msg=model.stan")
        .arg("-o")
        .arg(out_file.path())
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    {
        let mut stdin = child.stdin.take()?;
        stdin.write_all(stan_source.as_bytes()).ok()?;
    }

    let output = child.wait_with_output().ok()?;
    if output.status.success() {
        return None;
    }

    let mut raw = String::from_utf8_lossy(&output.stderr).into_owned();
    if raw.trim().is_empty() {
        raw = String::from_utf8_lossy(&output.stdout).into_owned();
    }
    Some(raw)
}

fn to_lsp_diagnostic(
    source: &str,
    starts: &[usize],
    generated: &GeneratedStan,
    gen_starts: &[usize],
    source_map: &SourceMap,
    library_block: Option<&LibraryBlock>,
    d: StancDiagnostic,
) -> Diagnostic {
    let severity = match d.severity {
        StancSeverity::Error => DiagnosticSeverity::ERROR,
        StancSeverity::Warning => DiagnosticSeverity::WARNING,
    };

    let mapped = gen_starts.get(d.line.saturating_sub(1)).and_then(|&line_start| {
        let span_len = d.column_end.saturating_sub(d.column_start).max(1);
        source_map
            .map(line_start + d.column_start)
            .map(|orig_start| orig_start..(orig_start + span_len).min(source.len()))
    });

    let (range, message) = match mapped {
        Some(r) => (r, d.message),
        None => package_fallback(source, generated, library_block, d.line)
            .map(|(r, package)| (r, format!("in imported package `{package}`: {}", d.message)))
            .unwrap_or((0..0, d.message)),
    };

    Diagnostic {
        range: LspRange {
            start: offset_to_position(source, starts, range.start.min(source.len())),
            end: offset_to_position(source, starts, range.end.min(source.len())),
        },
        severity: Some(severity),
        source: Some("stanc".to_string()),
        message,
        ..Default::default()
    }
}

/// When a `stanc` error lands in code spliced in from an imported package
/// (no position in this file to point at), fall back to that package's
/// `import` statement and note which package the error likely came from --
/// mirroring how `laplace build --validate`'s CLI output already annotates
/// this (`laplace::validate::annotate_with_packages`), just relocated onto
/// the original `.laplace` source instead of the generated text.
fn package_fallback(
    source: &str,
    generated: &GeneratedStan,
    library_block: Option<&LibraryBlock>,
    gen_line: usize,
) -> Option<(Range<usize>, String)> {
    let pkg_range = generated.package_line_ranges.iter().find(|r| r.lines.contains(&gen_line))?;
    let block = library_block?;
    let import = block.imports.iter().find(|i| i.name == pkg_range.package)?;
    Some((import_range(source, block, import), pkg_range.package.clone()))
}

/// Locates a usable `stanc` binary once per process: `$LAPLACE_STANC` if
/// set (matching the `laplace` CLI's own `--validate` flag), else `stanc` on
/// `PATH`, else the newest `bin/stanc` under `~/.cmdstan/cmdstan-*` -- how
/// most Stan users actually have it installed (CmdStan bundles its own
/// `stanc` but doesn't put it on `PATH`). `None` if nothing was found; live
/// type-checking is then silently skipped rather than failing.
fn stanc_path() -> Option<&'static Path> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(locate_stanc).as_deref()
}

fn locate_stanc() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LAPLACE_STANC") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(p) = find_on_path("stanc") {
        return Some(p);
    }
    newest_cmdstan_stanc()
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var).find_map(|dir| {
        let candidate = dir.join(name);
        candidate.is_file().then_some(candidate)
    })
}

fn newest_cmdstan_stanc() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let cmdstan_root = PathBuf::from(home).join(".cmdstan");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&cmdstan_root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("cmdstan-"))
        })
        .collect();
    versions.sort();
    let newest = versions.pop()?;
    let bin = newest.join("bin").join("stanc");
    bin.is_file().then_some(bin)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use laplace::resolve::lockfile::LockedPackage;

    use super::*;

    /// Writes a fake `stanc` shell script that always exits with
    /// `exit_code` and prints `output` to stderr, ignoring its arguments and
    /// stdin -- mirrors `laplace`'s own `tests/cli.rs::fake_stanc`, so these
    /// tests never depend on a real stanc install.
    fn fake_stanc(dir: &Path, exit_code: i32, output: &str) -> PathBuf {
        let script = dir.join("fake_stanc.sh");
        fs::write(
            &script,
            format!("#!/bin/sh\ncat <<'EOF' 1>&2\n{output}\nEOF\nexit {exit_code}\n"),
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(&script, perms).unwrap();
        script
    }

    #[test]
    fn no_diagnostics_when_stanc_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let stanc = fake_stanc(tmp.path(), 0, "");
        let source = "data {\n  int N;\n}\nmodel {\n}\n";
        let diags = compute_stanc_diagnostics(&stanc, source, &Lockfile::default(), tmp.path());
        assert!(diags.is_empty());
    }

    #[test]
    fn a_stanc_error_is_relocated_onto_the_original_source() {
        let tmp = tempfile::tempdir().unwrap();
        // `real y = x[1]` on line 5 of the generated (== original, no
        // imports) source: a missing semicolon.
        let source = "data {\n  vector[1] x;\n}\nmodel {\n  real y = x[1]\n  y ~ normal(0, 1);\n}\n";
        let stanc_output = "Syntax error in 'model.stan', line 5, column 2 to column 3, parsing error:
   -------------------------------------------------
     4:  model {
     5:    real y = x[1]
     6:    y ~ normal(0, 1);
           ^
   -------------------------------------------------

Ill-formed expression.
";
        let stanc = fake_stanc(tmp.path(), 1, stanc_output);
        let diags = compute_stanc_diagnostics(&stanc, source, &Lockfile::default(), tmp.path());

        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(d.source.as_deref(), Some("stanc"));
        assert!(d.message.starts_with("Ill-formed expression."));
        // Line 5 (1-indexed) == line index 4 -- the `real y = x[1]` line.
        assert_eq!(d.range.start.line, 4);
    }

    #[test]
    fn a_warning_gets_warning_severity() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "data {\n  int N;\n}\nmodel {\n}\n";
        let stanc_output = "Warning in 'model.stan', line 1, column 0 to column 4:
   -------------------------------------------------
     1:  data {
         ^
   -------------------------------------------------

Something deprecated.
";
        let stanc = fake_stanc(tmp.path(), 1, stanc_output);
        let diags = compute_stanc_diagnostics(&stanc, source, &Lockfile::default(), tmp.path());
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn error_inside_a_spliced_package_falls_back_to_the_import_statement() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("gps").join("1.0.0");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("laplace.toml"),
            "name = \"gps\"\nversion = \"1.0.0\"\nexports = [\"rbf_cov\"]\n",
        )
        .unwrap();
        fs::write(
            pkg_dir.join("gps.stan"),
            "matrix rbf_cov(vector x, real alpha, real rho) {\n  return x[1] * alpha * rho;\n}\n",
        )
        .unwrap();
        let lock = Lockfile {
            packages: vec![LockedPackage {
                name: "gps".to_string(),
                version: "1.0.0".to_string(),
                checksum: "sha256:whatever".to_string(),
                source: "registry".to_string(),
            }],
        };

        let source = "library {\n  import gps\n}\nmodel {\n  real y = gps::rbf_cov([1.0], 1.0, 1.0)[1, 1];\n}\n";

        // Figure out (by generating once, outside the fake stanc's control)
        // which generated line the spliced `gps` code landed on, so the
        // fake `stanc` output can point at a real line inside it.
        let block = parse_library_block(source).unwrap();
        let installed = vec![load_installed_package(&pkg_dir, "gps").unwrap()];
        let (generated, _) = codegen::generate_with_source_map(source, block.as_ref(), &installed).unwrap();
        let pkg_line = *generated.package_line_ranges[0].lines.start();

        let stanc_output = format!(
            "Semantic error in 'model.stan', line {pkg_line}, column 0 to column 1:
   -------------------------------------------------
   -------------------------------------------------

Something wrong in the package's own code.
"
        );
        let stanc = fake_stanc(tmp.path(), 1, &stanc_output);
        let diags = compute_stanc_diagnostics(&stanc, source, &lock, tmp.path());

        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        assert!(d.message.contains("in imported package `gps`"));
        // Located at the `import gps` statement, not offset 0..0.
        assert!(d.range.start.line > 0 || d.range.start.character > 0);
    }

    #[test]
    fn no_diagnostics_when_imports_dont_resolve() {
        // Left to `diagnostics::compute_diagnostics` to report; the stanc
        // pass shouldn't also run (there's nothing valid to compile).
        let tmp = tempfile::tempdir().unwrap();
        let stanc = fake_stanc(tmp.path(), 1, "should never run");
        let source = "library {\n  import gps\n}\nmodel {\n}\n";
        let diags = compute_stanc_diagnostics(&stanc, source, &Lockfile::default(), tmp.path());
        assert!(diags.is_empty());
    }
}
