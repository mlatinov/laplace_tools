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
    builtins, combinators, kernel_index, Builtin, BuiltinKind, Combinator, KernelEntry,
    KernelOrigin, KernelSetting, Project,
};

use laplace_sampler_syntax::builtin_kernels::BuiltIn;

use super::cursor::{self, Token, TokenKind};
use super::model::Model;
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

// ---- completion --------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// A `.lapkernel` context value.
    Value,
    /// A `.lapkernel` function, or a `.lapsampler` kernel.
    Function,
    /// A terminal, a combinator, `block` or `rest`.
    Keyword,
    /// A setting inside a call.
    Setting,
    /// A model parameter inside `block(...)`.
    Parameter,
}

/// One completion: `insert` is a snippet that replaces `replace` (the word
/// typed so far).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    pub kind: ItemKind,
    pub detail: Option<String>,
    /// Markdown.
    pub documentation: Option<String>,
    pub insert: String,
    pub replace: Range<usize>,
    /// Kernels first, then combinators.
    pub sort: String,
}

/// Escape snippet syntax in literal text.
fn literal(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('$', "\\$")
        .replace('}', "\\}")
}

/// What to offer at `offset`. `index` is needed for a `.lapsampler` only.
pub fn completion(
    language: Language,
    text: &str,
    offset: usize,
    index: Option<&Index>,
    model: Option<&Model>,
) -> Vec<Item> {
    if cursor::in_comment(text, offset) {
        return vec![];
    }
    let masked = cursor::mask_comments(text);
    let start = cursor::token_start(&masked, offset);
    let replace = start..offset;
    match language {
        Language::Kernel => kernel_completion(&masked, offset, replace),
        Language::Sampler => match index {
            Some(index) => sampler_completion(&masked, offset, replace, index, model),
            None => vec![],
        },
    }
}

/// Built-ins, in any block of the kernel: in `tuning { }` only those a
/// tuning default may use.
fn kernel_completion(masked: &str, offset: usize, replace: Range<usize>) -> Vec<Item> {
    let typed = &masked[replace.clone()];
    if !typed
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return vec![];
    }
    // Not a member: `x.1`.
    if masked[..replace.start].ends_with('.') {
        return vec![];
    }
    let sections = cursor::sections(masked, offset);
    let Some(section) = sections.get(1) else {
        return vec![];
    };
    builtins()
        .into_iter()
        .filter(|b| section != "tuning" || !b.needs_chain)
        .map(|b| Item {
            label: b.name.to_string(),
            kind: match b.kind {
                BuiltinKind::Value => ItemKind::Value,
                BuiltinKind::Function => ItemKind::Function,
                BuiltinKind::Terminal => ItemKind::Keyword,
            },
            detail: Some(b.signatures.join("  |  ")),
            documentation: Some(builtin_markdown(&b)),
            insert: match b.kind {
                BuiltinKind::Value => b.name.to_string(),
                _ => format!("{}($0)", b.name),
            },
            replace: replace.clone(),
            sort: format!("0{}", b.name),
        })
        .collect()
}

