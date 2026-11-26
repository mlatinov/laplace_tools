//! Diagnostics for the sampler files, `.lapkernel` and `.lapsampler`.
//!
//! Nothing is checked here: every diagnostic comes from
//! `laplace_sampler_syntax::editor`, the same code `laplace sampler check`
//! runs, so the editor and the command line cannot disagree. This module
//! only finds the project, calls the checker and turns its answers into LSP
//! diagnostics:
//!
//! - **Positions.** The checker reports a 1-based line and a 1-based column
//!   counted in characters (Unicode scalar values). LSP columns are UTF-16
//!   code units, so a character outside the BMP (`𝜎`, say) earlier on the
//!   line counts twice.
//! - **Ranges.** The checker reports where a problem starts, a point. The
//!   range here is the token at that point: an identifier (with `pkg::name`
//!   and a leading `@`), a number, a `.lapkernel` path, or else one
//!   character.
//! - **Related information.** When a `.lapsampler` names a kernel file that
//!   does not check, the message lists that file's own diagnostics on
//!   indented `file:line:column: message` lines. Each one whose file is on
//!   disk is also attached as related information, so it can be clicked.
//!
//! Block coverage (every model parameter in exactly one block) needs the
//! model, so it is not checked here: `laplace sampler check --model` does it.

use std::path::{Path, PathBuf};

use laplace_sampler_syntax::editor::{self, Project, Severity};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, Position, Range, Url,
};

use crate::workspace;

pub mod cursor;
pub mod features;
pub mod model;

/// The `source` shown with each diagnostic.
pub const SOURCE: &str = "laplace sampler";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// A `.lapkernel` kernel.
    Kernel,
    /// A `.lapsampler` sampler.
    Sampler,
}

impl Language {
    pub fn for_path(path: &Path) -> Option<Language> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("lapkernel") => Some(Language::Kernel),
            Some("lapsampler") => Some(Language::Sampler),
            _ => None,
        }
    }

    /// `None` for everything else, `.laplace` and `.laplacelib` included,
    /// and for a URI that isn't a file path.
    pub fn for_uri(uri: &Url) -> Option<Language> {
        uri.to_file_path().ok().and_then(|p| Language::for_path(&p))
    }
}

/// The project a `.lapsampler` in `dir` belongs to: the nearest directory
/// at or above it with a `laplace.lock` (where `laplace sample` would be
/// run), else `dir` itself, so that an `import` gets the checker's own "no
/// laplace.lock" message. Packages come from the CLI's cache
/// (`$LAPLACE_HOME/packages`, by default `~/.laplace/packages`).
pub fn project_for(dir: &Path) -> Project {
    let root = workspace::find_lockfile(dir)
        .and_then(|lock| lock.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| dir.to_path_buf());
    Project {
        dir: root,
        cache_root: laplace::resolve::default_cache_root(),
    }
}

/// Check the document at `path` (its current, possibly unsaved, `text`).
/// Local kernels a `.lapsampler` refers to are read from disk.
pub fn diagnostics(language: Language, path: &Path, text: &str) -> Vec<Diagnostic> {
    let shown = path.display().to_string();
    let found = match language {
        Language::Kernel => editor::check_lapkernel(&shown, text, None),
        Language::Sampler => {
            let dir = path.parent().unwrap_or_else(|| Path::new("."));
            editor::check_lapsampler(&shown, text, dir, &project_for(dir))
        }
    };
    found.iter().map(|d| to_lsp(text, &shown, d)).collect()
}

