//! The `tower-lsp` `LanguageServer` implementation: document store,
//! debounced diagnostics, semantic tokens, completion, hover, and
//! go-to-definition, all built on top of `analysis`/`diagnostics`/
//! `completion`/`workspace`.

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
use crate::position;
use crate::stanc;
use crate::workspace;

/// How long to wait after the last edit before running diagnostics, so a
/// fast typist doesn't trigger a full codegen pass per keystroke.
const DIAGNOSTICS_DEBOUNCE: Duration = Duration::from_millis(350);

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
            if let Ok(stanc_diags) =
                tokio::task::spawn_blocking(move || stanc::compute_stanc_diagnostics_for_uri(&stanc_uri, &stanc_text))
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
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![":".to_string()]),
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
                    SemanticTokensOptions {
                        legend: SemanticTokensLegend {
                            token_types: TOKEN_TYPES.iter().map(|s| SemanticTokenType::new(s)).collect(),
                            token_modifiers: vec![],
                        },
                        full: Some(SemanticTokensFullOptions::Bool(true)),
                        range: Some(false),
                        work_done_progress_options: Default::default(),
                    },
                )),
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
        self.documents.insert(uri.clone(), params.text_document.text);
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
        self.schedule_diagnostics(params.text_document.uri);
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
        Ok(Some(CompletionResponse::Array(completion::general_completions(&analysis))))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(text) = self.documents.get(&uri).map(|entry| entry.clone()) else {
            return Ok(None);
        };

        let offset = position::position_to_offset(&text, position);
        let analysis = analysis::analyze(&text);
        let Some(call) = analysis.library_calls.iter().find(|c| c.range.contains(&offset)) else {
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
            Ok(sig) => {
                let rendered = laplace::docs::render(&call.package, &sig);
                Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!("```\n{rendered}```"),
                    }),
                    range: None,
                }))
            }
            Err(_) => Ok(None),
        }
    }

    async fn goto_definition(&self, params: GotoDefinitionParams) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(text) = self.documents.get(&uri).map(|entry| entry.clone()) else {
            return Ok(None);
        };

        let offset = position::position_to_offset(&text, position);
        let analysis = analysis::analyze(&text);
        let Some(call) = analysis.library_calls.iter().find(|c| c.range.contains(&offset)) else {
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

        let Some((file, def_offset)) = workspace::find_function_definition(&package_dir, &call.func) else {
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

    async fn semantic_tokens_full(&self, params: SemanticTokensParams) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
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