fn sampler_completion(
    masked: &str,
    offset: usize,
    replace: Range<usize>,
    index: &Index,
    model: Option<&Model>,
) -> Vec<Item> {
    let sections = cursor::sections(masked, offset);
    if sections.len() != 2 || sections[1] != "kernel" {
        return vec![];
    }
    let line = cursor::kernel_line_start(masked, offset);
    let Some(colon) = cursor::line_colon(masked, line, offset) else {
        // The block side of `block(...) : kernel`: the line's first word,
        // or inside `block(...)` the model's parameters (Q8).
        if masked[line..replace.start].trim().is_empty() {
            return line_targets(replace);
        }
        return match (cursor::enclosing_call(masked, line, offset), model) {
            (Some(call), Some(model)) if call.callee.text == "block" => {
                parameter_items(masked, model, replace)
            }
            _ => vec![],
        };
    };
    let Some(call) = cursor::enclosing_call(masked, colon + 1, offset) else {
        return kernel_items(index, replace);
    };
    let arg = &masked[call.arg_start..offset];
    let has_colon = arg.contains(':') && !arg.contains("::");
    let kernels = match call.callee.text.as_str() {
        "seq" => true,
        "mix" => has_colon,
        "repeat" | "on" => call.arg >= 1,
        "adapt" if call.arg == 0 => true,
        "adapt" => {
            if arg.contains('=') {
                return vec![];
            }
            let given = cursor::named_args(masked, call.open, offset);
            let adapt = combinator("adapt").expect("adapt is a combinator");
            return adapt
                .settings
                .iter()
                .filter(|s| !given.iter().any(|g| g == *s))
                .map(|s| Item {
                    label: s.to_string(),
                    kind: ItemKind::Setting,
                    detail: Some(format!("{s} = ... (adapt)")),
                    documentation: Some(combinator_setting_markdown(adapt, s)),
                    insert: format!("{s} = $0"),
                    replace: replace.clone(),
                    sort: format!("0{s}"),
                })
                .collect();
        }
        "block" => return vec![],
        _ => {
            if arg.contains('=') {
                return vec![];
            }
            let Some(kernel) = index.find(&call.callee) else {
                return vec![];
            };
            let given = cursor::named_args(masked, call.open, offset);
            return kernel
                .settings
                .iter()
                .filter(|s| !given.contains(&s.name))
                .map(|s| setting_item(kernel, s, replace.clone()))
                .collect();
        }
    };
    if kernels {
        kernel_items(index, replace)
    } else {
        vec![]
    }
}

/// The model's parameters, except those any `block(...)` in the sampler
/// already names: each parameter belongs to exactly one block.
fn parameter_items(masked: &str, model: &Model, replace: Range<usize>) -> Vec<Item> {
    let named = block_names(masked, replace.clone());
    let file = model
        .file
        .file_name()
        .map_or(String::new(), |f| f.to_string_lossy().into_owned());
    model
        .parameters
        .iter()
        .enumerate()
        .filter(|(_, p)| !named.contains(&p.name))
        .map(|(i, p)| Item {
            label: p.name.clone(),
            kind: ItemKind::Parameter,
            detail: Some(format!("{}  ({file})", p.decl)),
            documentation: None,
            insert: literal(&p.name),
            replace: replace.clone(),
            sort: format!("{i:04}"),
        })
        .collect()
}

/// Every name inside a `block(...)` in `masked`, except at `except` (the
/// word being typed).
fn block_names(masked: &str, except: Range<usize>) -> Vec<String> {
    let b = masked.as_bytes();
    let mut names = vec![];
    let mut from = 0;
    while let Some(i) = masked[from..].find("block").map(|i| from + i) {
        from = i + "block".len();
        if i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') {
            continue;
        }
        let rest = &masked[from..];
        let Some(args) = rest.trim_start().strip_prefix('(') else {
            continue;
        };
        let open = masked.len() - args.len();
        let close = args
            .find([')', ':', '{', '}', '\n'])
            .map_or(masked.len(), |e| open + e);
        let mut j = open;
        for word in masked[open..close].split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            let start = j;
            j += word.len() + 1;
            if !word.is_empty() && (start..start + word.len()) != except {
                names.push(word.to_string());
            }
        }
    }
    names
}

fn line_targets(replace: Range<usize>) -> Vec<Item> {
    [
        (
            "block",
            "block(${1:param}) : $0",
            "a block of parameters and the kernel that updates it",
        ),
        (
            "rest",
            "rest : $0",
            "every parameter no other line names; the last line",
        ),
    ]
    .into_iter()
    .map(|(label, insert, doc)| Item {
        label: label.to_string(),
        kind: ItemKind::Keyword,
        detail: Some(doc.to_string()),
        documentation: None,
        insert: insert.to_string(),
        replace: replace.clone(),
        sort: format!("0{label}"),
    })
    .collect()
}

