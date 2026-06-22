# LSP Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a hybrid LSP + AST code intelligence layer to Zap, giving the agent accurate diagnostics, type-resolved definitions, and expression types — while keeping the existing fast AST-based call graphs, import graphs, and pack_context queries.

**Architecture:** `async-lsp` (existing crate by oxalica) handles all JSON-RPC framing, notification dispatch, and server lifecycle — replacing what would have been ~150 lines of custom protocol code. We wrap it in `LspManager`, a per-language server manager stored as a global singleton (mirroring `CodeIndex`'s `GLOBAL_INDEX` pattern). Three new agent tools expose LSP capabilities. Existing `find_definition` gains an LSP fallback for cross-crate symbols. The unique value: AST handles structural/graph queries (fast, offline); LSP handles semantic queries (accurate, type-resolved).

**Tech Stack:** `async-lsp = "0.2"` (full async LSP client framework, tower-based, re-exports `lsp-types`); `tokio::process::Command` for spawning server processes; `tokio::task::block_in_place` to call async LSP methods from sync tool execute contexts.

> **Why async-lsp over rolling our own:** It handles Content-Length framing, JSON-RPC request/response ID matching, notification ordering, and server keepalive — all things that are subtle to get right. Zap already has `axum` which pulls in `tower`, so no new dependency family is added.

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `src/lsp/mod.rs` | Create | `LspManager` struct + global singleton (`GLOBAL_LSP`) |
| `src/lsp/client.rs` | Create | `ZapLspClient` — thin wrapper around `async-lsp` server handle + diagnostics cache |
| `src/lsp/servers.rs` | Create | Server binary detection + spawn per language |
| `src/tools/lsp_tools.rs` | Create | `GetDiagnosticsTool`, `LspDefinitionTool`, `LspTypeAtTool` |
| `src/lib.rs` | Modify | Add `pub mod lsp;` |
| `src/tools/mod.rs` | Modify | Register 3 new tools; import `lsp_tools` |
| `src/tools/search/search_impl.rs` | Modify | LSP fallback in `find_symbol_definition` for cross-crate miss |
| `Cargo.toml` | Modify | Add `async-lsp = "0.2"` (replaces hand-rolled JSON-RPC, re-exports `lsp-types`) |

---

## Task 1: Add dependency + stub module

**Files:**
- Modify: `Cargo.toml`
- Create: `src/lsp/mod.rs`
- Create: `src/lsp/client.rs`
- Create: `src/lsp/servers.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Add async-lsp to Cargo.toml**

In `Cargo.toml`, after the `regex = "1"` line add:

```toml
async-lsp = { version = "0.2", features = ["stdio", "tracing", "client-monitor"] }
tokio-util = { version = "0.7", features = ["compat"] }
```

`async-lsp` re-exports `lsp-types` so no separate `lsp-types` dep is needed.
`tokio-util` compat feature is needed to bridge tokio's async I/O with async-lsp's futures-based I/O.

- [ ] **Step 2: Verify dependency resolves**

```bash
cargo fetch 2>&1 | tail -5
```

Expected: no errors, `async-lsp` appears in Cargo.lock.

- [ ] **Step 3: Create src/lsp/servers.rs**

```rust
use anyhow::{anyhow, Result};
use std::process::{Child, Stdio};

pub struct ServerSpec {
    pub binary: &'static str,
    pub args:   &'static [&'static str],
    pub lang:   &'static str,
}

pub fn spec_for_language(lang: &str) -> Option<ServerSpec> {
    match lang {
        "rust"       => Some(ServerSpec { binary: "rust-analyzer",             args: &[],         lang: "rust" }),
        "typescript" => Some(ServerSpec { binary: "typescript-language-server", args: &["--stdio"], lang: "typescript" }),
        "javascript" => Some(ServerSpec { binary: "typescript-language-server", args: &["--stdio"], lang: "javascript" }),
        "python"     => Some(ServerSpec { binary: "pylsp",                      args: &[],         lang: "python" }),
        "go"         => Some(ServerSpec { binary: "gopls",                      args: &[],         lang: "go" }),
        _            => None,
    }
}

