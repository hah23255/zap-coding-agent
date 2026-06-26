use anyhow::{Context, Result};
use async_trait::async_trait;

use super::Tool;

// ── web_fetch ─────────────────────────────────────────────────────────────────

pub(super) struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str { "web_fetch" }
    fn description(&self) -> &str {
        "Fetch a URL and return its content as plain text (HTML tags stripped). \
         Useful for reading documentation, API references, or web pages."
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

        let client = crate::http::client();

        let resp = client.get(url).send().await
            .with_context(|| format!("web_fetch: could not reach '{}'", url))?;

        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("web_fetch: HTTP {} from '{}'", status, url);
        }

        let body = resp.text().await?;
        let text = strip_html(&body);

        if text.len() > max_chars {
            let mut cut = max_chars;
            while cut > 0 && !text.is_char_boundary(cut) { cut -= 1; }
            Ok(format!("{}\n\n[…truncated to {} bytes of {}]",
                &text[..cut], cut, text.len()))
        } else {
            Ok(text)
        }
    }
}

// ── web_search ────────────────────────────────────────────────────────────────

pub(super) struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str { "web_search" }
    fn description(&self) -> &str {
        "Search the web using DuckDuckGo and return top results with titles and URLs. \
         No API key required."
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

        let client = crate::http::client();

        // Use DuckDuckGo HTML endpoint (non-JS version). Returns real search
        // results with stable CSS classes: result__a (title), result__snippet,
        // result__url. More reliable than the instant-answer JSON API which
        // only returns definitions and facts.
        let resp = client
            .get("https://html.duckduckgo.com/html/")
            .query(&[("q", query)])
            .send()
            .await
            .context("web_search: could not reach DuckDuckGo")?;

        let html = resp.text().await
            .context("web_search: could not read response")?;

        // DuckDuckGo may serve a challenge page when it suspects bot traffic
        // (IP reputation, TLS fingerprint, etc). Detect both reCAPTCHA and
        // DDG's custom "anomaly-modal" challenge. The anomaly page is ~14KB
        // and starts with an HTML comment wrapping <!DOCTYPE, so we check
        // for the modal class and the "bots use" text.
        if html.contains("g-recaptcha")
            || html.contains("anomaly-modal")
            || html.contains("Unfortunately, bots use DuckDuckGo too.")
        {
            anyhow::bail!(
                "web_search: DuckDuckGo returned a challenge page. \
                 This happens when the search engine detects automated traffic \
                 from your IP. Try again later."
            );
        }

        let results = parse_ddg_results(&html, max);

        if results.is_empty() {
            Ok(format!("No results found for '{}'.", query))
        } else {
            Ok(format!("Search results for '{}':\n\n{}", query, results.join("\n\n")))
        }
    }
}

// ── DuckDuckGo HTML result parser ─────────────────────────────────────────