/// Every kernel, then the combinators a kernel line may contain.
fn kernel_items(index: &Index, replace: Range<usize>) -> Vec<Item> {
    let kernels = index.kernels.iter().map(|k| Item {
        label: k.name.clone(),
        kind: ItemKind::Function,
        detail: Some(match (&k.error, &k.brief) {
            (Some(_), _) => format!("{}: does not check", origin_text(k)),
            (None, Some(b)) => format!("{}: {b}", origin_text(k)),
            (None, None) => origin_text(k),
        }),
        documentation: Some(kernel_markdown(k)),
        // A built-in needs its parentheses; the others take them only for
        // tuning values.
        insert: match k.origin {
            KernelOrigin::BuiltIn => format!("{}($0)", k.name),
            _ => literal(&k.name),
        },
        replace: replace.clone(),
        sort: format!(
            "{}{}",
            match k.origin {
                KernelOrigin::BuiltIn => 0,
                KernelOrigin::Package { .. } => 1,
                KernelOrigin::Local => 2,
            },
            k.name
        ),
    });
    let combinators = combinators()
        .iter()
        .filter(|c| c.in_kernel_lines)
        .map(|c| Item {
            label: c.name.to_string(),
            kind: ItemKind::Keyword,
            detail: Some(c.signature.to_string()),
            documentation: Some(combinator_markdown(c)),
            insert: format!("{}($0)", c.name),
            replace: replace.clone(),
            sort: format!("3{}", c.name),
        });
    kernels.chain(combinators).collect()
}

fn setting_item(kernel: &KernelEntry, s: &KernelSetting, replace: Range<usize>) -> Item {
    Item {
        label: s.name.clone(),
        kind: ItemKind::Setting,
        detail: Some(match (&s.decl, &s.default) {
            (Some(d), _) => d.clone(),
            (None, Some(v)) => format!("{} = {v}", s.name),
            (None, None) => s.name.clone(),
        }),
        documentation: Some(setting_markdown(kernel, s)),
        insert: match &s.default {
            Some(v) => format!("{} = ${{1:{}}}", s.name, literal(v)),
            None => format!("{} = $0", s.name),
        },
        replace,
        sort: format!("0{}", s.name),
    }
}

// ---- signature help ----------------------------------------------------------

/// One signature: `label`, and each parameter as a byte range of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub label: String,
    /// Markdown.
    pub documentation: Option<String>,
    pub parameters: Vec<(Range<usize>, Option<String>)>,
    pub active: Option<usize>,
}

