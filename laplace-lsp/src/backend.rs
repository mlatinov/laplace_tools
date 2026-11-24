//! The `tower-lsp` `LanguageServer` implementation: document store,
//! debounced diagnostics, semantic tokens, completion, hover, and
//! go-to-definition, all built on top of `analysis`/`diagnostics`/
//! `completion`/`workspace`. `.lapkernel` and `.lapsampler` documents get
//! diagnostics only (`sampler`); the other requests answer nothing for them.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{async_trait, Client, LanguageServer};

use crate::analysis::{self, TOKEN_TYPES};
use crate::completion;
use crate::diagnostics;
use crate::dialect::Dialect;
use crate::hover;
use crate::position;
use crate::sampler::{self, features, Language};
use crate::stanc;
use crate::workspace;

/// How long to wait after the last edit before running diagnostics, so a
/// fast typist doesn't trigger a full codegen pass per keystroke.
const DIAGNOSTICS_DEBOUNCE: Duration = Duration::from_millis(350);

/// The same for `.lapkernel`/`.lapsampler`: their check is a parse and a
/// type check in process, with no codegen and no `stanc`, so it can run
/// sooner.
const SAMPLER_DIAGNOSTICS_DEBOUNCE: Duration = Duration::from_millis(200);

pub struct Backend {
    client: Client,
    documents: Arc<DashMap<Url, String>>,
    generations: Arc<DashMap<Url, Arc<AtomicU64>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Backend {
            client,
            documents: Arc::new(DashMap::new()),
            generations: Arc::new(DashMap::new()),
        }
    }

    fn schedule_diagnostics(&self, uri: Url) {
        let generation = self
            .generations
            .entry(uri.clone())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)))
            .clone();
        let my_generation = generation.fetch_add(1, Ordering::SeqCst) + 1;

        let client = self.client.clone();
        let documents = self.documents.clone();

        if let Some(language) = Language::for_uri(&uri) {
            tokio::spawn(async move {
                tokio::time::sleep(SAMPLER_DIAGNOSTICS_DEBOUNCE).await;
                if generation.load(Ordering::SeqCst) != my_generation {
                    return;
                }
                let Some(text) = documents.get(&uri).map(|entry| entry.clone()) else {
                    return;
                };
                let Ok(path) = uri.to_file_path() else {
                    return;
                };
                // Reads kernel files and laplace.lock from disk.
                let Ok(diags) = tokio::task::spawn_blocking(move || {
                    sampler::diagnostics(language, &path, &text)
                })
                .await
                else {
                    return;
                };
                if generation.load(Ordering::SeqCst) != my_generation {
                    return;
                }
                client.publish_diagnostics(uri, diags, None).await;
            });
            return;
        }

        tokio::spawn(async move {
            tokio::time::sleep(DIAGNOSTICS_DEBOUNCE).await;
            if generation.load(Ordering::SeqCst) != my_generation {
                return; // superseded by a newer edit
            }
            let Some(text) = documents.get(&uri).map(|entry| entry.clone()) else {
                return;
            };
            let mut diags = diagnostics::compute_diagnostics_for_uri(&uri, &text);

            // Best-effort live `stanc` type-checking (catches missing
            // semicolons, unknown types, incompatible operand types, ...) on
            // top of the always-on import/lockfile checks above. Shells out
            // to a subprocess, so it runs off the async executor; silently
            // contributes nothing if `stanc` isn't installed.
            let stanc_uri = uri.clone();
            let stanc_text = text.clone();
            if let Ok(stanc_diags) = tokio::task::spawn_blocking(move || {
                stanc::compute_stanc_diagnostics_for_uri(&stanc_uri, &stanc_text)
            })
            .await
            {
                diags.extend(stanc_diags);
            }

            if generation.load(Ordering::SeqCst) != my_generation {
                return; // superseded while the stanc pass was running
            }
            client.publish_diagnostics(uri, diags, None).await;
        });
    }
}

