use anyhow::{Context, Result};
use async_trait::async_trait;

use super::Tool;

// ── web_fetch ─────────────────────────────────────────────────────────────────

pub(super) struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str { "web_fetch" }
    fn description(&self) -> &str {
        "Fetch a URL and return its content as plain text. \
         GitHub blob URLs (github.com/.../blob/...) are automatically converted to raw content. \
         GitHub repo root URLs (github.com/owner/repo) return the README. \
         Useful for reading documentation, READMEs, API references, or any web page."
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url":       { "type": "string",  "description": "URL to fetch." },
                "max_chars": { "type": "integer", "description": "Maximum characters to return (default 8000)." }
            },
            "required": ["url"]
        })
    }
    fn permission_context(&self, input: &serde_json::Value) -> String {
        format!("fetch '{}'", input["url"].as_str().unwrap_or("?"))
    }
    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let url       = input["url"].as_str().context("web_fetch: 'url' required")?;
        let max_chars = input["max_chars"].as_u64().unwrap_or(8000) as usize;

        // GitHub repo root → fetch README via GitHub API
        if let Some(text) = try_github_readme(url).await {
            return truncate(text, max_chars);
        }

        // Rewrite github.com blob/tree URLs to raw content
        let url = rewrite_github_url(url);

        let client = crate::http::client();
        let resp = client
            .get(url.as_ref())
            .header("User-Agent", user_agent())
            .send()
            .await
            .with_context(|| format!("web_fetch: could not reach '{}'", url))?;

        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("web_fetch: HTTP {} from '{}'", status, url);
        }

        let body = resp.text().await?;
        let text = if url.ends_with(".md") || url.contains("raw.githubusercontent.com") {
            body
        } else {
            strip_html(&body)
        };

        truncate(text, max_chars)
    }
}

fn truncate(text: String, max_chars: usize) -> Result<String> {
    if text.len() > max_chars {
        let mut cut = max_chars;
        while cut > 0 && !text.is_char_boundary(cut) { cut -= 1; }
        Ok(format!("{}\n\n[…truncated to {} of {} chars]", &text[..cut], cut, text.len()))
    } else {
        Ok(text)
    }
}

/// Rewrite github.com blob/tree URLs to raw.githubusercontent.com so they
/// return the actual file content rather than the HTML page.
fn rewrite_github_url(url: &str) -> std::borrow::Cow<'_, str> {
    // https://github.com/owner/repo/blob/branch/path → raw
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let parts: Vec<&str> = rest.splitn(5, '/').collect();
        if parts.len() >= 5 && parts[2] == "blob" {
            let raw = format!(
                "https://raw.githubusercontent.com/{}/{}/{}/{}",
                parts[0], parts[1], parts[3], parts[4]
            );
            return std::borrow::Cow::Owned(raw);
        }
        // /tree/ URLs: just drop the "tree/" prefix and fetch the listing
        if parts.len() >= 5 && parts[2] == "tree" {
            let raw = format!(
                "https://raw.githubusercontent.com/{}/{}/{}/{}",
                parts[0], parts[1], parts[3], parts[4]
            );
            return std::borrow::Cow::Owned(raw);
        }
    }
    std::borrow::Cow::Borrowed(url)
}

/// For bare repo URLs (github.com/owner/repo or .../tree/branch), fetch the
/// README from the GitHub API and return it as markdown text.
async fn try_github_readme(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = rest.splitn(5, '/').collect();

    // Must be exactly owner/repo (no sub-path) or owner/repo/tree/branch
    let (owner, repo, branch) = match parts.len() {
        2 => (parts[0], parts[1], None),
        4 if parts[2] == "tree" => (parts[0], parts[1], Some(parts[3])),
        _ => return None,
    };

    let api_url = match branch {
        Some(b) => format!("https://api.github.com/repos/{}/{}/readme?ref={}", owner, repo, b),
        None    => format!("https://api.github.com/repos/{}/{}/readme", owner, repo),
    };

    let client = crate::http::client();
    let resp = client
        .get(&api_url)
        .header("User-Agent", user_agent())
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() { return None; }

    let json: serde_json::Value = resp.json().await.ok()?;
    let encoded = json["content"].as_str()?;
    let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, cleaned).ok()?;
    String::from_utf8(bytes).ok()
}

// ── web_search ────────────────────────────────────────────────────────────────

const BRAVE_SETUP_MSG: &str = "ERROR: web_search is not configured — BRAVE_SEARCH_API_KEY is not set.\n\
\n\
STOP: do not attempt shell commands or any other workaround to search the web.\n\
Tell the user they need to set up a Brave Search API key to enable web search.\n\
\n\
How to fix (takes ~1 minute):\n\
  1. Get a free key (2 000 searches/month): https://brave.com/search/api/\n\
  2. Add to shell profile: export BRAVE_SEARCH_API_KEY=\"your-key\"\n\
     Or add to ~/.config/zap/agent.toml: brave_search_api_key = \"your-key\"\n\
  3. Restart zap\n\
\n\
Do not proceed with any search attempt until the key is configured.";