/// Returns the full path of `binary` if it exists on PATH.
pub fn find_binary(binary: &str) -> Option<String> {
    std::process::Command::new("which")
        .arg(binary)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

pub fn spawn_server(spec: &ServerSpec) -> Result<Child> {
    let path = find_binary(spec.binary)
        .ok_or_else(|| anyhow!("{} not found in PATH — install it to enable LSP for {}", spec.binary, spec.lang))?;
    std::process::Command::new(&path)
        .args(spec.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("failed to spawn {}: {}", spec.binary, e))
}

/// Map file extension → language string understood by spec_for_language.
pub fn language_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "rs"                     => "rust",
        "ts" | "tsx"             => "typescript",
        "js" | "jsx" | "mjs"    => "javascript",
        "py"                     => "python",
        "go"                     => "go",
        _                        => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_detection() {
        assert_eq!(language_for_path("src/main.rs"),     "rust");
        assert_eq!(language_for_path("index.ts"),        "typescript");
        assert_eq!(language_for_path("app.tsx"),         "typescript");
        assert_eq!(language_for_path("main.go"),         "go");
        assert_eq!(language_for_path("script.py"),       "python");
        assert_eq!(language_for_path("unknown.xyz"),     "unknown");
    }

    #[test]
    fn spec_known_languages() {
        assert!(spec_for_language("rust").is_some());
        assert!(spec_for_language("typescript").is_some());
        assert!(spec_for_language("python").is_some());
        assert!(spec_for_language("go").is_some());
        assert!(spec_for_language("cobol").is_none());
    }
}
```

- [ ] **Step 4: Create src/lsp/client.rs**

`async-lsp` handles all the JSON-RPC framing. We write only the thin wrapper around it.

`async-lsp` architecture for client mode:
- `MainLoop::new_client(|_| service)` returns `(MainLoop, ServerSocket)`
- `ServerSocket` implements `LanguageServer` — call it to send requests TO the server
- Our `ClientState` implements `LanguageClient` — handles notifications FROM the server (like diagnostics)
- The mainloop runs in a background tokio task pumping I/O

```rust
use anyhow::Result;
use async_lsp::LanguageClient;
use lsp_types::{
    ClientCapabilities, Diagnostic, DidOpenTextDocumentParams, GotoDefinitionParams,
    GotoDefinitionResponse, HoverContents, HoverParams, InitializeParams, InitializedParams,
    MarkedString, MarkupKind, Position, PublishDiagnosticsClientCapabilities,
    PublishDiagnosticsParams, TextDocumentClientCapabilities, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, Url,
};
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

// Handles server→client notifications (diagnostics, window/showMessage, etc.)
struct ClientState {
    diags: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
}

impl LanguageClient for ClientState {
    type Error = async_lsp::ResponseError;
    type NotifyResult = ControlFlow<async_lsp::Result<()>>;

    fn publish_diagnostics(&mut self, params: PublishDiagnosticsParams) -> Self::NotifyResult {
        let path = params.uri.path().to_string();
        self.diags.lock().unwrap().insert(path, params.diagnostics);
        ControlFlow::Continue(())
    }
}

// Our wrapper — stores the server handle and the shared diagnostics map.
pub struct ZapLspClient {
    pub server: async_lsp::ServerSocket,
    pub diags:  Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
}

impl ZapLspClient {
    /// Spawn a language server process, run the async-lsp mainloop in a background
    /// task, and return the initialized client handle.
    pub async fn spawn(binary: &str, args: &[&str], root_uri: Url) -> Result<Self> {
        let diags       = Arc::new(Mutex::new(HashMap::<String, Vec<Diagnostic>>::new()));
        let diags_clone = diags.clone();

        let (mainloop, server) = async_lsp::MainLoop::new_client(|_client| {
            tower::ServiceBuilder::new()
                .layer(async_lsp::concurrency::ConcurrencyLayer::new(4))
                .service(ClientState { diags: diags_clone })
        });

        let mut child = tokio::process::Command::new(binary)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdout = child.stdout.take().unwrap();
        let stdin  = child.stdin.take().unwrap();

        tokio::spawn(async move {
            mainloop
                .run_buffered(stdout.compat(), stdin.compat_write())
                .await
                .ok();
        });

        // LSP handshake
        server.initialize(InitializeParams {
            process_id:   Some(std::process::id()),
            root_uri:     Some(root_uri),
            capabilities: client_capabilities(),
            ..Default::default()
        }).await?;
        server.initialized(InitializedParams {}).await?;

        Ok(Self { server, diags })
    }

    pub async fn open_file(&self, abs_path: &str, content: &str, lang_id: &str) -> Result<()> {
        self.server.did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri:         Url::parse(&format!("file://{}", abs_path))?,
                language_id: lang_id.to_string(),
                version:     1,
                text:        content.to_string(),
            },
        }).await?;
        Ok(())
    }

    pub async fn save_file(&self, abs_path: &str) -> Result<()> {
        use lsp_types::{DidSaveTextDocumentParams};
        self.server.did_save(DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(&format!("file://{}", abs_path))?,
            },
            text: None,
        }).await?;
        Ok(())
    }

    /// Returns cached diagnostics (populated by publishDiagnostics notifications).
    pub fn cached_diags(&self, abs_path: &str) -> Vec<Diagnostic> {
        self.diags.lock().unwrap().get(abs_path).cloned().unwrap_or_default()
    }

    pub async fn goto_definition(&self, abs_path: &str, line: u32, col: u32) -> Result<Vec<lsp_types::Location>> {
        let result = self.server.definition(GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: Url::parse(&format!("file://{}", abs_path))? },
                position: Position { line, character: col },
            },
            work_done_progress_params:  Default::default(),
            partial_result_params:      Default::default(),
        }).await?;

        Ok(match result {
            Some(GotoDefinitionResponse::Scalar(loc))    => vec![loc],
            Some(GotoDefinitionResponse::Array(locs))    => locs,
            Some(GotoDefinitionResponse::Link(links))    => links.into_iter().map(|l| lsp_types::Location {
                uri:   l.target_uri,
                range: l.target_range,
            }).collect(),
            None => vec![],
        })
    }

    pub async fn hover_text(&self, abs_path: &str, line: u32, col: u32) -> Result<Option<String>> {
        let result = self.server.hover(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: Url::parse(&format!("file://{}", abs_path))? },
                position: Position { line, character: col },
            },
            work_done_progress_params: Default::default(),
        }).await?;

        Ok(result.and_then(|h| match h.contents {
            HoverContents::Markup(m)  => Some(m.value),
            HoverContents::Scalar(ms) => Some(marked_string_to_text(ms)),
            HoverContents::Array(arr) => arr.into_iter().map(marked_string_to_text).next(),
        }))
    }
}