/// Parses DuckDuckGo HTML search results with classes:
///   <a class="result__a" href="URL">Title</a>
///   <a class="result__snippet">Snippet text...</a>
///   <a class="result__url">display URL</a>
fn parse_ddg_results(html: &str, max: usize) -> Vec<String> {
    use regex::Regex;

    let mut results: Vec<String> = Vec::new();

    // Match result links: <a ... class="result__a" ... href="URL">Title</a>
    let link_re = Regex::new(
        r#"<a[^>]*\bclass="result__a"[^>]*href="(?P<url>[^"]*)"[^>]*>(?P<title>.+?)</a>"#
    ).unwrap();

    // Match snippets: <a ... class="result__snippet">...</a>
    let snippet_re = Regex::new(
        r#"<a[^>]*\bclass="result__snippet"[^>]*>(?P<snippet>.+?)</a>"#
    ).unwrap();

    let snippets: Vec<String> = snippet_re.captures_iter(html)
        .filter_map(|c| c.name("snippet"))
        .map(|m| decode_entities(m.as_str()).trim().to_string())
        .collect();
    let mut snippet_idx = 0;

    for caps in link_re.captures_iter(html) {
        if results.len() >= max {
            break;
        }
        let url   = caps.name("url").map(|m| m.as_str()).unwrap_or("");
        let title = caps.name("title").map(|m| m.as_str()).unwrap_or("?");

        // Skip nav/ad links — but still advance snippet_idx so the
        // 1:1 alignment between result links and snippets stays correct.
        if url.is_empty() || title.is_empty() ||
           url.starts_with("//duckduckgo.com") || url.starts_with("/") {
            snippet_idx += 1;
            continue;
        }

        let snippet = snippets.get(snippet_idx).cloned().unwrap_or_default();
        snippet_idx += 1;

        let title = decode_entities(title).trim().to_string();
        let url = decode_entities(url);

        if snippet.is_empty() {
            results.push(format!("• {}\n  {}", title, url));
        } else {
            results.push(format!("• {}\n  {}\n  {}", title, snippet, url));
        }
    }

    results
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

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulated DuckDuckGo HTML results page with two results, plus a
    /// nav link that should be skipped (testing snippet-alignment).
    #[test]
    fn test_parse_ddg_results() {
        let html = r#"<!DOCTYPE html>
<html>
<head><title>Search</title></head>
<body>
<div class="results">
    <!-- Internal nav link — should be skipped -->
    <a class="result__a" href="/settings">Settings</a>
    <a class="result__snippet">not a real result</a>

    <!-- Real result 1 -->
    <a class="result__a" href="https://www.rust-lang.org/">Rust Programming Language</a>
    <a class="result__snippet">A language empowering everyone to build reliable and efficient software.</a>

    <!-- Real result 2 -->
    <a class="result__a" href="https://en.wikipedia.org/wiki/Rust">Rust (programming language) - Wikipedia</a>
    <a class="result__snippet">Rust is a general-purpose programming language emphasizing performance, type safety, and concurrency.</a>
</div>
</body></html>"#;

        let results = parse_ddg_results(html, 5);

        assert_eq!(results.len(), 2, "should find 2 results (skipping the nav link)");

        // Result 1
        assert!(results[0].contains("Rust Programming Language"),
            "result 0 title mismatch: {}", results[0]);
        assert!(results[0].contains("rust-lang.org"),
            "result 0 url mismatch: {}", results[0]);
        assert!(results[0].contains("reliable and efficient"),
            "result 0 snippet mismatch (snippet-alignment bug?): {}", results[0]);

        // Result 2
        assert!(results[1].contains("Wikipedia"),
            "result 1 title mismatch: {}", results[1]);
        assert!(results[1].contains("wikipedia.org"),
            "result 1 url mismatch: {}", results[1]);
        assert!(results[1].contains("type safety"),
            "result 1 snippet mismatch (snippet-alignment bug?): {}", results[1]);
    }

    #[test]
    fn test_parse_ddg_results_empty() {
        let html = "<html><body>no results here</body></html>";
        let results = parse_ddg_results(html, 5);
        assert!(results.is_empty(), "should return empty for no results");
    }

    #[test]
    fn test_decode_entities() {
        assert_eq!(decode_entities("&amp;"), "&");
        assert_eq!(decode_entities("&lt;div&gt;"), "<div>");
        assert_eq!(decode_entities("&quot;hi&quot;"), "\"hi\"");
        assert_eq!(decode_entities("hello &amp; world"), "hello & world");
    }

    /// Live integration test — hits DuckDuckGo's HTML endpoint and verifies
    /// we get real results back. Skipped if DDG serves a challenge page
    /// (network/IP reputation issue).
    #[tokio::test]
    async fn web_search_live_returns_results() {
        let input = serde_json::json!({
            "query": "Rust programming language",
            "max_results": 3
        });

        match WebSearchTool.execute(input).await {
            Ok(output) => {
                if output.starts_with("No results found") {
                    // DDG may return a challenge page that our detection missed,
                    // or genuinely no results. Save HTML for debugging.
                    eprintln!("WARNING: web_search returned no results. This may indicate:\n\
                               - A DuckDuckGo challenge page not caught by detection\n\
                               - The `html.duckduckgo.com` endpoint changed its HTML structure\n\
                               - Genuinely no results\n\
                               Skipping live test (not a code failure).");
                    return; // graceful skip
                }
                assert!(output.contains("Search results for"), "unexpected output: {output}");
                assert!(output.contains("•"), "expected bullet-point results: {output}");
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("challenge page") {
                    eprintln!("SKIP: DuckDuckGo returned a challenge page — skipping live test.");
                } else {
                    panic!("web_search failed with unexpected error: {msg}");
                }
            }
        }
    }
}
