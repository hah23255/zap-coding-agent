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
        if !self.clients.contains_key(lang) {
            let spec = servers::spec_for_language(lang)
                .ok_or_else(|| anyhow!("no LSP server configured for language: {}", lang))?;
            servers::find_binary(spec.binary)
                .ok_or_else(|| anyhow!("{} not found in PATH", spec.binary))?;
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

static GLOBAL_LSP: OnceLock<Arc<tokio::sync::Mutex<LspManager>>> = OnceLock::new();

pub fn set_global(manager: LspManager) {
    let _ = GLOBAL_LSP.set(Arc::new(tokio::sync::Mutex::new(manager)));
}

pub fn global_lsp() -> Option<Arc<tokio::sync::Mutex<LspManager>>> {
    GLOBAL_LSP.get().cloned()
}
