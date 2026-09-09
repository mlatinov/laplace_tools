//! Best-effort live Stan-level syntax/type checking: if a `stanc` binary can
//! be found, run it against the generated `.stan` text on every debounced
//! diagnostics pass and relocate its errors back onto the original
//! `.laplace` source via `codegen`'s `SourceMap`. This is a bonus on top of
//! `diagnostics`'s always-on import/lockfile checks -- if `stanc` can't be
//! found, this module silently contributes no diagnostics rather than
//! failing; nothing here is required for the LSP to work.
//!
//! A `.laplacelib` file is not a Stan program -- it is a bag of function
//! definitions -- so handing one to `stanc` as-is would report every valid
//! library as a syntax error. [`wrap_library_source`] first rewrites it into
//! the equivalent `.laplace` text (its definitions gathered into a single
//! `functions { }` block, which `stanc` accepts as a program on its own),
//! and the offsets are mapped back through that rewrite as well as through
//! `codegen`'s own `SourceMap`.

use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range as LspRange, Url};

use laplace::codegen::{self, CodegenOptions, GeneratedStan, SourceMap};
use laplace::parser::blocks::{find_top_level_blocks, BlockKind};
use laplace::parser::library_block::{parse_library_block, LibraryBlock};
use laplace::resolve::lockfile::Lockfile;
use laplace::validate::{self, StancDiagnostic, StancSeverity};

use crate::diagnostics::import_range;
use crate::dialect::Dialect;
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

    compute_stanc_diagnostics(stanc, source, Dialect::for_uri(uri), &lock, &cache_root)
}

fn compute_stanc_diagnostics(
    stanc: &Path,
    source: &str,
    dialect: Dialect,
    lock: &Lockfile,
    cache_root: &Path,
) -> Vec<Diagnostic> {
    // What codegen actually compiles. For a `.laplacelib` that is a rewritten
    // copy of the file, so every offset coming back out has to be mapped
    // through `wrapper` before it means anything in the user's buffer.
    let wrapper = match dialect {
        Dialect::Laplace => None,
        Dialect::Library => match wrap_library_source(source) {
            Some(w) => Some(w),
            // A forbidden block, which diagnostics.rs already reports. There
            // is no valid Stan program to build, so contribute nothing here
            // rather than a cascade of confusing syntax errors.
            None => return Vec::new(),
        },
    };
    let compiled = wrapper.as_ref().map_or(source, |w| w.text.as_str());

    let Ok(library_block) = parse_library_block(compiled) else {
        return Vec::new(); // diagnostics::compute_diagnostics already reports this
    };
    // The same block located in the *user's* file: its byte ranges are what
    // `package_fallback` needs to point a diagnostic at an import statement.
    let original_library_block = match parse_library_block(source) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
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

    let Ok((generated, source_map)) = codegen::generate_with_source_map(
        compiled,
        library_block.as_ref(),
        &installed,
        &CodegenOptions::inline(),
    ) else {
        return Vec::new(); // codegen error; diagnostics.rs already flags it
    };

    let Some(raw) = run_stanc(stanc, &generated.source) else {
        return Vec::new();
    };

    let starts = line_starts(source);
    let gen_starts = line_starts(&generated.source);
    let mapper = Mapper {
        source_map: &source_map,
        wrapper: wrapper.as_ref(),
    };

    validate::parse_stanc_output(&raw)
        .into_iter()
        .map(|d| {
            to_lsp_diagnostic(
                source,
                &starts,
                &generated,
                &gen_starts,
                &mapper,
                original_library_block.as_ref(),
                d,
            )
        })
        .collect()
}

/// Byte offsets in the text handed to `stanc`, walked all the way back to
/// byte offsets in the file the user is editing: through `codegen`'s
/// `SourceMap` first, then (for a `.laplacelib`) through the wrapping
/// rewrite. `None` at either step means the offset has no position in the
/// user's file -- code spliced in from an imported package, or the
/// boilerplate `functions { }` wrapper itself.
struct Mapper<'a> {
    source_map: &'a SourceMap,
    wrapper: Option<&'a WrappedLibrary>,
}

impl Mapper<'_> {
    fn map(&self, generated_offset: usize) -> Option<usize> {
        let compiled_offset = self.source_map.map(generated_offset)?;
        match self.wrapper {
            None => Some(compiled_offset),
            Some(w) => w.map(compiled_offset),
        }
    }
}

