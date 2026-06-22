use anyhow::Result;
use async_lsp::{lsp_types, LanguageClient, LanguageServer, ResponseError};
use lsp_types::{
    ClientCapabilities, Diagnostic, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    GotoDefinitionParams, GotoDefinitionResponse, HoverContents, HoverParams, InitializeParams,
    InitializedParams, MarkedString, MarkupKind, Position, PublishDiagnosticsClientCapabilities,
    PublishDiagnosticsParams, TextDocumentClientCapabilities, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, Url, WorkDoneProgressParams, WorkspaceFolder,
};
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

// ---------------------------------------------------------------------------
// ClientState: handles notifications FROM the language server
// ---------------------------------------------------------------------------

struct ClientState {
    diags: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
}

impl LanguageClient for ClientState {
    type Error = ResponseError;
    type NotifyResult = ControlFlow<async_lsp::Result<()>>;

    fn publish_diagnostics(&mut self, params: PublishDiagnosticsParams) -> Self::NotifyResult {
        let key = params.uri.to_string();
        // Use unwrap_or_else to recover from a poisoned mutex rather than panic.
        self.diags
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, params.diagnostics);
        ControlFlow::Continue(())
    }
}

// ---------------------------------------------------------------------------
// ZapLspClient: thin public wrapper around an active LSP connection
// ---------------------------------------------------------------------------

pub struct ZapLspClient {
    pub(crate) server:       async_lsp::ServerSocket,
    diags:                   Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
    mainloop_handle:         tokio::task::JoinHandle<()>,
    opened_files:            std::collections::HashSet<String>,
}

impl ZapLspClient {
    /// Spawn the language-server binary, run the LSP initialize handshake, and
    /// return a connected client.
    ///
    /// `binary` should be the fully-resolved path returned by `servers::find_binary`.
    pub async fn spawn(binary: &str, args: &[&str], root_uri: Url) -> Result<Self> {
        let diags       = Arc::new(Mutex::new(HashMap::<String, Vec<Diagnostic>>::new()));
        let diags_clone = diags.clone();

        let (mainloop, mut server) = async_lsp::MainLoop::new_client(|_client| {
            async_lsp::router::Router::from_language_client(ClientState { diags: diags_clone })
        });

        let mut child = tokio::process::Command::new(binary)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)  // kill child when the handle is dropped
            .spawn()?;

        let stdout = child.stdout.take().unwrap();
        let stdin  = child.stdin.take().unwrap();

        // Keep child alive by moving it into the task; kill_on_drop ensures the
        // OS process is reaped when this JoinHandle (and thus the task) is dropped.
        let mainloop_handle = tokio::spawn(async move {
            let _child = child; // hold child alive for the duration of the mainloop
            // Convert tokio AsyncRead/AsyncWrite to futures-compatible via tokio-util compat.
            mainloop
                .run_buffered(stdout.compat(), stdin.compat_write())
                .await
                .ok();
        });

        // LSP initialize handshake — initialize is async (request/response).
        server
            .initialize(InitializeParams {
                process_id: Some(std::process::id()),
                workspace_folders: Some(vec![WorkspaceFolder {
                    uri:  root_uri,
                    name: "root".into(),
                }]),
                capabilities: client_capabilities(),
                ..Default::default()
            })
            .await?;

        // initialized is a notification — synchronous, no await.
        server.initialized(InitializedParams {})?;

        Ok(Self { server, diags, mainloop_handle, opened_files: std::collections::HashSet::new() })
    }

    /// Returns true if the background mainloop task is still running (server alive).
    pub fn is_alive(&self) -> bool {
        !self.mainloop_handle.is_finished()
    }

    /// Notify the server that a file has been opened.
    /// Skips sending if `didOpen` was already sent for this path (protocol dedup).
    pub fn open_file(&mut self, abs_path: &str, content: &str, lang_id: &str) -> Result<()> {
        if self.opened_files.contains(abs_path) {
            return Ok(());
        }
        // Use Url::from_file_path to correctly percent-encode spaces and special chars.
        let uri = Url::from_file_path(abs_path)
            .map_err(|_| anyhow::anyhow!("invalid path: {}", abs_path))?;
        self.server
            .notify::<lsp_types::notification::DidOpenTextDocument>(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri,
                    language_id: lang_id.to_string(),
                    version:     1,
                    text:        content.to_string(),
                },
            })?;
        self.opened_files.insert(abs_path.to_string());
        Ok(())
    }

    /// Notify the server that a file has been saved.
    pub fn save_file(&self, abs_path: &str) -> Result<()> {
        let uri = Url::from_file_path(abs_path)
            .map_err(|_| anyhow::anyhow!("invalid path: {}", abs_path))?;
        self.server
            .notify::<lsp_types::notification::DidSaveTextDocument>(DidSaveTextDocumentParams {
                text_document: TextDocumentIdentifier { uri },
                text: None,
            })?;
        Ok(())
    }

    /// Return cached diagnostics for the given absolute path (empty vec if none).
    pub fn cached_diags(&self, abs_path: &str) -> Vec<Diagnostic> {
        let key = Url::from_file_path(abs_path)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| abs_path.to_string());
        self.diags
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    /// Request goto-definition locations.
    pub async fn goto_definition(
        &mut self,
        abs_path: &str,
        line: u32,
        col: u32,
    ) -> Result<Vec<lsp_types::Location>> {
        let uri = Url::from_file_path(abs_path)
            .map_err(|_| anyhow::anyhow!("invalid path: {}", abs_path))?;
        let result = self
            .server
            .definition(GotoDefinitionParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: Position { line, character: col },
                },
                work_done_progress_params:  WorkDoneProgressParams::default(),
                partial_result_params:      Default::default(),
            })
            .await?;

        Ok(match result {
            Some(GotoDefinitionResponse::Scalar(loc))  => vec![loc],
            Some(GotoDefinitionResponse::Array(locs))  => locs,
            Some(GotoDefinitionResponse::Link(links))  => links
                .into_iter()
                .map(|l| lsp_types::Location {
                    uri:   l.target_uri,
                    range: l.target_selection_range, // prefer selection range over full target range
                })
                .collect(),
            None => vec![],
        })
    }

    /// Request hover documentation.
    pub async fn hover_text(&mut self, abs_path: &str, line: u32, col: u32) -> Result<Option<String>> {
        let uri = Url::from_file_path(abs_path)
            .map_err(|_| anyhow::anyhow!("invalid path: {}", abs_path))?;
        let result = self
            .server
            .hover(HoverParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: Position { line, character: col },
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
            })
            .await?;

        Ok(result.and_then(|h| match h.contents {
            HoverContents::Markup(m)  => Some(m.value),
            HoverContents::Scalar(ms) => Some(marked_string_to_text(ms)),
            HoverContents::Array(arr) => arr.into_iter().map(marked_string_to_text).next(),
        }))
    }
}

impl Drop for ZapLspClient {
    fn drop(&mut self) {
        // Abort the mainloop task; kill_on_drop on the child handles OS-level cleanup.
        self.mainloop_handle.abort();
    }
}

fn marked_string_to_text(ms: MarkedString) -> String {
    match ms {
        MarkedString::String(s)          => s,
        MarkedString::LanguageString(ls) => ls.value,
    }
}

fn client_capabilities() -> ClientCapabilities {
    ClientCapabilities {
        text_document: Some(TextDocumentClientCapabilities {
            publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                related_information: Some(true),
                ..Default::default()
            }),
            hover: Some(lsp_types::HoverClientCapabilities {
                content_format: Some(vec![MarkupKind::PlainText]),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}