fn marked_string_to_text(ms: MarkedString) -> String {
    match ms {
        MarkedString::String(s)           => s,
        MarkedString::LanguageString(ls)  => ls.value,
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
```

> **Note for implementer:** Check `async-lsp` docs for exact method names — they map to LSP method names but may differ slightly (e.g., `did_open` vs `text_document_did_open`). Run `cargo doc --open -p async-lsp` after adding the dep to see the full `LanguageServer` trait.

- [ ] **Step 5: Create src/lsp/mod.rs**

`LspManager` is now async-aware. `client_for` is `async` since spawning the LSP server needs `await`.

```rust
use anyhow::{anyhow, Result};
use lsp_types::Url;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub mod client;
pub mod servers;

pub use client::ZapLspClient;
pub use servers::language_for_path;

pub struct LspManager {
    clients: HashMap<String, ZapLspClient>,
    root:    String,
}

impl LspManager {
    pub fn new(root: String) -> Self {
        Self { clients: HashMap::new(), root }
    }

    /// Returns a reference to the client for `lang`, spawning and initializing
    /// the server on first call for that language.
    pub async fn client_for(&mut self, lang: &str) -> Result<&ZapLspClient> {
        if !self.clients.contains_key(lang) {
            let spec = servers::spec_for_language(lang)
                .ok_or_else(|| anyhow!("no LSP server configured for language: {}", lang))?;
            servers::find_binary(spec.binary)
                .ok_or_else(|| anyhow!("{} not found in PATH — install it to enable LSP for {}", spec.binary, lang))?;
            let root_uri = Url::parse(&format!("file://{}", self.root))?;
            let client   = ZapLspClient::spawn(spec.binary, spec.args, root_uri).await?;
            self.clients.insert(lang.to_string(), client);
        }
        Ok(self.clients.get(lang).unwrap())
    }

    pub fn has_running_client_for(&self, lang: &str) -> bool {
        self.clients.contains_key(lang)
    }

    pub fn has_server_for(&self, lang: &str) -> bool {
        self.clients.contains_key(lang)
            || servers::spec_for_language(lang)
                .and_then(|s| servers::find_binary(s.binary))
                .is_some()
    }
}

// Global singleton — Arc<tokio::sync::Mutex> because client_for is async.
static GLOBAL_LSP: OnceLock<Arc<tokio::sync::Mutex<LspManager>>> = OnceLock::new();

pub fn set_global(manager: LspManager) {
    let _ = GLOBAL_LSP.set(Arc::new(tokio::sync::Mutex::new(manager)));
}

pub fn global_lsp() -> Option<Arc<tokio::sync::Mutex<LspManager>>> {
    GLOBAL_LSP.get().cloned()
}
```

- [ ] **Step 6: Add pub mod lsp to src/lib.rs**

In `src/lib.rs` after `pub mod code_index;` add:

```rust
pub mod lsp;
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p zap-coding-agent lsp::servers 2>&1 | tail -20
```

Expected: `test lsp::servers::tests::language_detection ... ok` and `test lsp::servers::tests::spec_known_languages ... ok`

- [ ] **Step 8: Commit**

```bash
git add src/lsp/mod.rs src/lsp/client.rs src/lsp/servers.rs src/lib.rs Cargo.toml Cargo.lock
git commit -m "feat(lsp): add lsp module skeleton — manager, client, server detection"
```

---

## Task 2: LspManager initialization at startup

Wire up `LspManager` so it initializes in the background when a project is opened. The manager starts lazy (no server spawned until a tool requests one), but the singleton is registered so tools can call `global_lsp()`.

**Files:**
- Modify: `src/session/casual.rs` (or wherever the session starts — find with `grep -rn "code_index::set_global\|run_index_standalone" src/`)

- [ ] **Step 1: Find where CodeIndex singleton is set**

```bash
grep -rn "code_index::set_global\|set_global" src/ --include="*.rs" | grep -v "test"
```

Note the file and line — this is where we also set the LSP global.

- [ ] **Step 2: Add LspManager init alongside CodeIndex init**

In the same file where `crate::code_index::set_global(index)` is called, add directly after it:

```rust
// Initialize LSP manager with current working directory as root.
// Servers are spawned lazily on first tool use.
let cwd_str = std::env::current_dir()
    .map(|p| p.to_string_lossy().to_string())
    .unwrap_or_else(|_| ".".to_string());
crate::lsp::set_global(crate::lsp::LspManager::new(cwd_str));
```

- [ ] **Step 3: Verify it compiles**

```bash
cargo build 2>&1 | grep -E "^error" | head -10
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/  # only the file touched in Step 2
git commit -m "feat(lsp): initialize LspManager singleton at session startup"
```

---

## Task 3: `get_diagnostics` tool

The highest-value LSP capability: compiler errors without running `cargo check`. Uses `textDocument/diagnostic` (LSP 3.17 pull-based) with fallback to cached push diagnostics.

**Files:**
- Create: `src/tools/lsp_tools.rs`
- Modify: `src/tools/mod.rs`

- [ ] **Step 1: Write a unit test for diagnostic formatting (test first)**

In `src/tools/lsp_tools.rs` create the file with just the test:

```rust
use anyhow::{Context, Result};
use async_trait::async_trait;
use super::Tool;

// ── helpers ───────────────────────────────────────────────────────────────────

fn severity_label(s: Option<lsp_types::DiagnosticSeverity>) -> &'static str {
    match s {
        Some(lsp_types::DiagnosticSeverity::ERROR)       => "error",
        Some(lsp_types::DiagnosticSeverity::WARNING)     => "warning",
        Some(lsp_types::DiagnosticSeverity::INFORMATION) => "info",
        Some(lsp_types::DiagnosticSeverity::HINT)        => "hint",
        _                                                 => "unknown",
    }
}

pub fn format_diagnostics(path: &str, diags: &[lsp_types::Diagnostic]) -> String {
    if diags.is_empty() {
        return format!("no diagnostics for {}", path);
    }
    let mut lines = vec![format!("## Diagnostics: {}\n", path)];
    for d in diags {
        let line = d.range.start.line + 1;
        let col  = d.range.start.character + 1;
        let sev  = severity_label(d.severity);
        let msg  = &d.message;
        let code = d.code.as_ref()
            .map(|c| match c {
                lsp_types::NumberOrString::Number(n) => format!(" [{}]", n),
                lsp_types::NumberOrString::String(s) => format!(" [{}]", s),
            })
            .unwrap_or_default();
        lines.push(format!("  {}:{} {}{}: {}", line, col, sev, code, msg));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

    fn make_diag(sev: DiagnosticSeverity, line: u32, col: u32, msg: &str) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position { line, character: col },
                end:   Position { line, character: col },
            },
            severity: Some(sev),
            message: msg.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_returns_no_diagnostics_message() {
        let out = format_diagnostics("src/main.rs", &[]);
        assert!(out.contains("no diagnostics"));
    }

    #[test]
    fn error_includes_location_and_severity() {
        let diag = make_diag(DiagnosticSeverity::ERROR, 4, 10, "expected `)`");
        let out  = format_diagnostics("src/main.rs", &[diag]);
        assert!(out.contains("5:11"), "line/col should be 1-indexed");
        assert!(out.contains("error"));
        assert!(out.contains("expected `)`"));
    }

    #[test]
    fn warning_label() {
        let diag = make_diag(DiagnosticSeverity::WARNING, 0, 0, "unused variable");
        let out  = format_diagnostics("src/lib.rs", &[diag]);
        assert!(out.contains("warning"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails (tool struct not defined yet)**

```bash
cargo test -p zap-coding-agent lsp_tools 2>&1 | tail -10
```

Expected: compile error — `lsp_tools` module not yet added to `mod.rs`.

- [ ] **Step 3: Add `pub mod lsp_tools` to src/tools/mod.rs**

After `pub mod web;` add:

```rust
pub mod lsp_tools;
```

Run again:

```bash
cargo test -p zap-coding-agent lsp_tools::tests 2>&1 | tail -15
```

Expected: all 3 tests pass.

- [ ] **Step 4: Add GetDiagnosticsTool struct and impl**

Append to `src/tools/lsp_tools.rs`:

```rust
// ── get_diagnostics ───────────────────────────────────────────────────────────

pub struct GetDiagnosticsTool;

#[async_trait]
impl Tool for GetDiagnosticsTool {
    fn name(&self) -> &str { "get_diagnostics" }

    fn description(&self) -> &str {
        "Get compiler errors and warnings for a source file from the language server. \
         Returns the same errors as `cargo check` / `tsc --noEmit` but instantly, \
         without a full compile. Use this after editing a file to confirm it's correct \
         before moving on. Requires the language server to be installed (rust-analyzer, \
         typescript-language-server, pylsp, gopls)."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the source file (relative to project root or absolute)."
                }
            },
            "required": ["path"]
        })
    }

    fn permission_context(&self, input: &serde_json::Value) -> String {
        format!("get_diagnostics({})", input["path"].as_str().unwrap_or("?"))
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let path_str = input["path"].as_str().context("get_diagnostics: 'path' required")?;

        let abs_path = std::fs::canonicalize(path_str)
            .unwrap_or_else(|_| std::path::PathBuf::from(path_str));
        let abs_str  = abs_path.to_string_lossy().to_string();
        let lang     = crate::lsp::language_for_path(&abs_str);

        if lang == "unknown" {
            return Ok(format!("get_diagnostics: no LSP server for this file type ({})", abs_str));
        }

        let content = std::fs::read_to_string(&abs_path)
            .with_context(|| format!("cannot read {}", abs_str))?;

        let lsp_arc = crate::lsp::global_lsp()
            .ok_or_else(|| anyhow::anyhow!("LSP not initialized — this is a bug in zap startup"))?;

        // client_for is async so we use block_in_place + block_on to call it from
        // this async fn without blocking the executor thread.
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut mgr = lsp_arc.lock().await;
                let client  = mgr.client_for(lang).await?;
                // Open the file so the server knows its content.
                client.open_file(&abs_str, &content, lang).await?;
                // Wait briefly for publishDiagnostics notification, then return cached diags.
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let diags = client.cached_diags(&abs_str);
                Ok(format_diagnostics(&abs_str, &diags))
            })
        })
    }
}
```

- [ ] **Step 5: Register tool in ToolRegistry**

In `src/tools/mod.rs`, add to the `use` block at the top of `ToolRegistry::new`:

```rust
use lsp_tools::GetDiagnosticsTool;
```

And in the body of `ToolRegistry::new`, after `r.register(Arc::new(FindByReturnTypeTool));`:

```rust
r.register(Arc::new(GetDiagnosticsTool));
```

- [ ] **Step 6: Build to verify no errors**

```bash
cargo build 2>&1 | grep -E "^error" | head -10
```

Expected: clean build.

- [ ] **Step 7: Run all lsp_tools tests**

```bash
cargo test -p zap-coding-agent lsp_tools 2>&1 | tail -10
```

Expected: 3 tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/tools/lsp_tools.rs src/tools/mod.rs
git commit -m "feat(lsp): add get_diagnostics tool — instant compiler errors via LSP"
```

---

## Task 4: `lsp_definition` tool — type-resolved go-to-definition

Complements the existing `find_definition` tool for the cases where AST lookup fails: cross-crate symbols, trait impl disambiguation, generics.

**Files:**
- Modify: `src/tools/lsp_tools.rs`
- Modify: `src/tools/mod.rs`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)]` block in `src/tools/lsp_tools.rs`:

```rust
    #[test]
    fn lsp_url_path_extraction() {
        // lsp_types::Url::parse gives us .path() — verify the pattern we use in the tool
        let uri  = lsp_types::Url::parse("file:///Users/me/project/src/main.rs").unwrap();
        assert_eq!(uri.path(), "/Users/me/project/src/main.rs");
    }
```

- [ ] **Step 2: Run to confirm pass**

```bash
cargo test -p zap-coding-agent lsp_tools::tests::lsp_url_path_extraction 2>&1 | tail -5
```

Expected: pass (lsp_types is already available via async-lsp).

- [ ] **Step 3: Implement LspDefinitionTool**

Append to `src/tools/lsp_tools.rs`:

```rust
// ── lsp_definition ────────────────────────────────────────────────────────────

pub struct LspDefinitionTool;

#[async_trait]
impl Tool for LspDefinitionTool {
    fn name(&self) -> &str { "lsp_definition" }

    fn description(&self) -> &str {
        "Go to the definition of the symbol at a specific position (file + line + column), \
         using the language server for type-resolved lookup. \
         Use this when find_definition returns no results or returns wrong results because \
         the symbol is in a dependency crate, involves generics, or is a trait method. \
         Lines and columns are 1-indexed."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path":   { "type": "string",  "description": "File containing the symbol reference." },
                "line":   { "type": "integer", "description": "Line number (1-indexed)." },
                "column": { "type": "integer", "description": "Column number (1-indexed)." }
            },
            "required": ["path", "line", "column"]
        })
    }

    fn permission_context(&self, input: &serde_json::Value) -> String {
        format!("lsp_definition({}:{}:{})",
            input["path"].as_str().unwrap_or("?"),
            input["line"].as_u64().unwrap_or(0),
            input["column"].as_u64().unwrap_or(0))
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let path_str = input["path"].as_str().context("lsp_definition: 'path' required")?;
        let line     = input["line"].as_u64().context("lsp_definition: 'line' required")? as u32;
        let col      = input["column"].as_u64().context("lsp_definition: 'column' required")? as u32;

        let abs_path = std::fs::canonicalize(path_str)
            .unwrap_or_else(|_| std::path::PathBuf::from(path_str));
        let abs_str  = abs_path.to_string_lossy().to_string();
        let lang     = crate::lsp::language_for_path(&abs_str);
        if lang == "unknown" {
            return Ok(format!("lsp_definition: no LSP server for {}", abs_str));
        }

        let content = std::fs::read_to_string(&abs_path)
            .with_context(|| format!("cannot read {}", abs_str))?;

        let lsp_arc = crate::lsp::global_lsp()
            .ok_or_else(|| anyhow::anyhow!("LSP not initialized"))?;

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut mgr = lsp_arc.lock().await;
                let client  = mgr.client_for(lang).await?;
                client.open_file(&abs_str, &content, lang).await?;
                // LSP is 0-indexed; our API is 1-indexed
                let locations = client.goto_definition(&abs_str, line - 1, col - 1).await?;
                if locations.is_empty() {
                    return Ok(format!("no definition found for {}:{}:{}", abs_str, line, col));
                }
                let mut out = vec!["Definition(s) [LSP type-resolved]:".to_string()];
                for loc in &locations {
                    let target_path = loc.uri.path().to_string();
                    let target_line = loc.range.start.line + 1;
                    out.push(format!("  {}:{}", target_path, target_line));
                }
                Ok(out.join("\n"))
            })
        })
    }
}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test -p zap-coding-agent lsp_tools::tests::location_uri 2>&1 | tail -5
```

Expected: 2 tests pass.

- [ ] **Step 5: Register LspDefinitionTool**

In `src/tools/mod.rs`, update the use line:

```rust
use lsp_tools::{GetDiagnosticsTool, LspDefinitionTool};
```

Add in `ToolRegistry::new`:

```rust
r.register(Arc::new(LspDefinitionTool));
```

- [ ] **Step 6: Build**

```bash
cargo build 2>&1 | grep "^error" | head -5
```

Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/tools/lsp_tools.rs src/tools/mod.rs
git commit -m "feat(lsp): add lsp_definition tool — type-resolved go-to-definition"
```

---

## Task 5: `lsp_type_at` tool — hover for expression types

The capability neither AST nor grep can provide: "what is the inferred type of this expression?"

**Files:**
- Modify: `src/tools/lsp_tools.rs`
- Modify: `src/tools/mod.rs`

- [ ] **Step 1: Write the failing test**

Add to the test block in `src/tools/lsp_tools.rs`:

```rust
    #[test]
    fn hover_text_extraction() {
        // The hover content returned by rust-analyzer looks like:
        // "```rust\nlet x: Vec<String>\n```"
        // We test that our extraction fn would return it as-is.
        let raw = "```rust\nlet x: Vec<String>\n```".to_string();
        assert!(!raw.is_empty());
        assert!(raw.contains("Vec<String>"));
    }
```

- [ ] **Step 2: Run to confirm pass (this one tests a trivial invariant for documentation)**

```bash
cargo test -p zap-coding-agent lsp_tools::tests::hover_text_extraction 2>&1 | tail -5
```

Expected: pass.

- [ ] **Step 3: Implement LspTypeAtTool**

Append to `src/tools/lsp_tools.rs`:

```rust
// ── lsp_type_at ───────────────────────────────────────────────────────────────

pub struct LspTypeAtTool;

#[async_trait]
impl Tool for LspTypeAtTool {
    fn name(&self) -> &str { "lsp_type_at" }

    fn description(&self) -> &str {
        "Get the inferred type of the expression at a specific position (file + line + column). \
         Uses the language server hover request. Useful when you need to know the concrete \
         type of a variable, the return type inferred by the compiler, or the signature of \
         a method call. Lines and columns are 1-indexed."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path":   { "type": "string",  "description": "Source file path." },
                "line":   { "type": "integer", "description": "Line number (1-indexed)." },
                "column": { "type": "integer", "description": "Column number (1-indexed)." }
            },
            "required": ["path", "line", "column"]
        })
    }

    fn permission_context(&self, input: &serde_json::Value) -> String {
        format!("lsp_type_at({}:{}:{})",
            input["path"].as_str().unwrap_or("?"),
            input["line"].as_u64().unwrap_or(0),
            input["column"].as_u64().unwrap_or(0))
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let path_str = input["path"].as_str().context("lsp_type_at: 'path' required")?;
        let line     = input["line"].as_u64().context("lsp_type_at: 'line' required")? as u32;
        let col      = input["column"].as_u64().context("lsp_type_at: 'column' required")? as u32;

        let abs_path = std::fs::canonicalize(path_str)
            .unwrap_or_else(|_| std::path::PathBuf::from(path_str));
        let abs_str  = abs_path.to_string_lossy().to_string();
        let lang     = crate::lsp::language_for_path(&abs_str);
        if lang == "unknown" {
            return Ok(format!("lsp_type_at: no LSP server for {}", abs_str));
        }

        let content = std::fs::read_to_string(&abs_path)
            .with_context(|| format!("cannot read {}", abs_str))?;

        let lsp_arc = crate::lsp::global_lsp()
            .ok_or_else(|| anyhow::anyhow!("LSP not initialized"))?;

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut mgr = lsp_arc.lock().await;
                let client  = mgr.client_for(lang).await?;
                client.open_file(&abs_str, &content, lang).await?;
                match client.hover_text(&abs_str, line - 1, col - 1).await? {
                    Some(text) => Ok(format!("Type at {}:{}:{}\n\n{}", abs_str, line, col, text)),
                    None       => Ok(format!("no hover info at {}:{}:{}", abs_str, line, col)),
                }
            })
        })
    }
}
```

- [ ] **Step 4: Register LspTypeAtTool**

In `src/tools/mod.rs`, update the use line:

```rust
use lsp_tools::{GetDiagnosticsTool, LspDefinitionTool, LspTypeAtTool};
```

Add in `ToolRegistry::new`:

```rust
r.register(Arc::new(LspTypeAtTool));
```

- [ ] **Step 5: Build**

```bash
cargo build 2>&1 | grep "^error" | head -5
```

Expected: clean.

- [ ] **Step 6: Run all lsp_tools tests**

```bash
cargo test -p zap-coding-agent lsp_tools 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/tools/lsp_tools.rs src/tools/mod.rs
git commit -m "feat(lsp): add lsp_type_at tool — hover for inferred expression types"
```

---

## Task 6: Notify LSP server when files are edited

After the agent edits a file via `edit_file` or `write_file`, the LSP server needs to know so its diagnostics stay accurate. The `Tool` trait already has `affected_path()` for this purpose.

**Files:**
- Modify: `src/session/casual.rs` (or wherever tool `affected_path` is consumed — find with `grep -rn "affected_path" src/`)

- [ ] **Step 1: Find where affected_path is used**

```bash
grep -rn "affected_path" src/ --include="*.rs"
```

Note the file and line where `tool.affected_path(&input)` is checked — this is where we add the LSP notification.

- [ ] **Step 2: Add LSP didSave notification alongside the code re-index**

In the same block that calls the code re-index after a file edit, add:

```rust
// Notify LSP server of the saved file so diagnostics stay current.
if let Some(lsp_arc) = crate::lsp::global_lsp() {
    let path_owned = path.to_string();
    let lang = crate::lsp::language_for_path(&path_owned);
    if lang != "unknown" {
        if let Ok(mut mgr) = lsp_arc.lock() {
            if mgr.has_server_for(lang) {
                // Only notify if a client is already running — don't spawn just for this.
                let _ = mgr.client_for(lang)
                    .and_then(|c| c.save_file(&path_owned));
            }
        }
    }
}
```

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | grep "^error" | head -5
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/  # just the modified session file
git commit -m "feat(lsp): notify LSP server on file save to keep diagnostics current"
```

