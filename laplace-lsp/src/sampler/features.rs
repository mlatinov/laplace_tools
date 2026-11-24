//! Hover, completion, go-to-definition and signature help for `.lapkernel`
//! and `.lapsampler` (Phase 3). What is shown comes from the editor API,
//! from the tables the checker uses: `builtins()`, `combinators()` and
//! `kernel_index()`. This module finds what is under the cursor
//! (`cursor`) and formats it.
//!
//! Functions here are pure (text, offset, index in; answer out). The
//! backend loads the index and converts offsets to LSP positions.

use std::ops::Range;
use std::path::{Path, PathBuf};

use laplace_sampler_syntax::editor::{
    builtins, combinators, kernel_index, Builtin, Combinator, KernelEntry, KernelOrigin,
    KernelSetting, Project,
};

use super::cursor::{self, Token, TokenKind};
use super::{project_for, Language};

/// The kernels a `.lapsampler` can name, and its directory (local kernel
/// paths are relative to it).
pub struct Index {
    pub dir: PathBuf,
    pub kernels: Vec<KernelEntry>,
}

impl Index {
    /// The index for a `.lapsampler` at `path`, in its project
    /// ([`project_for`]). Reads laplace.lock, the installed packages and
    /// the project's `.lapkernel` files: call it off the async executor.
    pub fn for_document(path: &Path) -> Index {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        Index::load(dir, &project_for(dir))
    }

    /// The index for a `.lapsampler` in `dir`, in `project`.
    pub fn load(dir: &Path, project: &Project) -> Index {
        let (kernels, _problems) = kernel_index(project, dir);
        Index {
            dir: dir.to_path_buf(),
            kernels,
        }
    }

    /// The kernel a token names: a built-in or `pkg::name` by name, a path
    /// by the file it resolves to.
    pub fn find(&self, token: &Token) -> Option<&KernelEntry> {
        match token.kind {
            TokenKind::Path => {
                let file = self.dir.join(&token.text).canonicalize().ok()?;
                self.kernels.iter().find(|k| {
                    k.origin == KernelOrigin::Local
                        && k.file.as_ref().and_then(|f| f.canonicalize().ok()) == Some(file.clone())
                })
            }
            TokenKind::Name => self.kernels.iter().find(|k| k.name == token.text),
            TokenKind::Other => None,
        }
    }
}

fn builtin(name: &str) -> Option<Builtin> {
    builtins().into_iter().find(|b| b.name == name)
}

fn combinator(name: &str) -> Option<&'static Combinator> {
    combinators().iter().find(|c| c.name == name)
}

// ---- hover -------------------------------------------------------------------

/// Hover text (Markdown) for the name at `offset`, and the byte range it
/// covers. `index` is needed for a `.lapsampler` only.
pub fn hover(
    language: Language,
    text: &str,
    offset: usize,
    index: Option<&Index>,
) -> Option<(Range<usize>, String)> {
    let masked = cursor::mask_comments(text);
    let token = cursor::token_at(&masked, offset)?;
    let markdown = match language {
        Language::Kernel => {
            if token.kind != TokenKind::Name {
                return None;
            }
            builtin_markdown(&builtin(&token.text)?)
        }
        Language::Sampler => sampler_hover(&masked, &token, index?)?,
    };
    Some((token.range, markdown))
}

fn sampler_hover(masked: &str, token: &Token, index: &Index) -> Option<String> {
    // A setting: `name =` inside a kernel call or adapt().
    if token.kind == TokenKind::Name && cursor::next_char(masked, token.range.end) == Some('=') {
        let line = cursor::kernel_line_start(masked, token.range.start);
        let call = cursor::enclosing_call(masked, line, token.range.start)?;
        if let Some(c) = combinator(&call.callee.text) {
            return c
                .settings
                .contains(&token.text.as_str())
                .then(|| combinator_setting_markdown(c, &token.text));
        }
        let kernel = index.find(&call.callee)?;
        let setting = kernel.settings.iter().find(|s| s.name == token.text)?;
        return Some(setting_markdown(kernel, setting));
    }
    if token.kind == TokenKind::Name && cursor::next_char(masked, token.range.end) == Some('(') {
        if let Some(c) = combinator(&token.text) {
            return Some(combinator_markdown(c));
        }
    }
    Some(kernel_markdown(index.find(token)?))
}

fn fenced(language: &str, lines: &[&str]) -> String {
    format!("```{language}\n{}\n```", lines.join("\n"))
}

pub fn builtin_markdown(b: &Builtin) -> String {
    let mut md = fenced("lapkernel", b.signatures);
    md += &format!("\n\n{}", b.doc);
    if b.rng != "none" {
        md += &format!("\n\n*Random numbers:* {}", b.rng);
    }
    if !b.needs_chain {
        md += "\n\n*Also allowed in tuning defaults.*";
    }
    md
}

pub fn combinator_markdown(c: &Combinator) -> String {
    let mut md = fenced("lapsampler", &[c.signature]);
    md += &format!("\n\n{}", c.doc);
    md
}

fn combinator_setting_markdown(c: &Combinator, name: &str) -> String {
    format!(
        "{}\n\nA setting of `{}()`: {}",
        fenced("lapsampler", &[&format!("{name} = ...")]),
        c.name,
        c.doc
    )
}

fn origin_text(k: &KernelEntry) -> String {
    match &k.origin {
        KernelOrigin::BuiltIn => "built-in kernel".to_string(),
        KernelOrigin::Package { package, version } => {
            format!("kernel package `{package}` {version}")
        }
        KernelOrigin::Local => "local kernel file".to_string(),
    }
}

pub fn kernel_markdown(k: &KernelEntry) -> String {
    match &k.error {
        Some(e) => format!(
            "**{}**, {}: does not check\n\n{}",
            k.name,
            origin_text(k),
            fenced("text", &[e])
        ),
        None => fenced("text", &[k.doc.trim_end()]),
    }
}

fn setting_markdown(k: &KernelEntry, s: &KernelSetting) -> String {
    let decl = match (&s.decl, &s.default) {
        (Some(d), _) => d.clone(),
        (None, Some(v)) => format!("{} = {v}", s.name),
        (None, None) => s.name.clone(),
    };
    let mut md = fenced("lapkernel", &[&decl]);
    if let Some(a) = &s.adapt {
        md += &format!("\n\n`{a}`");
    }
    if let Some(d) = &s.doc {
        md += &format!("\n\n{d}");
    }
    md += &format!("\n\nA setting of `{}` ({}).", k.name, origin_text(k));
    md
}

#[cfg(test)]
mod tests;
