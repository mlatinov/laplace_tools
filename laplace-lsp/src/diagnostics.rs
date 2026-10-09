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

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Range as LspRange, Url};

use laplace::codegen::{self, CodegenError, CodegenOptions};
use laplace::expand::blocks::BlockExpandError;
use laplace::expand::macros::MacroExpandError;
use laplace::parser::blocks::{find_top_level_blocks, BlockKind};
use laplace::parser::identifiers::check_identifiers;
use laplace::parser::laplacelib::{self, LaplaceLibError};
use laplace::parser::macros::find_macros;
use laplace::parser::template::find_templates;
use laplace::parser::library_block::{parse_library_block, ImportStatement, LibraryBlock};
use laplace::resolve::lockfile::{self, Lockfile};

use crate::dialect::Dialect;
use crate::position::{line_starts, offset_to_position};
use crate::workspace::{default_cache_root, find_lockfile, load_installed_package};

/// Stable `Diagnostic::code` values, so a client can act on a diagnostic's
/// *kind* without matching on its prose.
pub mod code {
    /// The `library { }` block itself would not parse.
    pub const LIBRARY_BLOCK: &str = "library-block";
    /// A model-shaped block in a `.laplacelib`.
    pub const FORBIDDEN_BLOCK: &str = "forbidden-block";
    /// An import with no matching entry in `laplace.lock`.
    pub const UNRESOLVED_IMPORT: &str = "unresolved-import";
    /// An `@version` pin that disagrees with `laplace.lock`.
    pub const VERSION_MISMATCH: &str = "version-mismatch";
    /// Locked, but absent from the package cache.
    pub const NOT_INSTALLED: &str = "not-installed";
    /// Present in the cache, but unreadable as a package.
    pub const BAD_PACKAGE: &str = "bad-package";
    /// A `pkg::name` the package does not define: a function, or the
    /// template or macro an `@use` / `@expand` names.
    pub const UNKNOWN_EXPORT: &str = "unknown-export";
    /// A `pkg::name` the package defines but did not mark `pub`.
    pub const PRIVATE_ITEM: &str = "private-item";
    /// A hand-written identifier containing `__`, which laplace reserves
    /// for the names it generates.
    pub const RESERVED_IDENTIFIER: &str = "reserved-identifier";
    /// A `.laplacelib` definition the compiler rejects: a misplaced `pub`,
    /// or a malformed `@template` / `@macro`.
    pub const LIBRARY_ITEM: &str = "library-item";
    /// An `@use` / `@expand` that names a real template or macro but
    /// cannot be expanded: wrong arguments, wrong block, a name collision.
    pub const EXPANSION: &str = "expansion";
    /// A higher-order function (`func(...) -> ...` parameter, `@wait`)
    /// that cannot be specialized.
    pub const HIGHER_ORDER: &str = "higher-order";
    /// Any other `codegen` failure.
    pub const CODEGEN: &str = "codegen";
}

/// The name this file goes by in the compiler's messages. A message that
/// points `--> <source>:line:column` is pointing into the file being
/// edited, as opposed to into an installed package.
pub(crate) const SOURCE_NAME: &str = "<source>";

pub fn compute_diagnostics_for_uri(uri: &Url, source: &str) -> Vec<Diagnostic> {
    let Ok(path) = uri.to_file_path() else {
        return Vec::new();
    };
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let lock = find_lockfile(dir)
        .and_then(|p| lockfile::read_lockfile(&p).ok())
        .unwrap_or_default();
    let cache_root = default_cache_root();

    compute_diagnostics(source, Dialect::for_uri(uri), &lock, &cache_root)
}