/// One checker diagnostic as an LSP diagnostic on the document `source`
/// (whose path, as given to the checker, is `path`).
pub fn to_lsp(source: &str, path: &str, d: &editor::Diagnostic) -> Diagnostic {
    let (range, message) = if d.file == path {
        (token_range(source, d.line, d.column), d.message.clone())
    } else {
        // Not expected (the checker reports other files' problems inside a
        // diagnostic at the reference), but never put it at a position
        // in the wrong file.
        (
            Range::default(),
            format!("{}:{}:{}: {}", d.file, d.line, d.column, d.message),
        )
    };
    let related: Vec<DiagnosticRelatedInformation> =
        message.lines().skip(1).filter_map(related_line).collect();
    Diagnostic {
        range,
        severity: Some(match d.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        source: Some(SOURCE.to_string()),
        message,
        related_information: (!related.is_empty()).then_some(related),
        ..Default::default()
    }
}

/// `  <file>:<line>:<column>: <message>`, when `<file>` is on disk.
fn related_line(line: &str) -> Option<DiagnosticRelatedInformation> {
    let (location, message) = line.trim().split_once(": ")?;
    let mut parts = location.rsplitn(3, ':');
    let column: usize = parts.next()?.parse().ok()?;
    let line_no: usize = parts.next()?.parse().ok()?;
    let file = PathBuf::from(parts.next()?);
    if !file.is_absolute() {
        return None;
    }
    let text = std::fs::read_to_string(&file).ok()?;
    Some(DiagnosticRelatedInformation {
        location: Location {
            uri: Url::from_file_path(&file).ok()?,
            range: token_range(&text, line_no, column),
        },
        message: message.to_string(),
    })
}

/// The range of the token starting at the checker's `line`:`column`
/// (1-based; the column counts characters), in LSP positions (0-based;
/// UTF-16 code units). `0, 0` (the whole file) is the start of the file.
pub fn token_range(source: &str, line: usize, column: usize) -> Range {
    if line == 0 {
        return Range::default();
    }
    let lines: Vec<&str> = source.split('\n').collect();
    // A position past the end (a syntax error at end of input) is the end
    // of the last line.
    let (index, column) = if line > lines.len() {
        let last = lines.len() - 1;
        (last, usize::MAX)
    } else {
        (line - 1, column)
    };
    let chars: Vec<char> = lines[index].trim_end_matches('\r').chars().collect();
    let mut start = column.saturating_sub(1).min(chars.len());
    let mut end = token_end(&chars, start);
    if start == chars.len() && start > 0 {
        // At the end of the line: mark its last character.
        start -= 1;
        end = chars.len();
    }
    let utf16 = |n: usize| chars[..n].iter().map(|c| c.len_utf16()).sum::<usize>() as u32;
    let line = index as u32;
    Range {
        start: Position::new(line, utf16(start)),
        end: Position::new(line, utf16(end)),
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Where the token starting at `chars[start]` ends (exclusive).
fn token_end(chars: &[char], start: usize) -> usize {
    let at = |i: usize| chars.get(i).copied();
    let Some(first) = at(start) else {
        return start;
    };
    let ident_from = |mut i: usize| {
        while at(i).is_some_and(is_ident) {
            i += 1;
        }
        i
    };
    // A local kernel path: ./x, ../x or /x, up to ( ) , ; or whitespace, as
    // in the .lapsampler grammar.
    let path = match (first, at(start + 1)) {
        ('.', Some('/')) => true,
        ('.', Some('.')) => at(start + 2) == Some('/'),
        // Not `//` or `/*`, a comment.
        ('/', Some(c)) => c != '/' && c != '*' && !c.is_whitespace(),
        _ => false,
    };
    if path {
        let mut i = start;
        while at(i).is_some_and(|c| !c.is_whitespace() && !"(),;".contains(c)) {
            i += 1;
        }
        return i;
    }
    // An identifier, `pkg::name`, or `@adapt`.
    if is_ident_start(first) || (first == '@' && at(start + 1).is_some_and(is_ident_start)) {
        let mut i = ident_from(start + 1);
        while at(i) == Some(':') && at(i + 1) == Some(':') && at(i + 2).is_some_and(is_ident_start)
        {
            i = ident_from(i + 2);
        }
        return i;
    }
    // A number: -?digits[.digits][e[+-]digits], or .digits.
    let digit = |i: usize| at(i).is_some_and(|c| c.is_ascii_digit());
    let mut i = start;
    if first == '-' && (digit(i + 1) || (at(i + 1) == Some('.') && digit(i + 2))) {
        i += 1;
    }
    if digit(i) || (at(i) == Some('.') && digit(i + 1)) {
        while digit(i) {
            i += 1;
        }
        if at(i) == Some('.') && !matches!(at(i + 1), Some('*') | Some('/')) {
            i += 1;
            while digit(i) {
                i += 1;
            }
        }
        if matches!(at(i), Some('e') | Some('E')) {
            let mut j = i + 1;
            if matches!(at(j), Some('+') | Some('-')) {
                j += 1;
            }
            if digit(j) {
                i = j;
                while digit(i) {
                    i += 1;
                }
            }
        }
        return i;
    }
    start + 1
}

#[cfg(test)]
mod tests;
