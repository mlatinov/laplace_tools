//! The model a `.lapsampler` is used with, for completing parameter names
//! inside `block(...)` (open question Q8).
//!
//! A sampler doesn't name its model: `laplace sample model.laplace
//! --sampler s.lapsampler` pairs them. The editor guesses, in order:
//!
//! 1. a `// model: <path>` comment in the sampler (relative to it), which
//!    overrides the rest;
//! 2. `<stem>.laplace` next to `<stem>.lapsampler`;
//! 3. the only `.laplace` file in the sampler's directory.
//!
//! The names are the declarations written in the model's `parameters { }`
//! block, read with the compiler crate's own scanners. A parameter that a
//! template, macro or package call adds isn't written there, so it isn't
//! offered. Nothing is checked: `laplace sampler check --model` does that.

use std::path::{Path, PathBuf};

use laplace::parser::blocks::{find_top_level_blocks, BlockKind};
use laplace::parser::declarations::declarations;

/// One parameter a model declares.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: String,
    /// Its declaration, on one line: `vector<lower=0>[K] tau`.
    pub decl: String,
}

/// A model and the parameters it declares.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub file: PathBuf,
    pub parameters: Vec<Parameter>,
}

/// The path a `// model: <path>` comment gives, as written.
pub fn directive(text: &str) -> Option<&str> {
    text.lines().find_map(|line| {
        let comment = line.trim_start().strip_prefix("//")?;
        let path = comment.trim_start().strip_prefix("model:")?.trim();
        (!path.is_empty()).then_some(path)
    })
}

/// Which model the sampler at `sampler` (with contents `text`) goes with.
pub fn model_file(sampler: &Path, text: &str) -> Option<PathBuf> {
    let dir = sampler.parent()?;
    if let Some(path) = directive(text) {
        let file = dir.join(path);
        return file.is_file().then_some(file);
    }
    let sibling = sampler.with_extension("laplace");
    if sibling.is_file() {
        return Some(sibling);
    }
    let mut models = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("laplace"));
    let only = models.next()?;
    models.next().is_none().then_some(only)
}

/// The parameters `source` (a `.laplace` model) declares in its
/// `parameters { }` block, in order.
pub fn parameters(source: &str) -> Vec<Parameter> {
    // The compiler's scanners skip `//` comments and strings but not
    // `/* */`, so blank comments out first (offsets are kept).
    let source = &super::cursor::mask_comments(source);
    let mut out: Vec<Parameter> = vec![];
    for block in find_top_level_blocks(source) {
        if block.kind != BlockKind::Parameters {
            continue;
        }
        let body = &source[block.body_range.clone()];
        for d in declarations(body) {
            if d.depth != 0 || out.iter().any(|p| p.name == d.name) {
                continue;
            }
            out.push(Parameter {
                decl: declaration_text(body, d.range.clone()),
                name: d.name,
            });
        }
    }
    out
}

/// The statement around a declared name, from the previous `;` (or the
/// block's start) to the name's end, on one line, comments dropped.
fn declaration_text(body: &str, name: std::ops::Range<usize>) -> String {
    let start = body[..name.start].rfind(';').map_or(0, |i| i + 1);
    body[start..name.end]
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The model for the sampler at `sampler`, read from disk. `None` when
/// none is found or it declares no parameters.
pub fn load(sampler: &Path, text: &str) -> Option<Model> {
    let file = model_file(sampler, text)?;
    let source = std::fs::read_to_string(&file).ok()?;
    let parameters = parameters(&source);
    (!parameters.is_empty()).then_some(Model { file, parameters })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = "data {\n  int N;\n}\nparameters {\n  real mu; // the mean\n  \
        vector<lower=0>[N]\n    tau;\n  real<lower=0> sigma;\n}\nmodel {\n  real local;\n}\n";

    #[test]
    fn the_parameters_block_only_at_its_top_level() {
        let names: Vec<(String, String)> = parameters(MODEL)
            .into_iter()
            .map(|p| (p.name, p.decl))
            .collect();
        assert_eq!(
            names,
            [
                ("mu".into(), "real mu".into()),
                ("tau".into(), "vector<lower=0>[N] tau".into()),
                ("sigma".into(), "real<lower=0> sigma".into()),
            ]
        );
        assert_eq!(parameters("data { int N; }"), vec![]);
        // Not Stan, but only the block's own level counts.
        let nested = parameters("parameters { real a; { real b; } }");
        assert_eq!(nested.len(), 1);
        assert_eq!(parameters("/* parameters { real x; } */"), vec![]);
    }

    #[test]
    fn the_directive() {
        assert_eq!(
            directive("// model: ../m.laplace\nsampler s {}"),
            Some("../m.laplace")
        );
        assert_eq!(
            directive("sampler s {}\n  //model:x.laplace  "),
            Some("x.laplace")
        );
        assert_eq!(directive("// the model: x.laplace"), None);
        assert_eq!(directive("// model:\n"), None);
    }

    #[test]
    fn which_model_a_sampler_goes_with() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let s = dir.join("s.lapsampler");
        let file = |name: &str| {
            std::fs::write(dir.join(name), MODEL).unwrap();
            dir.join(name)
        };
        assert_eq!(model_file(&s, ""), None);
        let only = file("only.laplace");
        assert_eq!(model_file(&s, ""), Some(only));
        file("other.laplace");
        assert_eq!(model_file(&s, ""), None, "two models, neither the sibling");
        let sibling = file("s.laplace");
        assert_eq!(model_file(&s, ""), Some(sibling));
        std::fs::create_dir(dir.join("m")).unwrap();
        let named = file("m/x.laplace");
        assert_eq!(model_file(&s, "// model: m/x.laplace\n"), Some(named));
        // The directive overrides, even when its file is missing.
        assert_eq!(model_file(&s, "// model: nope.laplace\n"), None);
        assert_eq!(load(&s, "").unwrap().parameters.len(), 3);
    }
}
