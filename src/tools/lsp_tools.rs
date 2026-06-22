use anyhow::{Context, Result};
use async_lsp::lsp_types;
use async_trait::async_trait;
use super::Tool;

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

pub struct GetDiagnosticsTool;

#[async_trait]
impl Tool for GetDiagnosticsTool {
    fn name(&self) -> &str { "get_diagnostics" }

    fn description(&self) -> &str {
        "Get compiler errors and warnings for a source file from the language server. \
         Returns the same errors as `cargo check` / `tsc --noEmit` but without a full compile. \
         Use this after editing a file to confirm it is correct before moving on. \
         Requires the language server to be installed (rust-analyzer, typescript-language-server, pylsp, gopls)."
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
            .ok_or_else(|| anyhow::anyhow!("LSP not initialized"))?;

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut mgr = lsp_arc.lock().await;
                let client  = mgr.client_for(lang).await?;
                client.open_file(&abs_str, &content, lang)?;
                // Wait briefly for publishDiagnostics notification from server.
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let diags = client.cached_diags(&abs_str);
                Ok(format_diagnostics(&abs_str, &diags))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

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