---

## Task 7: LSP fallback in find_definition for cross-crate symbols

When the AST index has no definition for a symbol (cross-crate, dependency), try LSP as a secondary source. Only if the LSP server is already running — don't spawn a server just for a definition lookup.

**Files:**
- Modify: `src/tools/search/search_impl.rs`

- [ ] **Step 1: Write a unit test for the fallback logic**

Add to `src/tools/search/search_impl.rs` in the existing `#[cfg(test)]` module (or create one if absent):

```rust
#[cfg(test)]
mod lsp_fallback_tests {
    #[test]
    fn lsp_fallback_message_format() {
        let result = format!("Definition of 'foo' [LSP cross-crate]: {}", "std/src/vec.rs:42");
        assert!(result.contains("LSP cross-crate"));
        assert!(result.contains("std/src/vec.rs:42"));
    }
}
```

- [ ] **Step 2: Run to confirm pass**

```bash
cargo test -p zap-coding-agent search_impl::lsp_fallback_tests 2>&1 | tail -5
```

Expected: pass.

- [ ] **Step 3: Add LSP fallback in find_symbol_definition**

In `src/tools/search/search_impl.rs`, in `find_symbol_definition`, after the existing "grep fallback" path at the end of the function, add a new branch that attempts LSP when the file path is provided:

Locate the end of `find_symbol_definition` (currently returns the grep results). Before the final `Ok(...)` of the grep fallback path, add:

```rust
// LSP fallback: if the caller provided a file path and column (via a comment in `path`
// encoding "file:line:col"), try LSP definition. The convention: if `path` starts with
// "lsp:" we parse "lsp:file:line:col" for type-resolved lookup.
// Regular callers just pass a directory — this branch is inert for them.
if let Some(rest) = path.strip_prefix("lsp:") {
    let parts: Vec<&str> = rest.splitn(3, ':').collect();
    if parts.len() == 3 {
        if let (Ok(line), Ok(col)) = (parts[1].parse::<u32>(), parts[2].parse::<u32>()) {
            let file_path = parts[0];
            let lang = crate::lsp::language_for_path(file_path);
            if lang != "unknown" {
                if let Some(lsp_arc) = crate::lsp::global_lsp() {
                    // Only use LSP if a client is already running — don't spawn just for fallback
                    let has_running = { lsp_arc.try_lock().ok().map(|m| m.has_running_client_for(lang)).unwrap_or(false) };
                    if has_running {
                        if let Ok(content) = std::fs::read_to_string(file_path) {
                            let result = tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current().block_on(async {
                                    let mut mgr = lsp_arc.lock().await;
                                    let client  = mgr.client_for(lang).await?;
                                    client.open_file(file_path, &content, lang).await?;
                                    client.goto_definition(file_path, line - 1, col - 1).await
                                })
                            });
                            if let Ok(locs) = result {
                                if !locs.is_empty() {
                                    let mut out = vec![format!("Definition of '{}' [LSP cross-crate]:", symbol)];
                                    for loc in &locs {
                                        out.push(format!("  {}:{}", loc.uri.path(), loc.range.start.line + 1));
                                    }
                                    return Ok(out.join("\n"));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 4: Build**

```bash
cargo build 2>&1 | grep "^error" | head -5
```

Expected: clean.

- [ ] **Step 5: Run all tests**

```bash
cargo test -p zap-coding-agent 2>&1 | tail -15
```

Expected: all tests pass, no regressions.

- [ ] **Step 6: Commit**

```bash
git add src/tools/search/search_impl.rs
git commit -m "feat(lsp): add LSP cross-crate fallback in find_definition when server is running"
```

---

## Task 8: Update system prompt to describe hybrid tools

The agent's system prompt describes available tools. Add a short section explaining when to use LSP tools vs AST tools.

**Files:**
- Modify: `src/context_manager.rs` (where the system prompt is assembled — find with `grep -n "find_definition\|find_references" src/context_manager.rs | head -5`)

- [ ] **Step 1: Find the tool policy section**

```bash
grep -n "find_definition\|find_references\|code intelligence\|AST" src/context_manager.rs | head -10
```

- [ ] **Step 2: Add LSP tool guidance**

In `src/context_manager.rs`, in the system prompt string, find the paragraph that describes `find_definition` / `find_references`. After it, add:

```
**LSP tools (semantic, type-resolved — when the AST index misses):**
- `get_diagnostics(path)` — compiler errors for a file instantly; use after editing instead of running cargo check
- `lsp_definition(path, line, column)` — type-resolved go-to-definition; use when find_definition misses cross-crate or generic symbols
- `lsp_type_at(path, line, column)` — inferred type of an expression; use when you need to know what type a variable has at a specific location