#[async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    // `:` for `pkg::`. The rest are for sampler files only
                    // (a setting after `(` or `,`, a path after `/`); a
                    // .laplace request they trigger answers nothing.
                    trigger_characters: Some([":", "(", ",", "/"].map(String::from).to_vec()),
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: TOKEN_TYPES
                                    .iter()
                                    .map(|s| SemanticTokenType::new(s))
                                    .collect(),
                                token_modifiers: vec![],
                            },
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: Some(false),
                            work_done_progress_options: Default::default(),
                        },
                    ),
                ),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "laplace-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "laplace-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents
            .insert(uri.clone(), params.text_document.text);
        self.schedule_diagnostics(uri);
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.pop() {
            self.documents.insert(uri.clone(), change.text);
        }
        self.schedule_diagnostics(uri);
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        // An open .lapsampler reads the kernels it names from disk, so
        // saving a kernel re-checks every open sampler file.
        if Language::for_uri(&uri) == Some(Language::Kernel) {
            let samplers: Vec<Url> = self
                .documents
                .iter()
                .map(|entry| entry.key().clone())
                .filter(|u| *u != uri && Language::for_uri(u) == Some(Language::Sampler))
                .collect();
            for sampler_uri in samplers {
                self.schedule_diagnostics(sampler_uri);
            }
        }
        if self.documents.contains_key(&uri) {
            self.schedule_diagnostics(uri);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.remove(&uri);
        self.generations.remove(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let Some(text) = self.documents.get(&uri).map(|entry| entry.clone()) else {
            return Ok(None);
        };
        if let Some(language) = Language::for_uri(&uri) {
            let items = sampler_completion(&uri, language, text, position).await;
            return Ok(Some(CompletionResponse::Array(items)));
        }
        let sampler_trigger = params
            .context
            .and_then(|c| c.trigger_character)
            .is_some_and(|c| c != ":");
        if sampler_trigger {
            return Ok(None);
        }

        let offset = position::position_to_offset(&text, position);
        let prefix = &text[..offset.min(text.len())];

        if let Some(package) = package_prefix(prefix) {
            let Ok(path) = uri.to_file_path() else {
                return Ok(None);
            };
            let dir = path.parent().unwrap_or_else(|| Path::new("."));
            let cache_root = workspace::default_cache_root();
            let lock = workspace::find_lockfile(dir)
                .and_then(|p| laplace::resolve::lockfile::read_lockfile(&p).ok())
                .unwrap_or_default();
            let items = completion::package_completions(&lock, &cache_root, &package);
            return Ok(Some(CompletionResponse::Array(items)));
        }

        let analysis = analysis::analyze(&text);
        Ok(Some(CompletionResponse::Array(
            completion::general_completions(&analysis, Dialect::for_uri(&uri)),
        )))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(text) = self.documents.get(&uri).map(|entry| entry.clone()) else {
            return Ok(None);
        };
        if let Some(language) = Language::for_uri(&uri) {
            return Ok(sampler_hover(&uri, language, text, position).await);
        }

        let offset = position::position_to_offset(&text, position);
        let analysis = analysis::analyze(&text);
        let Some(call) = analysis
            .library_calls
            .iter()
            .find(|c| c.range.contains(&offset))
        else {
            return Ok(None);
        };

        let Ok(path) = uri.to_file_path() else {
            return Ok(None);
        };
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let Some(lockfile_path) = workspace::find_lockfile(dir) else {
            return Ok(None);
        };
        let cache_root = workspace::default_cache_root();

        match laplace::docs::lookup(&lockfile_path, &cache_root, &call.package, &call.func) {
            Ok(sigs) => Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: hover::markdown(&laplace::docs::render_overloads(&call.package, &sigs)),
                }),
                range: None,
            })),
            Err(_) => Ok(None),
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(text) = self.documents.get(&uri).map(|entry| entry.clone()) else {
            return Ok(None);
        };
        if let Some(language) = Language::for_uri(&uri) {
            return Ok(sampler_definition(&uri, language, text, position)
                .await
                .map(GotoDefinitionResponse::Scalar));
        }

        let offset = position::position_to_offset(&text, position);
        let analysis = analysis::analyze(&text);
        let Some(call) = analysis
            .library_calls
            .iter()
            .find(|c| c.range.contains(&offset))
        else {
            return Ok(None);
        };

        let Ok(path) = uri.to_file_path() else {
            return Ok(None);
        };
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let Some(lockfile_path) = workspace::find_lockfile(dir) else {
            return Ok(None);
        };
        let Ok(lock) = laplace::resolve::lockfile::read_lockfile(&lockfile_path) else {
            return Ok(None);
        };
        let Some(locked) = lock.packages.iter().find(|p| p.name == call.package) else {
            return Ok(None);
        };
        let cache_root = workspace::default_cache_root();
        let package_dir = cache_root.join(&locked.name).join(&locked.version);

        let Some((file, def_offset)) =
            workspace::find_function_definition(&package_dir, &call.func)
        else {
            return Ok(None);
        };
        let Ok(def_source) = std::fs::read_to_string(&file) else {
            return Ok(None);
        };
        let Ok(def_uri) = Url::from_file_path(&file) else {
            return Ok(None);
        };
        let starts = position::line_starts(&def_source);
        let start_pos = position::offset_to_position(&def_source, &starts, def_offset);
        let end_pos = position::offset_to_position(
            &def_source,
            &starts,
            (def_offset + call.func.len()).min(def_source.len()),
        );

        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri: def_uri,
            range: Range {
                start: start_pos,
                end: end_pos,
            },
        })))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
        if Language::for_uri(&uri).is_some() {
            return Ok(None);
        }
        let Some(text) = self.documents.get(&uri).map(|entry| entry.clone()) else {
            return Ok(None);
        };
        let analysis = analysis::analyze(&text);
        let raw = analysis::collect_tokens(&text, &analysis);
        let data = position::build_semantic_tokens(&text, &raw);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }
}