/// A `.laplacelib` file rewritten as the equivalent `.laplace` text, plus
/// the offset mapping back.
struct WrappedLibrary {
    text: String,
    /// `(range in `text`, the offset in the original source that range
    /// starts at)`, in ascending order of the range. Every segment is a
    /// verbatim copy, so the mapping within one is 1:1; text not covered by
    /// any segment is boilerplate this module synthesized.
    segments: Vec<(Range<usize>, usize)>,
}

impl WrappedLibrary {
    fn map(&self, offset: usize) -> Option<usize> {
        let idx = self.segments.partition_point(|(r, _)| r.end <= offset);
        let (range, origin) = self.segments.get(idx)?;
        (offset >= range.start).then(|| origin + (offset - range.start))
    }
}

/// Rewrite a `.laplacelib` file into the `.laplace` text that means the same
/// thing: its `library { }` blocks first, then every function definition it
/// contains gathered into one `functions { }` block -- exactly the shape
/// `laplace::parser::laplacelib::parse` produces for a consumer, and a
/// program `stanc` will accept on its own.
///
/// Definitions are moved rather than copied in place, since a file may
/// legally interleave them with its `library { }` block, and Stan allows
/// only one `functions { }` block. `None` if the file contains a block the
/// library dialect forbids -- there is no meaningful Stan program to build
/// from it, and `diagnostics` reports that on its own.
fn wrap_library_source(source: &str) -> Option<WrappedLibrary> {
    let blocks = find_top_level_blocks(source);
    if blocks
        .iter()
        .any(|b| !matches!(b.kind, BlockKind::Library | BlockKind::Functions))
    {
        return None;
    }

    let mut wrapped = WrappedLibrary {
        text: String::with_capacity(source.len() + 16),
        segments: Vec::new(),
    };

    for block in blocks.iter().filter(|b| b.kind == BlockKind::Library) {
        wrapped.copy(source, block.byte_range.clone());
        wrapped.text.push('\n');
    }

    wrapped.text.push_str("functions {\n");
    let mut cursor = 0usize;
    for block in &blocks {
        wrapped.copy(source, cursor..block.byte_range.start);
        // A `functions { }` wrapper is unwrapped (its contents are kept, its
        // bookends dropped); a `library { }` block was already emitted above.
        if block.kind == BlockKind::Functions {
            wrapped.copy(source, block.body_range.clone());
        }
        cursor = block.byte_range.end;
    }
    wrapped.copy(source, cursor..source.len());
    wrapped.text.push_str("\n}\n");

    Some(wrapped)
}

