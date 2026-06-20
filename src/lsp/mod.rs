use anyhow::{anyhow, Result};
use async_lsp::lsp_types::Url;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

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

    pub async fn client_for(&mut self, lang: &str) -> Result<&ZapLspClient> {
        // Evict a dead client so we respawn on the next call.
        if let Some(c) = self.clients.get(lang) {
            if !c.is_alive() {
                self.clients.remove(lang);
            }
        }

        if !self.clients.contains_key(lang) {
            let spec = servers::spec_for_language(lang)
                .ok_or_else(|| anyhow!("no LSP server configured for language: {}", lang))?;
            // Resolve the binary once and pass the full path to avoid a TOCTOU race.
            let resolved = servers::find_binary(spec.binary)
                .ok_or_else(|| anyhow!("{} not found in PATH", spec.binary))?;
            let root_uri = Url::from_file_path(&self.root)
                .map_err(|_| anyhow!("invalid root path: {}", self.root))?;
            let client = ZapLspClient::spawn(&resolved, spec.args, root_uri).await?;
            self.clients.insert(lang.to_string(), client);
        }
        Ok(self.clients.get(lang).unwrap())
    }

    /// Returns true if a live client for `lang` is already connected.
    pub fn has_client_for(&self, lang: &str) -> bool {
        self.clients.get(lang).map(|c| c.is_alive()).unwrap_or(false)
    }

    pub fn has_server_for(&self, lang: &str) -> bool {
        self.has_client_for(lang)
            || servers::spec_for_language(lang)
                .and_then(|s| servers::find_binary(s.binary))
                .is_some()
    }
}

static GLOBAL_LSP: OnceLock<Arc<tokio::sync::Mutex<LspManager>>> = OnceLock::new();

pub fn set_global(manager: LspManager) {
    let _ = GLOBAL_LSP.set(Arc::new(tokio::sync::Mutex::new(manager)));
}

pub fn global_lsp() -> Option<Arc<tokio::sync::Mutex<LspManager>>> {
    GLOBAL_LSP.get().cloned()
}