/// The arguments in `label`'s outer parentheses, split at top-level
/// commas, as byte ranges (a `...` is not an argument).
fn label_parameters(label: &str) -> Vec<Range<usize>> {
    let Some(open) = label.find('(') else {
        return vec![];
    };
    let close = label.rfind(')').unwrap_or(label.len());
    let mut out = vec![];
    let mut depth = 0;
    let mut start = open + 1;
    for (i, c) in label.char_indices().take(close).skip(open + 1) {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(start..i);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(start..close);
    out.into_iter()
        .map(|r| {
            let text = &label[r.clone()];
            let lead = text.len() - text.trim_start().len();
            r.start + lead..r.start + text.trim_end().len()
        })
        .filter(|r| !r.is_empty() && &label[r.clone()] != "...")
        .collect()
}

/// The signature of the combinator or kernel call around `offset` in a
/// `.lapsampler` kernel line.
pub fn signature_help(text: &str, offset: usize, index: &Index) -> Option<Signature> {
    if cursor::in_comment(text, offset) {
        return None;
    }
    let masked = cursor::mask_comments(text);
    let sections = cursor::sections(&masked, offset);
    if sections.len() != 2 || sections[1] != "kernel" {
        return None;
    }
    let line = cursor::kernel_line_start(&masked, offset);
    let colon = cursor::line_colon(&masked, line, offset)?;
    let call = cursor::enclosing_call(&masked, colon + 1, offset)?;
    if let Some(c) = combinator(&call.callee.text) {
        let parameters: Vec<(Range<usize>, Option<String>)> = label_parameters(c.signature)
            .into_iter()
            .map(|r| (r, None))
            .collect();
        let active = call.arg.min(parameters.len().saturating_sub(1));
        return Some(Signature {
            label: c.signature.to_string(),
            documentation: Some(c.doc.to_string()),
            parameters,
            active: Some(active),
        });
    }
    let kernel = index.find(&call.callee)?;
    if kernel.error.is_some() {
        return None;
    }
    let mut label = format!("{}(", kernel.name);
    let mut parameters = vec![];
    for (i, s) in kernel.settings.iter().enumerate() {
        if i > 0 {
            label += ", ";
        }
        let start = label.len();
        label += &format!("{} = {}", s.name, s.default.as_deref().unwrap_or("..."));
        let mut doc = vec![];
        if let Some(a) = &s.adapt {
            doc.push(format!("`{a}`"));
        }
        if let Some(d) = &s.doc {
            doc.push(d.clone());
        }
        parameters.push((start..label.len(), (!doc.is_empty()).then(|| doc.join(" "))));
    }
    label += ")";
    // Settings are named: the active one is the one being written, else
    // the first not yet given.
    let arg = masked[call.arg_start..offset].trim_start();
    let typed: String = arg
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let given = cursor::named_args(&masked, call.open, offset);
    let active = if !typed.is_empty() {
        kernel
            .settings
            .iter()
            .position(|s| s.name == typed)
            .or_else(|| {
                (!arg.contains('='))
                    .then(|| {
                        kernel
                            .settings
                            .iter()
                            .position(|s| s.name.starts_with(&typed))
                    })
                    .flatten()
            })
    } else {
        kernel
            .settings
            .iter()
            .position(|s| !given.contains(&s.name))
    };
    Some(Signature {
        label,
        documentation: kernel.brief.clone(),
        parameters,
        active,
    })
}

// ---- go-to-definition --------------------------------------------------------

/// Where a kernel reference is defined.
#[derive(Clone, Debug, PartialEq)]
pub enum Definition {
    /// A `.lapkernel`, and the byte range of the kernel's name in it.
    File(PathBuf, Range<usize>),
    /// A built-in kernel, implemented in Rust (`BuiltIn::source()`).
    BuiltIn(BuiltIn),
}

/// The definition of the kernel named at `offset` in a `.lapsampler`.
pub fn definition(text: &str, offset: usize, index: &Index) -> Option<Definition> {
    if cursor::in_comment(text, offset) {
        return None;
    }
    let masked = cursor::mask_comments(text);
    let token = cursor::token_at(&masked, offset)?;
    let next = cursor::next_char(&masked, token.range.end);
    if token.kind == TokenKind::Name && (next == Some('=') || combinator(&token.text).is_some()) {
        return None;
    }
    let file = match index.find(&token) {
        Some(k) if k.origin == KernelOrigin::BuiltIn => {
            return BuiltIn::parse(&k.name).ok().map(Definition::BuiltIn);
        }
        Some(k) => k.file.clone()?,
        // A path outside the project is still a file.
        None if token.kind == TokenKind::Path => {
            let f = index.dir.join(&token.text);
            f.is_file().then_some(f)?
        }
        None => return None,
    };
    let source = std::fs::read_to_string(&file).unwrap_or_default();
    Some(Definition::File(file, kernel_name_range(&source)))
}

/// A built-in kernel's implementation as a file to open: written once
/// under `cache` (a directory per version of the source), read-only, as
/// `<name>.rs`. The source is the one built into this server
/// (`BuiltIn::source()`), so it matches the checker it was built with.
pub fn builtin_source_file(cache: &Path, b: BuiltIn) -> std::io::Result<PathBuf> {
    use std::hash::{Hash, Hasher};
    let (_, source) = b.source();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let dir = cache.join(format!("{:016x}", h.finish()));
    let file = dir.join(format!("{}.rs", b.name()));
    if std::fs::read_to_string(&file).ok().as_deref() != Some(source) {
        std::fs::create_dir_all(&dir)?;
        if file.exists() {
            let mut perms = std::fs::metadata(&file)?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            std::fs::set_permissions(&file, perms)?;
        }
        std::fs::write(&file, source)?;
        let mut perms = std::fs::metadata(&file)?.permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&file, perms)?;
    }
    Ok(file)
}

/// The name in `kernel <name> {` (or the start of the file).
fn kernel_name_range(source: &str) -> Range<usize> {
    let masked = cursor::mask_comments(source);
    let b = masked.as_bytes();
    let mut from = 0;
    while let Some(i) = masked[from..].find("kernel").map(|i| from + i) {
        let before_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        let after = i + "kernel".len();
        if before_ok && b.get(after).is_some_and(|c| c.is_ascii_whitespace()) {
            let start = after + masked[after..].len() - masked[after..].trim_start().len();
            let end = masked[start..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .map_or(masked.len(), |e| start + e);
            return start..end;
        }
        from = after;
    }
    0..0
}

#[cfg(test)]
mod tests;