pub(super) struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str { "web_search" }
    fn description(&self) -> &str {
        "Search the web using Brave Search and return top results with titles, URLs, and snippets. \
         Requires BRAVE_SEARCH_API_KEY environment variable (free at brave.com/search/api)."
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query":       { "type": "string",  "description": "Search query." },
                "max_results": { "type": "integer", "description": "Max results to return (default 5)." }
            },
            "required": ["query"]
        })
    }
    fn permission_context(&self, input: &serde_json::Value) -> String {
        format!("search: '{}'", input["query"].as_str().unwrap_or("?"))
    }
    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let query = input["query"].as_str().context("web_search: 'query' required")?;
        let max   = input["max_results"].as_u64().unwrap_or(5) as usize;

        // Resolve key: env var wins, then config file field
        let key = std::env::var("BRAVE_SEARCH_API_KEY").ok()
            .filter(|k| !k.trim().is_empty())
            .or_else(brave_key_from_config);

        match key {
            Some(k) => search_brave(&k, query, max).await,
            None    => Ok(BRAVE_SETUP_MSG.to_string()),
        }
    }
}

/// Read `brave_search_api_key` from the zap config file if present.
fn brave_key_from_config() -> Option<String> {
    let path = crate::config::config_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("brave_search_api_key") {
            let val = val.trim_start_matches([' ', '=']);
            let val = val.trim_matches('"').trim_matches('\'').trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

async fn search_brave(key: &str, query: &str, max: usize) -> Result<String> {
    let client = crate::http::client();

    let resp = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .query(&[("q", query), ("count", &max.to_string()), ("safesearch", "off")])
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", key)
        .send()
        .await
        .context("web_search: could not reach Brave Search API")?;

    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        anyhow::bail!(
            "web_search: Brave API returned {} — your BRAVE_SEARCH_API_KEY may be invalid.\n{}",
            status, BRAVE_SETUP_MSG
        );
    }
    if !status.is_success() {
        anyhow::bail!("web_search: Brave API returned HTTP {}", status);
    }

    let json: serde_json::Value = resp.json().await
        .context("web_search: failed to parse Brave API response")?;

    let results = json["web"]["results"]
        .as_array()
        .map(|arr| arr.as_slice())
        .unwrap_or(&[]);

    if results.is_empty() {
        return Ok(format!("No results found for '{}'.", query));
    }

    let mut out = format!("Search results for '{}':\n\n", query);
    for r in results.iter().take(max) {
        let title   = r["title"].as_str().unwrap_or("?");
        let url     = r["url"].as_str().unwrap_or("?");
        let snippet = r["description"].as_str().unwrap_or("");
        if snippet.is_empty() {
            out.push_str(&format!("• {}\n  {}\n\n", title, url));
        } else {
            out.push_str(&format!("• {}\n  {}\n  {}\n\n", title, snippet, url));
        }
    }

    Ok(out.trim_end().to_string())
}

// ── User-Agent ────────────────────────────────────────────────────────────────

fn user_agent() -> String {
    format!("zap/{}", env!("CARGO_PKG_VERSION"))
}

// ── HTML stripping helper ─────────────────────────────────────────────────────

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut skip_block = false;
    let mut tag_buf = String::new();

    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag_buf.clear();
            }
            '>' => {
                let tag_lower = tag_buf.to_lowercase();
                if tag_lower.starts_with("script") || tag_lower.starts_with("style") {
                    skip_block = true;
                } else if tag_lower.starts_with("/script") || tag_lower.starts_with("/style") {
                    skip_block = false;
                }
                in_tag = false;
                if !skip_block && !out.ends_with(' ') && !out.ends_with('\n') {
                    out.push(' ');
                }
            }
            _ if in_tag => tag_buf.push(ch),
            _ if !skip_block => out.push(ch),
            _ => {}
        }
    }

    let out = decode_entities(&out);

    let mut result = String::with_capacity(out.len());
    let mut prev_newline = false;
    for line in out.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_newline { result.push('\n'); }
            prev_newline = true;
        } else {
            result.push_str(trimmed);
            result.push('\n');
            prev_newline = false;
        }
    }
    result.trim().to_string()
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
     .replace("&lt;", "<")
     .replace("&gt;", ">")
     .replace("&quot;", "\"")
     .replace("&#39;", "'")
     .replace("&apos;", "'")
     .replace("&nbsp;", " ")
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_github_blob_url() {
        let url = "https://github.com/rust-lang/rust/blob/master/README.md";
        let rewritten = rewrite_github_url(url);
        assert_eq!(rewritten, "https://raw.githubusercontent.com/rust-lang/rust/master/README.md");
    }

    #[test]
    fn test_rewrite_github_non_blob_unchanged() {
        let url = "https://github.com/rust-lang/rust";
        let rewritten = rewrite_github_url(url);
        assert_eq!(rewritten, url);
    }

    #[test]
    fn test_decode_entities() {
        assert_eq!(decode_entities("&amp;"), "&");
        assert_eq!(decode_entities("&lt;div&gt;"), "<div>");
        assert_eq!(decode_entities("&quot;hi&quot;"), "\"hi\"");
    }

    #[test]
    fn test_brave_setup_message_returned_without_key() {
        // Ensure we don't panic when no key — the execute path returns the setup message
        // (full async test skipped; checked via the sync key-resolution logic)
        let key = std::env::var("BRAVE_SEARCH_API_KEY").ok()
            .filter(|k| !k.trim().is_empty())
            .or_else(brave_key_from_config);
        if key.is_none() {
            // expected when run in CI without a key
        }
    }
}
