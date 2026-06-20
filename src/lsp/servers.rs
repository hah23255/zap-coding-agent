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

pub fn language_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "rs"                  => "rust",
        "ts" | "tsx"          => "typescript",
        "js" | "jsx" | "mjs" => "javascript",
        "py"                  => "python",
        "go"                  => "go",
        _                     => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_detection() {
        assert_eq!(language_for_path("src/main.rs"),  "rust");
        assert_eq!(language_for_path("index.ts"),     "typescript");
        assert_eq!(language_for_path("app.tsx"),      "typescript");
        assert_eq!(language_for_path("main.go"),      "go");
        assert_eq!(language_for_path("script.py"),    "python");
        assert_eq!(language_for_path("unknown.xyz"),  "unknown");
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