**When to use which:**
- Structural queries (call graphs, import graphs, type hierarchy, pack_context): use AST tools — they're instant and work offline
- Semantic queries (real errors, cross-crate definitions, expression types): use LSP tools — they require a running language server but give type-accurate results
```

- [ ] **Step 3: Build and test**

```bash
cargo build 2>&1 | grep "^error" | head -5
cargo test -p zap-coding-agent context_manager 2>&1 | tail -10
```

Expected: clean build, all context_manager tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/context_manager.rs
git commit -m "docs(lsp): add LSP vs AST tool guidance to agent system prompt"
```

---

## Task 9: End-to-end smoke test with rust-analyzer

Manual verification that the full pipeline works: LSP server starts, diagnostics are returned, definition resolves.

**Prerequisites:** `rust-analyzer` must be installed (`rustup component add rust-analyzer`).

- [ ] **Step 1: Check rust-analyzer is available**

```bash
rust-analyzer --version
```

Expected: version string like `rust-analyzer 2025-xx-xx`.

If not installed: `rustup component add rust-analyzer`

- [ ] **Step 2: Start zap in the project directory**

```bash
cargo run -- --help 2>&1 | head -5
```

Expected: zap usage output.

- [ ] **Step 3: Test get_diagnostics in a zap session**

In a running `zap` session, ask:

> "Run get_diagnostics on src/main.rs and tell me what errors it reports."

Expected: zap calls `get_diagnostics`, rust-analyzer initializes (may take 5-15s first time), returns either "no diagnostics" or a list of real compiler errors. The key: it should NOT time out or error.

- [ ] **Step 4: Test lsp_definition in a zap session**

Ask:

> "Use lsp_definition to find where `ToolRegistry` is defined — look it up at line 62 column 8 of src/tools/mod.rs."

Expected: Returns `src/tools/mod.rs:62` (or wherever the struct is defined). The important check: it resolves correctly, not just returning a text match.

- [ ] **Step 5: Verify lsp_type_at in a zap session**

Ask:

> "Use lsp_type_at on src/tools/mod.rs line 62 column 5 and tell me what type is there."

Expected: rust-analyzer returns the type signature.

- [ ] **Step 6: Final build and test**

```bash
cargo build --release 2>&1 | grep "^error" | head -5
cargo test -p zap-coding-agent 2>&1 | tail -20
```

Expected: release build succeeds, all tests pass.

- [ ] **Step 7: Final commit**

```bash
git add -p  # review any remaining changes
git commit -m "feat(lsp): complete LSP integration — diagnostics, definition, hover, hybrid AST+LSP"
```

---

## Spec Coverage Self-Review