pub fn compute_diagnostics(
    source: &str,
    dialect: Dialect,
    lock: &Lockfile,
    cache_root: &Path,
) -> Vec<Diagnostic> {
    let starts = line_starts(source);
    let mut diags = Vec::new();

    // `__` is checked first by both the pipeline and `laplacelib::parse`,
    // and the library check below would only repeat it.
    let identifiers_ok = match check_identifiers(source) {
        Ok(()) => true,
        Err(err) => {
            let range = err.offset..err.offset + err.identifier.len();
            let message = format!("{err}\n  help: {}", err.help());
            diags.push(diagnostic(source, &starts, range, code::RESERVED_IDENTIFIER, message));
            false
        }
    };

    if dialect.is_library() {
        diags.extend(forbidden_block_diagnostics(source, &starts));
        if identifiers_ok {
            diags.extend(library_item_diagnostic(source, &starts));
        }
    }

    let library_block = match parse_library_block(source) {
        Ok(b) => b,
        Err(e) => {
            diags.push(diagnostic(source, &starts, 0..0, code::LIBRARY_BLOCK, e.to_string()));
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
    // Imports whose `@version` pin disagrees with the lockfile, flagged just
    // below. `codegen` reports the same disagreement as
    // `ImportVersionMismatch`; ours names the command that fixes it, so it
    // is the one to keep.
    let mut version_mismatched = std::collections::HashSet::new();

    for import in imports {
        let range = import_range(source, library_block.as_ref().unwrap(), import);

        let Some(locked) = lock.packages.iter().find(|p| p.name == import.name) else {
            diags.push(diagnostic(
                source,
                &starts,
                range,
                code::UNRESOLVED_IMPORT,
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
                    code::VERSION_MISMATCH,
                    format!(
                        "`{}` is pinned to {pin} here, but laplace.lock has {} -- run `laplace update {}`",
                        import.name, locked.version, import.name
                    ),
                ));
                version_mismatched.insert(import.name.clone());
            }
        }

        let package_dir = cache_root.join(&locked.name).join(&locked.version);
        if !package_dir.is_dir() {
            diags.push(diagnostic(
                source,
                &starts,
                range,
                code::NOT_INSTALLED,
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
                diags.push(diagnostic(source, &starts, range, code::BAD_PACKAGE, msg));
                unavailable.insert(import.name.clone());
            }
        }
    }

    let options = CodegenOptions::inline().named(SOURCE_NAME);
    if let Err(err) = codegen::generate_with_options(source, library_block.as_ref(), &installed, &options) {
        let already_reported = match &err {
            CodegenError::MissingImport { package } => unavailable.contains(package),
            CodegenError::ImportVersionMismatch { package, .. } => version_mismatched.contains(package),
            _ => false,
        };
        if !already_reported {
            let range = codegen_error_range(source, library_block.as_ref(), &err);
            let message = without_own_location(&err.to_string());
            diags.push(diagnostic(source, &starts, range, codegen_error_code(&err), message));
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

/// The model-shaped blocks a `.laplacelib` file may not contain. Reported
/// per offending block rather than stopping at the first one, since an
/// editor can show them all at once (`laplace::parser::laplacelib::parse`,
/// compiling a whole package, stops at the first).
///
/// Blocks that start inside a `@template` or `@macro` definition are not
/// blocks of this file, and are skipped the same way `laplacelib::parse`
/// skips them: a macro header ends `in model {`, which reads exactly like
/// a `model` block, and a template body is full of real ones.
///
/// If a definition does not parse, its extent is unknown, and every block
/// after it could be a header misread. Nothing is reported then: the
/// compiler stops at the broken definition too, and
/// [`library_item_diagnostic`] says what is wrong with it.
fn forbidden_block_diagnostics(source: &str, starts: &[usize]) -> Vec<Diagnostic> {
    let (Ok(templates), Ok(macros)) = (find_templates(source, &|_| false), find_macros(source, &|_| false)) else {
        return Vec::new();
    };
    let definitions: Vec<Range<usize>> = templates
        .into_iter()
        .map(|t| t.range)
        .chain(macros.into_iter().map(|m| m.range))
        .collect();
    let in_definition = |offset: usize| definitions.iter().any(|r| r.contains(&offset));

    find_top_level_blocks(source)
        .into_iter()
        .filter(|b| !matches!(b.kind, BlockKind::Library | BlockKind::Functions))
        .filter(|b| !in_definition(b.byte_range.start))
        .map(|b| {
            let keyword = b.kind.keyword();
            diagnostic(
                source,
                starts,
                b.byte_range.start..b.byte_range.start + keyword.len(),
                code::FORBIDDEN_BLOCK,
                format!(
                    "a `.laplacelib` file cannot contain a `{keyword}` block -- a library provides \
                     functions to a model, it is not a model itself (move this into the `.laplace` \
                     file that uses the library)"
                ),
            )
        })
        .collect()
}

/// What `laplacelib::parse` has to say about a library's own definitions:
/// a `pub` in the wrong place, a malformed `@template` or `@macro`. At
/// most one, since the parser stops at the first.
///
/// Forbidden blocks and the `library { }` block are skipped: both are
/// reported elsewhere, and in more detail. `__` is the caller's.
fn library_item_diagnostic(source: &str, starts: &[usize]) -> Option<Diagnostic> {
    let err = laplacelib::parse(Path::new(SOURCE_NAME), source).err()?;
    let (line, column) = match &err {
        LaplaceLibError::Visibility { line, column, .. }
        | LaplaceLibError::Macro { line, column, .. }
        | LaplaceLibError::Template { line, column, .. } => (*line, *column),
        LaplaceLibError::ForbiddenBlock { .. }
        | LaplaceLibError::Imports { .. }
        | LaplaceLibError::ReservedIdentifier { .. } => return None,
    };
    let range = line_range_from(source, line_col_to_offset(source, line, column));
    let message = without_own_location(&err.to_string());
    Some(diagnostic(source, starts, range, code::LIBRARY_ITEM, message))
}

/// Which [`code`] a `codegen` failure reports under.
fn codegen_error_code(err: &CodegenError) -> &'static str {
    match err {
        CodegenError::FunctionNotExported { .. } | CodegenError::UnknownPackageReference { .. } => {
            code::UNKNOWN_EXPORT
        }
        CodegenError::Expand(e) if matches!(**e, BlockExpandError::UnknownTemplate { .. }) => {
            code::UNKNOWN_EXPORT
        }
        CodegenError::ExpandMacro(e) if matches!(**e, MacroExpandError::UnknownMacro { .. }) => {
            code::UNKNOWN_EXPORT
        }
        CodegenError::ItemIsPrivate { .. } => code::PRIVATE_ITEM,
        CodegenError::Expand(e) if matches!(**e, BlockExpandError::TemplateIsPrivate { .. }) => {
            code::PRIVATE_ITEM
        }
        CodegenError::ExpandMacro(e) if matches!(**e, MacroExpandError::MacroIsPrivate { .. }) => {
            code::PRIVATE_ITEM
        }
        CodegenError::Template(_)
        | CodegenError::Expand(_)
        | CodegenError::Macro(_)
        | CodegenError::ExpandMacro(_) => code::EXPANSION,
        CodegenError::Monomorphize(_) => code::HIGHER_ORDER,
        _ => code::CODEGEN,
    }
}

fn codegen_error_range(source: &str, library_block: Option<&LibraryBlock>, err: &CodegenError) -> Range<usize> {
    let calls = laplace::codegen::rename::find_qualified_calls(source);
    let import_of = |package: &str| {
        library_block.and_then(|b| {
            b.imports
                .iter()
                .find(|i| i.name == package)
                .map(|i| import_range(source, b, i))
        })
    };

    match err {
        CodegenError::MissingImport { package }
        | CodegenError::ExportedFunctionMissing { package, .. }
        // A version the `library { }` block pins against a lockfile that
        // pins something else is a disagreement about one import line, so
        // that line is exactly where it belongs.
        | CodegenError::ImportVersionMismatch { package, .. }
        // A dependency of an imported package, and a package reaching for
        // someone else's dependency, are both problems in a package's own
        // source: the nearest thing to them in *this* file is the import
        // that pulled that package in.
        | CodegenError::MissingDependency { package, .. }
        | CodegenError::DuplicatePackage { package } => import_of(package).unwrap_or(0..0),
        CodegenError::UndeclaredPackageReference { in_package, .. } => {
            import_of(in_package).unwrap_or(0..0)
        }
        CodegenError::FunctionNotExported { package, func }
        | CodegenError::UnknownPackageReference { package, func }
        | CodegenError::ItemIsPrivate { package, func, .. }
        // Calling a density without `~` is a mistake at the call site, not
        // in the import: point at the call the user has to rewrite.
        | CodegenError::DensityCalledWithoutTilde { package, func, .. } => calls
            .iter()
            .find(|c| &c.package == package && &c.func == func)
            .map(|c| c.range.clone())
            .unwrap_or(0..0),
        // Collisions between two packages: neither import is more at fault
        // than the other, so point at whichever comes first in this file.
        CodegenError::DuplicateMangledName {
            first_package,
            second_package,
            ..
        }
        | CodegenError::FunctionFileCollision {
            first_package,
            second_package,
            ..
        } => import_of(first_package)
            .or_else(|| import_of(second_package))
            .unwrap_or(0..0),
        // Expansion and specialization errors already name the line they
        // are about. When that line is in a package rather than here, the
        // message keeps its `-->` and the start of the file stands in.
        CodegenError::Template(_)
        | CodegenError::Expand(_)
        | CodegenError::Macro(_)
        | CodegenError::ExpandMacro(_)
        | CodegenError::Monomorphize(_) => {
            let offset = own_location(source, &err.to_string()).or(match err {
                CodegenError::Template(e) => Some(e.offset()),
                CodegenError::Macro(e) => Some(e.offset()),
                _ => None,
            });
            offset.map_or(0..0, |o| line_range_from(source, o))
        }
    }
}

/// The byte offset a compiler message points at with
/// `--> <source>:line:column`, if it points into this file.
fn own_location(source: &str, message: &str) -> Option<usize> {
    let prefix = format!("--> {SOURCE_NAME}:");
    let rest = message.lines().find_map(|l| l.trim_start().strip_prefix(prefix.as_str()))?;
    let (line, column) = rest.trim_end().split_once(':')?;
    Some(line_col_to_offset(source, line.parse().ok()?, column.parse().ok()?))
}

/// The message without its `--> <source>:...` line: the diagnostic's range
/// already says where. A `-->` into a package is kept, since nothing else
/// says where that is.
fn without_own_location(message: &str) -> String {
    let prefix = format!("--> {SOURCE_NAME}:");
    message
        .lines()
        .filter(|l| !l.trim_start().starts_with(prefix.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The byte offset of a 1-indexed line and character column, as
/// `laplace::parser::origin::line_col` counts them. Clamped to the line.
fn line_col_to_offset(source: &str, line: usize, column: usize) -> usize {
    let starts = line_starts(source);
    let Some(&start) = starts.get(line.saturating_sub(1)) else {
        return source.len();
    };
    let text = source[start..].split('\n').next().unwrap_or("");
    start
        + text
            .char_indices()
            .nth(column.saturating_sub(1))
            .map_or(text.len(), |(i, _)| i)
}

/// From `offset` to the end of its line, trailing whitespace excluded: the
/// `@use ...;`, `@expand ...;` or header the compiler pointed at.
fn line_range_from(source: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(source.len());
    let rest = source[offset..].split('\n').next().unwrap_or("");
    offset..offset + rest.trim_end().len()
}

fn diagnostic(
    source: &str,
    starts: &[usize],
    range: Range<usize>,
    code: &str,
    message: String,
) -> Diagnostic {
    let start = offset_to_position(source, starts, range.start.min(source.len()));
    let end = offset_to_position(source, starts, range.end.min(source.len()));
    Diagnostic {
        range: LspRange { start, end },
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String(code.to_string())),
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
            root: vec!["gps".to_string()],
            packages: vec![LockedPackage {
                name: "gps".to_string(),
                version: "1.0.0".to_string(),
                checksum: "sha256:whatever".to_string(),
                source: "registry".to_string(),
                dependencies: Vec::new(),
            }],
        }
    }

    #[test]
    fn clean_file_with_installed_package_has_no_diagnostics() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_gps(tmp.path());
        let source = "library {\n  import gps\n}\nmodel {\n  real y = gps::rbf_cov([1.0], 1.0, 1.0)[1, 1];\n}\n";
        assert!(compute_diagnostics(source, Dialect::Laplace, &lock, tmp.path()).is_empty());
    }

    #[test]
    fn import_missing_from_lockfile_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "library {\n  import gps\n}\nmodel {\n}\n";
        let diags = compute_diagnostics(source, Dialect::Laplace, &Lockfile::default(), tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("laplace add gps"));
    }

    #[test]
    fn locked_but_not_installed_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
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
        let source = "library {\n  import gps\n}\nmodel {\n}\n";
        let diags = compute_diagnostics(source, Dialect::Laplace, &lock, tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("laplace install"));
    }

    #[test]
    fn pinned_version_mismatch_against_lockfile_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_gps(tmp.path());
        let source = "library {\n  import gps@2.0.0\n}\nmodel {\n}\n";
        let diags = compute_diagnostics(source, Dialect::Laplace, &lock, tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("pinned to 2.0.0"));
        assert!(diags[0].message.contains("laplace.lock has 1.0.0"));
    }

    #[test]
    fn unresolved_pkg_func_call_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_gps(tmp.path());
        let source = "library {\n  import gps\n}\nmodel {\n  real y = gps::matern_cov(1.0);\n}\n";
        let diags = compute_diagnostics(source, Dialect::Laplace, &lock, tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("matern_cov"));
        assert!(diags[0].message.contains("defines no `matern_cov`"));
    }

    #[test]
    fn call_to_an_unimported_package_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "model {\n  real y = other::helper(1.0);\n}\n";
        let diags = compute_diagnostics(source, Dialect::Laplace, &Lockfile::default(), tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("was not imported"));
    }

    // --- `.laplacelib` -----------------------------------------------------

    #[test]
    fn a_library_of_functions_and_imports_alone_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_gps(tmp.path());
        // Bare definitions, no `functions { }` wrapper and no model-shaped
        // blocks: exactly what the library dialect exists to allow.
        let source = "library {\n  import gps\n}\n\nmatrix k(vector x) {\n  return gps::rbf_cov(x, 1.0, 1.0);\n}\n";
        assert!(compute_diagnostics(source, Dialect::Library, &lock, tmp.path()).is_empty());
    }

    #[test]
    fn a_model_shaped_block_in_a_library_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "real f(real x) {\n  return x;\n}\n\nmodel {\n  y ~ normal(0, 1);\n}\n";
        let diags = compute_diagnostics(source, Dialect::Library, &Lockfile::default(), tmp.path());

        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("cannot contain a `model` block"));
        // Pointed at the keyword itself, not the whole block or the file.
        assert_eq!(diags[0].range.start.line, 4);
        assert_eq!(diags[0].range.start.character, 0);
        assert_eq!(diags[0].range.end.character, 5);
    }

    #[test]
    fn every_forbidden_block_is_reported_not_only_the_first() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "data {\n  int N;\n}\nparameters {\n  real mu;\n}\n";
        let diags = compute_diagnostics(source, Dialect::Library, &Lockfile::default(), tmp.path());

        assert_eq!(diags.len(), 2);
        assert!(diags[0].message.contains("`data` block"));
        assert!(diags[1].message.contains("`parameters` block"));
    }

    #[test]
    fn functions_and_library_blocks_are_allowed_in_a_library() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_gps(tmp.path());
        let source = "library {\n  import gps\n}\nfunctions {\n  matrix k(vector x) {\n    return gps::rbf_cov(x, 1.0, 1.0);\n  }\n}\n";
        assert!(compute_diagnostics(source, Dialect::Library, &lock, tmp.path()).is_empty());
    }

    #[test]
    fn the_same_blocks_are_fine_in_a_laplace_file() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "data {\n  int N;\n}\nmodel {\n}\n";
        assert!(compute_diagnostics(source, Dialect::Laplace, &Lockfile::default(), tmp.path()).is_empty());
    }

    #[test]
    fn a_library_still_gets_the_ordinary_import_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "library {\n  import gps\n}\nreal f(real x) {\n  return x;\n}\n";
        let diags = compute_diagnostics(source, Dialect::Library, &Lockfile::default(), tmp.path());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("laplace add gps"));
    }

    fn codes(diags: &[Diagnostic]) -> Vec<String> {
        diags
            .iter()
            .map(|d| match &d.code {
                Some(NumberOrString::String(c)) => c.clone(),
                other => panic!("expected a string code, got {other:?}"),
            })
            .collect()
    }

    /// Every diagnostic carries a code, so the client can filter on kind
    /// rather than on message text.
    #[test]
    fn import_diagnostics_are_coded() {
        let tmp = tempfile::tempdir().unwrap();

        let source = "library {\n  import gps\n}\nmodel {\n}\n";
        let diags = compute_diagnostics(source, Dialect::Laplace, &Lockfile::default(), tmp.path());
        assert_eq!(codes(&diags), vec![code::UNRESOLVED_IMPORT]);

        let lock = install_gps(tmp.path());
        let pinned = "library {\n  import gps@2.0.0\n}\nmodel {\n}\n";
        let diags = compute_diagnostics(pinned, Dialect::Laplace, &lock, tmp.path());
        assert_eq!(codes(&diags), vec![code::VERSION_MISMATCH]);

        let bad_call = "library {\n  import gps\n}\nmodel {\n  real y = gps::matern_cov(1.0);\n}\n";
        let diags = compute_diagnostics(bad_call, Dialect::Laplace, &lock, tmp.path());
        assert_eq!(codes(&diags), vec![code::UNKNOWN_EXPORT]);
    }

    // --- patch 1 ----------------------------------------------------------

    /// A `.laplacelib` package `stats` with templates and macros, in the
    /// shape the compiler's own acceptance tests use.
    const STATS_LIB: &str = "\
pub @template ncp($name: ident, $N: expr) {
  parameters {
    vector[$N] ${name}_raw;
    real<lower=0> ${name}_sigma;
  }
  transformed parameters {
    vector[$N] $name = ${name}_sigma * ${name}_raw;
  }
  model {
    ${name}_raw ~ std_normal();
  }
}

