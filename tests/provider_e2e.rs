/// E2E tests for the /provider list and live Codex API connectivity.
use zap_coding_agent::session::commands::provider::provider_slugs;

#[test]
fn codex_in_provider_list() {
    let slugs = provider_slugs();
    assert!(
        slugs.contains(&"codex"),
        "codex missing from provider list — got: {:?}",
        slugs
    );
}

#[test]
fn codex_after_claude_code() {
    let slugs = provider_slugs();
    let claude_pos = slugs.iter().position(|&s| s == "claude_code").expect("claude_code missing");
    let codex_pos  = slugs.iter().position(|&s| s == "codex").expect("codex missing");
    assert!(
        codex_pos == claude_pos + 1,
        "codex should immediately follow claude_code in the list (claude_code={}, codex={})",
        claude_pos, codex_pos
    );
}

#[test]
fn all_expected_providers_present() {
    let slugs = provider_slugs();
    for expected in &["lm_studio", "ollama", "anthropic", "claude_code", "codex",
                      "openai", "gemini", "deepseek", "groq", "mistral",
                      "xai", "together", "perplexity", "cohere",
                      "openrouter", "kimi", "fireworks", "cerebras", "custom"] {
        assert!(slugs.contains(expected), "provider '{}' missing from list", expected);
    }
    assert_eq!(slugs.len(), 19, "expected 19 providers, got {}", slugs.len());
}

#[test]
fn check_codex_auth_absent_returns_none() {
    // When ~/.codex/auth.json doesn't exist, check_codex() must return None.
    // (Overriding CODEX_HOME to a temp dir guarantees no auth file exists.)
    let tmp = std::env::temp_dir().join("zap_codex_test_no_auth");
    let _ = std::fs::create_dir_all(&tmp);
    std::env::set_var("CODEX_HOME", &tmp);
    let result = zap_coding_agent::llm_client::auth::check_codex();
    std::env::remove_var("CODEX_HOME");
    assert!(result.is_none(), "check_codex() should return None when auth.json absent");
}

#[test]
fn check_codex_auth_present_returns_some() {
    let tmp = std::env::temp_dir().join("zap_codex_test_with_auth");
    let _ = std::fs::create_dir_all(&tmp);
    std::fs::write(
        tmp.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"tok","account_id":"acc"}}"#,
    ).unwrap();
    std::env::set_var("CODEX_HOME", &tmp);
    let result = zap_coding_agent::llm_client::auth::check_codex();
    std::env::remove_var("CODEX_HOME");
    let _ = std::fs::remove_dir_all(&tmp);
    assert_eq!(result, Some("ready".into()), "check_codex() should return Some when auth.json present");
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn codex_creds() -> (String, String) {
    let auth_path = dirs::home_dir()
        .expect("no home dir")
        .join(".codex/auth.json");
    assert!(auth_path.exists(), "~/.codex/auth.json not found — run `codex login` first");
    let raw = std::fs::read_to_string(&auth_path).expect("failed to read auth.json");
    let data: serde_json::Value = serde_json::from_str(&raw).expect("auth.json is not valid JSON");
    let token = data["tokens"]["access_token"].as_str()
        .expect("tokens.access_token missing").to_owned();
    let account = data["tokens"]["account_id"].as_str()
        .expect("tokens.account_id missing").to_owned();
    (token, account)
}

fn codex_post(token: &str, account: &str, model: &str) -> (u16, String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = reqwest::Client::new();
        let payload = serde_json::json!({
            "model":        model,
            "input":        [{"role": "user", "content": "Reply with the single word: pong"}],
            "instructions": "You are a test echo bot. Follow instructions exactly.",
            "store":        false,
            "stream":       true,
        });
        let resp = client
            .post("https://chatgpt.com/backend-api/codex/responses")
            .header("Authorization", format!("Bearer {token}"))
            .header("ChatGPT-Account-ID", account)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .expect("HTTP request failed");
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        (status, body)
    })
}

// ── live model tests ──────────────────────────────────────────────────────────

/// Live network test — hits the real Codex Responses API.
/// Skipped by default; run explicitly with: cargo test -- --ignored
#[test]
#[ignore = "live network call — requires ~/.codex/auth.json with a valid token"]
fn codex_live_api_o4_mini() {
    let (token, account) = codex_creds();
    let (status, body) = codex_post(&token, &account, "gpt-5.5");

    println!("Codex API status : {status}");
    println!("Codex API body   : {}", &body[..body.len().min(800)]);

    assert!((200..300).contains(&status), "Expected 2xx from Codex API, got {status}: {body}");
    assert!(!body.is_empty(), "Response body was empty");
    // SSE stream — must not start with an error JSON blob.
    assert!(!body.starts_with("{\"detail\""), "Codex API returned error: {body}");
}

#[test]
#[ignore = "live network call — documents that gpt-5.4 is NOT supported via ChatGPT account"]
fn codex_live_api_gpt54_unsupported() {
    let (token, account) = codex_creds();
    let (status, body) = codex_post(&token, &account, "gpt-5.4");

    println!("gpt-5.4 status : {status}");
    println!("gpt-5.4 body   : {body}");

    assert_eq!(status, 400, "Expected 400 for unsupported model gpt-5.4, got {status}");
    assert!(
        body.contains("not supported"),
        "Expected 'not supported' in error, got: {body}"
    );
}

#[test]
#[ignore = "live network call — documents that gpt-5.3-codex is NOT supported via ChatGPT account"]
fn codex_live_api_gpt53_unsupported() {
    let (token, account) = codex_creds();
    let (status, body) = codex_post(&token, &account, "gpt-5.3-codex");

    println!("gpt-5.3-codex status : {status}");
    println!("gpt-5.3-codex body   : {body}");

    assert_eq!(status, 400, "Expected 400 for unsupported model gpt-5.3-codex, got {status}");
    assert!(
        body.contains("not supported"),
        "Expected 'not supported' in error, got: {body}"
    );
}