/// If `prefix` (all text up to the cursor) ends with `<ident>::<partial>`,
/// return `<ident>` -- the package name completion should be scoped to.
fn package_prefix(prefix: &str) -> Option<String> {
    let (before, after) = prefix.rsplit_once("::")?;
    if !after.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let name: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// The kernel index for a `.lapsampler` (reads the disk, so off the async
/// executor). `None` for a `.lapkernel`, which needs none.
async fn sampler_index(uri: &Url, language: Language) -> Option<features::Index> {
    if language != Language::Sampler {
        return None;
    }
    let path = uri.to_file_path().ok()?;
    tokio::task::spawn_blocking(move || features::Index::for_document(&path))
        .await
        .ok()
}

/// A byte range of `text` as an LSP range.
fn lsp_range(text: &str, range: std::ops::Range<usize>) -> Range {
    let starts = position::line_starts(text);
    Range {
        start: position::offset_to_position(text, &starts, range.start),
        end: position::offset_to_position(text, &starts, range.end),
    }
}

async fn sampler_hover(
    uri: &Url,
    language: Language,
    text: String,
    position: Position,
) -> Option<Hover> {
    let offset = position::position_to_offset(&text, position);
    let index = sampler_index(uri, language).await;
    let (range, markdown) = features::hover(language, &text, offset, index.as_ref())?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: Some(lsp_range(&text, range)),
    })
}

async fn sampler_completion(
    uri: &Url,
    language: Language,
    text: String,
    position: Position,
) -> Vec<CompletionItem> {
    let offset = position::position_to_offset(&text, position);
    let index = sampler_index(uri, language).await;
    features::completion(language, &text, offset, index.as_ref())
        .into_iter()
        .map(|item| CompletionItem {
            label: item.label.clone(),
            kind: Some(match item.kind {
                features::ItemKind::Value => CompletionItemKind::VARIABLE,
                features::ItemKind::Function => CompletionItemKind::FUNCTION,
                features::ItemKind::Keyword => CompletionItemKind::KEYWORD,
                features::ItemKind::Setting => CompletionItemKind::PROPERTY,
            }),
            detail: item.detail,
            documentation: item.documentation.map(|value| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value,
                })
            }),
            sort_text: Some(item.sort),
            filter_text: Some(item.label),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: lsp_range(&text, item.replace),
                new_text: item.insert,
            })),
            ..Default::default()
        })
        .collect()
}

/// Where built-in kernels' sources are written for go-to-definition:
/// `$XDG_CACHE_HOME/laplace-lsp/builtin-kernels`, else under `~/.cache`.
fn builtin_kernels_cache() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cache")))?;
    Some(base.join("laplace-lsp").join("builtin-kernels"))
}

async fn sampler_definition(
    uri: &Url,
    language: Language,
    text: String,
    position: Position,
) -> Option<Location> {
    let offset = position::position_to_offset(&text, position);
    let index = sampler_index(uri, language).await?;
    tokio::task::spawn_blocking(move || {
        let (file, range) = match features::definition(&text, offset, &index)? {
            features::Definition::File(file, range) => (file, range),
            features::Definition::BuiltIn(b) => {
                let file = features::builtin_source_file(&builtin_kernels_cache()?, b).ok()?;
                (file, 0..0)
            }
        };
        let source = std::fs::read_to_string(&file).ok()?;
        Some(Location {
            uri: Url::from_file_path(&file).ok()?,
            range: lsp_range(&source, range),
        })
    })
    .await
    .ok()
    .flatten()
}