@template internal_only($n: ident) {
  parameters {
    real $n;
  }
}

pub @macro priors(each $p: ident, $dist: expr) : stmt in model {
  $p ~ $dist;
}

pub real half(real x) {
  return x / 2;
}
";

    /// Installs `stats` (above) into `cache_root/stats/1.0.0/`.
    fn install_stats(cache_root: &Path) -> Lockfile {
        let dir = cache_root.join("stats").join("1.0.0");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("laplace.toml"), "name = \"stats\"\nversion = \"1.0.0\"\n").unwrap();
        fs::write(dir.join("stats.laplacelib"), STATS_LIB).unwrap();

        Lockfile {
            root: vec!["stats".to_string()],
            packages: vec![LockedPackage {
                name: "stats".to_string(),
                version: "1.0.0".to_string(),
                checksum: "sha256:whatever".to_string(),
                source: "registry".to_string(),
                dependencies: Vec::new(),
            }],
        }
    }

    #[test]
    fn a_library_using_all_of_patch_1_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_gps(tmp.path());
        let source = "\
pub real mean_of(vector x) {
  return sum(x) / num_elements(x);
}

pub matrix expand_rows(vector x, func(real) -> vector f) {
  matrix[num_elements(x), @wait(f).size] out;
  for (i in 1:num_elements(x)) {
    @wait(f) row = f(x[i]);
    out[i] = row';
  }
  return out;
}

