//! Completion candidates, powered by the same `Analysis` symbol tables as
//! semantic tokens, plus `resolve`'s installed-package docs for `pkg::`.

use std::path::Path;

use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Documentation};

use laplace::docs::PackageDocs;
use laplace::resolve::lockfile::Lockfile;

use crate::analysis::Analysis;

pub const BLOCK_KEYWORDS: &[&str] = &[
    "data",
    "parameters",
    "transformed data",
    "transformed parameters",
    "model",
    "generated quantities",
    "functions",
    "library",
];

/// Variables in scope, user-defined functions, block/section keywords, and
/// `pkg::` starters for every imported library.
pub fn general_completions(analysis: &Analysis) -> Vec<CompletionItem> {
    let mut items = Vec::new();

    for name in analysis.symbol_roles.keys() {
        items.push(CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::VARIABLE),
            ..Default::default()
        });
    }

    for name in &analysis.user_functions {
        items.push(CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("user-defined".to_string()),
            ..Default::default()
        });
    }

    for kw in BLOCK_KEYWORDS {
        items.push(CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        });
    }

    for import in &analysis.imports {
        items.push(CompletionItem {
            label: format!("{}::", import.name),
            insert_text: Some(format!("{}::", import.name)),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some("imported library".to_string()),
            ..Default::default()
        });
    }

    items
}

/// Completions for `pkg::` -- every exported function of `pkg`, resolved via
/// the project's lockfile and that package's installed `docs.json` sidecar.
pub fn package_completions(lock: &Lockfile, cache_root: &Path, package: &str) -> Vec<CompletionItem> {
    let Some(locked) = lock.packages.iter().find(|p| p.name == package) else {
        return Vec::new();
    };
    let docs_path = cache_root
        .join(&locked.name)
        .join(&locked.version)
        .join("docs.json");
    let Ok(text) = std::fs::read_to_string(&docs_path) else {
        return Vec::new();
    };
    let Ok(docs) = serde_json::from_str::<PackageDocs>(&text) else {
        return Vec::new();
    };

    docs.functions
        .into_iter()
        .map(|sig| {
            let params = sig
                .params
                .iter()
                .map(|(name, ty)| format!("{name}: {ty}"))
                .collect::<Vec<_>>()
                .join(", ");
            CompletionItem {
                label: sig.name.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(format!("({params}) -> {}", sig.return_type)),
                documentation: sig
                    .doc
                    .as_ref()
                    .and_then(|d| d.brief.clone())
                    .map(Documentation::String),
                ..Default::default()
            }
        })
        .collect()
}