impl WrappedLibrary {
    /// Append `range` of `source` verbatim, recording where it came from.
    fn copy(&mut self, source: &str, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(&source[range.clone()]);
        self.segments.push((start..self.text.len(), range.start));
    }
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
    mapper: &Mapper,
    library_block: Option<&LibraryBlock>,
    d: StancDiagnostic,
) -> Diagnostic {
    let severity = match d.severity {
        StancSeverity::Error => DiagnosticSeverity::ERROR,
        StancSeverity::Warning => DiagnosticSeverity::WARNING,
    };

    let mapped = gen_starts.get(d.line.saturating_sub(1)).and_then(|&line_start| {
        let span_len = d.column_end.saturating_sub(d.column_start).max(1);
        mapper
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
        let diags = compute_stanc_diagnostics(&stanc, source, Dialect::Laplace, &Lockfile::default(), tmp.path());
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
        let diags = compute_stanc_diagnostics(&stanc, source, Dialect::Laplace, &Lockfile::default(), tmp.path());

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
        let diags = compute_stanc_diagnostics(&stanc, source, Dialect::Laplace, &Lockfile::default(), tmp.path());
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
            root: vec!["gps".to_string()],
            packages: vec![LockedPackage {
                name: "gps".to_string(),
                version: "1.0.0".to_string(),
                checksum: "sha256:whatever".to_string(),
                source: "registry".to_string(),
                dependencies: Vec::new(),
            }],
        };

        let source = "library {\n  import gps\n}\nmodel {\n  real y = gps::rbf_cov([1.0], 1.0, 1.0)[1, 1];\n}\n";

        // Figure out (by generating once, outside the fake stanc's control)
        // which generated line the spliced `gps` code landed on, so the
        // fake `stanc` output can point at a real line inside it.
        let block = parse_library_block(source).unwrap();
        let installed = vec![load_installed_package(&pkg_dir, "gps").unwrap()];
        let (generated, _) = codegen::generate_with_source_map(source, block.as_ref(), &installed, &CodegenOptions::inline()).unwrap();
        let pkg_line = *generated.package_line_ranges[0].lines.start();

        let stanc_output = format!(
            "Semantic error in 'model.stan', line {pkg_line}, column 0 to column 1:
   -------------------------------------------------
   -------------------------------------------------

Something wrong in the package's own code.
"
        );
        let stanc = fake_stanc(tmp.path(), 1, &stanc_output);
        let diags = compute_stanc_diagnostics(&stanc, source, Dialect::Laplace, &lock, tmp.path());

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
        let diags = compute_stanc_diagnostics(&stanc, source, Dialect::Laplace, &Lockfile::default(), tmp.path());
        assert!(diags.is_empty());
    }

    // --- `.laplacelib` -----------------------------------------------------

    #[test]
    fn a_library_of_bare_definitions_is_wrapped_into_a_stan_program() {
        let source = "library {\n  import gps\n}\n\nreal f(real x) {\n  return x;\n}\n";
        let wrapped = wrap_library_source(source).unwrap();

        // The import block survives (codegen still has to strip it and
        // rename `gps::` calls), and the definition is now inside a
        // `functions { }` block -- a program stanc will accept.
        assert!(wrapped.text.contains("library {\n  import gps\n}"));
        assert!(wrapped.text.contains("functions {"));
        let body_at = wrapped.text.find("real f(real x)").unwrap();
        assert!(wrapped.text[..body_at].contains("functions {"));
        assert!(wrapped.text.trim_end().ends_with('}'));
    }

    #[test]
    fn an_existing_functions_wrapper_is_unwrapped_rather_than_nested() {
        let source = "functions {\n  real f(real x) {\n    return x;\n  }\n}\n";
        let wrapped = wrap_library_source(source).unwrap();
        assert_eq!(wrapped.text.matches("functions {").count(), 1);
    }

    #[test]
    fn wrapping_maps_every_copied_byte_back_to_where_it_came_from() {
        let source = "library {\n  import gps\n}\n\nreal f(real x) {\n  return x;\n}\n";
        let wrapped = wrap_library_source(source).unwrap();

        // Every segment is a verbatim copy, so a mapped offset must name the
        // very same byte in the user's file.
        for (range, _) in &wrapped.segments {
            for offset in range.clone() {
                let original = wrapped.map(offset).expect("a copied byte maps back");
                assert_eq!(
                    wrapped.text.as_bytes()[offset],
                    source.as_bytes()[original],
                    "offset {offset} mapped to {original}"
                );
            }
        }

        // ... and the synthesized wrapper text maps nowhere.
        let functions_kw = wrapped.text.find("functions {").unwrap();
        assert_eq!(wrapped.map(functions_kw), None);
    }

    #[test]
    fn a_forbidden_block_produces_no_stanc_diagnostics() {
        // `diagnostics::compute_diagnostics` reports the block itself; this
        // pass must not pile a cascade of syntax errors on top of it.
        let tmp = tempfile::tempdir().unwrap();
        let source = "model {\n  y ~ normal(0, 1);\n}\n";
        assert!(wrap_library_source(source).is_none());

        let stanc = fake_stanc(tmp.path(), 1, "should never run");
        let diags = compute_stanc_diagnostics(&stanc, source, Dialect::Library, &Lockfile::default(), tmp.path());
        assert!(diags.is_empty());
    }

    #[test]
    fn a_stanc_error_in_a_library_is_relocated_onto_the_original_source() {
        let tmp = tempfile::tempdir().unwrap();
        // No `functions { }` wrapper, so the compiled text is one line
        // longer than the file: the error stanc reports on generated line 3
        // is on line 2 of what the user is editing.
        let source = "real f(real x) {\n  return x\n}\n";
        let stanc_output = "Syntax error in 'model.stan', line 3, column 2 to column 3, parsing error:
   -------------------------------------------------
     3:    return x
   -------------------------------------------------

Ill-formed statement.
";
        let stanc = fake_stanc(tmp.path(), 1, stanc_output);
        let diags = compute_stanc_diagnostics(&stanc, source, Dialect::Library, &Lockfile::default(), tmp.path());

        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        assert!(d.message.starts_with("Ill-formed statement."));
        // Line 1 (0-indexed) of the original == `  return x`, not line 2.
        assert_eq!(d.range.start.line, 1);
        assert_eq!(d.range.start.character, 2);
    }
}