vector[2] to_pair(real x) {
  return [x, x * 2]';
}

pub @template ncp($name: ident, $N: expr) {
  parameters {
    vector[$N] ${name}_raw;
  }
  model {
    ${name}_raw ~ std_normal();
  }
}

pub @macro priors(each $p: ident, $dist: expr) : stmt in model {
  $p ~ $dist;
}
";
        let diags = compute_diagnostics(source, Dialect::Library, &lock, tmp.path());
        assert!(diags.is_empty(), "expected none, got {:?}", codes(&diags));
    }

    /// `in model {` puts `model {` at brace depth 0, where
    /// `find_top_level_blocks` alone cannot tell it from a real block.
    #[test]
    fn a_macro_headers_in_clause_is_not_a_forbidden_block() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "\
pub @macro z($p: ident) : stmt in transformed parameters, generated quantities {
  real ${p}_z = 0;
}
";
        let diags = compute_diagnostics(source, Dialect::Library, &Lockfile::default(), tmp.path());
        assert!(diags.is_empty(), "expected none, got {:?}", codes(&diags));
    }

    /// A `~` cannot go in `transformed parameters`, so the compiler rejects
    /// this macro. That is the one thing to report: its `model {` is still
    /// part of a header, not a block.
    #[test]
    fn a_macro_with_an_impossible_target_is_flagged_as_the_macro() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "\
pub @macro z($p: ident) : stmt in transformed parameters, model {
  $p ~ std_normal();
}
";
        let diags = compute_diagnostics(source, Dialect::Library, &Lockfile::default(), tmp.path());
        assert_eq!(codes(&diags), vec![code::LIBRARY_ITEM], "{diags:?}");
    }

    #[test]
    fn a_real_model_block_after_a_macro_is_still_forbidden() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "\
pub @macro z($p: ident) : stmt in model {
  $p ~ std_normal();
}

