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
        "java"       => Some(ServerSpec { binary: "jdtls",                      args: &[],         lang: "java" }),
        _            => None,
    }
}

/// Search PATH for `binary` and return its full resolved path, or None if not found.
pub fn find_binary(binary: &str) -> Option<String> {
    std::process::Command::new("which")
        .arg(binary)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

pub fn language_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "rs"                  => "rust",
        "ts" | "tsx"          => "typescript",
        "js" | "jsx" | "mjs" => "javascript",
        "py"                  => "python",
        "go"                  => "go",
        "java"                => "java",
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
        assert_eq!(language_for_path("app.js"),       "javascript");
        assert_eq!(language_for_path("mod.mjs"),      "javascript");
        assert_eq!(language_for_path("Main.java"),    "java");
    }

    #[test]
    fn spec_known_languages() {
        assert!(spec_for_language("rust").is_some());
        assert!(spec_for_language("typescript").is_some());
        assert!(spec_for_language("javascript").is_some());
        assert!(spec_for_language("python").is_some());
        assert!(spec_for_language("go").is_some());
        assert!(spec_for_language("java").is_some());
        assert!(spec_for_language("cobol").is_none());
    }
}