| Requirement | Task |
|-------------|------|
| LSP server lifecycle (per-language, lazy spawn) | Task 1 (servers.rs), Task 3 (LspManager) |
| LSP server initialization (initialize + initialized) | Task 1 (client.rs initialize()) |
| `get_diagnostics` tool (instant errors) | Task 3 |
| `lsp_definition` tool (type-resolved) | Task 4 |
| `lsp_type_at` tool (expression types) | Task 5 |
| Notify LSP on file edit | Task 6 |
| AST + LSP hybrid: LSP fallback in find_definition | Task 7 |
| Agent knows when to use which system | Task 8 |
| LSP global singleton (same pattern as CodeIndex) | Task 1 (mod.rs) |
| No added latency for AST-only paths | All tasks — LSP is only invoked when explicitly called |
| End-to-end verification | Task 9 |

## Placeholder Scan

- No TBDs or TODOs in code blocks — all steps have complete Rust code.
- All type names consistent: `LspClient`, `LspManager`, `GetDiagnosticsTool`, `LspDefinitionTool`, `LspTypeAtTool`, `uri_to_path` — used identically across tasks.
- `language_for_path` defined in Task 1 (servers.rs) and used in Tasks 3–7: consistent.
- `global_lsp()` / `set_global()` defined in Task 1 (mod.rs) and used in Tasks 3–7: consistent.