model {
}
";
        let diags = compute_diagnostics(source, Dialect::Library, &Lockfile::default(), tmp.path());
        assert_eq!(codes(&diags), vec![code::FORBIDDEN_BLOCK]);
        assert_eq!(diags[0].range.start.line, 4);
    }

    #[test]
    fn a_model_using_templates_and_macros_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_stats(tmp.path());
        let source = "\
library {
  import stats
}

@use stats::ncp(theta, K);

data {
  int K;
}
parameters {
  real alpha;
  real beta;
}
model {
  @expand stats::priors([alpha, beta], normal(0, 1));
}
";
        let diags = compute_diagnostics(source, Dialect::Laplace, &lock, tmp.path());
        assert!(diags.is_empty(), "expected none, got {:?}", diags);
    }

    #[test]
    fn an_unknown_template_is_flagged_on_its_use() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_stats(tmp.path());
        let source = "library {\n  import stats\n}\n\n@use stats::nope(theta, K);\n";
        let diags = compute_diagnostics(source, Dialect::Laplace, &lock, tmp.path());

        assert_eq!(codes(&diags), vec![code::UNKNOWN_EXPORT]);
        assert!(diags[0].message.contains("no template `nope`"), "{}", diags[0].message);
        assert_eq!(diags[0].range.start.line, 4);
        assert_eq!(diags[0].range.start.character, 0);
        // The range says where; the message need not.
        assert!(!diags[0].message.contains(SOURCE_NAME), "{}", diags[0].message);
    }

    #[test]
    fn a_private_template_is_flagged_as_private() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_stats(tmp.path());
        let source = "library {\n  import stats\n}\n\n@use stats::internal_only(z);\n";
        let diags = compute_diagnostics(source, Dialect::Laplace, &lock, tmp.path());
        assert_eq!(codes(&diags), vec![code::PRIVATE_ITEM]);
        assert_eq!(diags[0].range.start.line, 4);
    }

    #[test]
    fn a_macro_expanded_in_the_wrong_block_is_flagged_there() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = install_stats(tmp.path());
        let source = "\
library {
  import stats
}
parameters {
  real alpha;
}
generated quantities {
  @expand stats::priors([alpha], normal(0, 1));
}
";
        let diags = compute_diagnostics(source, Dialect::Laplace, &lock, tmp.path());
        assert_eq!(codes(&diags), vec![code::EXPANSION]);
        assert_eq!(diags[0].range.start.line, 7);
    }

    #[test]
    fn a_double_underscore_identifier_is_flagged_at_the_identifier() {
        let tmp = tempfile::tempdir().unwrap();
        let source = "data {\n  int my__n;\n}\n";
        let diags = compute_diagnostics(source, Dialect::Laplace, &Lockfile::default(), tmp.path());

        assert_eq!(codes(&diags), vec![code::RESERVED_IDENTIFIER]);
        assert_eq!(diags[0].range.start.line, 1);
        assert_eq!(diags[0].range.start.character, 6);
        assert_eq!(diags[0].range.end.character, 11);
        assert!(diags[0].message.contains("my_n"), "{}", diags[0].message);
    }

    #[test]
    fn a_malformed_macro_in_a_library_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        // No `: stmt in <blocks>` clause.
        let source = "real f(real x) {\n  return x;\n}\n\npub @macro bad($p: ident) {\n  $p ~ std_normal();\n}\n";
        let diags = compute_diagnostics(source, Dialect::Library, &Lockfile::default(), tmp.path());
        assert_eq!(codes(&diags), vec![code::LIBRARY_ITEM], "{diags:?}");
        assert_eq!(diags[0].range.start.line, 4);
    }
}
