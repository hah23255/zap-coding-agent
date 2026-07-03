use super::*;
use std::collections::HashMap;

fn make_model_entry(context: Option<usize>, reasoning: bool) -> ModelEntry {
    ModelEntry { name: None, reasoning, context, output: None }
}

// ── ModelEntry deserialization ────────────────────────────────────────────

#[test]
fn model_entry_full_toml() {
    let toml = r#"
name      = "mimo-v2.5"
reasoning = true
context   = 1000000
output    = 128000
"#;
    let m: ModelEntry = toml::from_str(toml).unwrap();
    assert_eq!(m.name.as_deref(), Some("mimo-v2.5"));
    assert!(m.reasoning);
    assert_eq!(m.context, Some(1_000_000));
    assert_eq!(m.output, Some(128_000));
}

#[test]
fn model_entry_minimal_toml() {
    let toml = r#"context = 32768"#;
    let m: ModelEntry = toml::from_str(toml).unwrap();
    assert_eq!(m.context, Some(32_768));
    assert!(!m.reasoning);
    assert!(m.name.is_none());
    assert!(m.output.is_none());
}

#[test]
fn model_entry_default_is_empty() {
    let m = ModelEntry::default();
    assert!(!m.reasoning);
    assert!(m.context.is_none());
    assert!(m.output.is_none());
}

// ── resolve_provider_kind ──────────────────────────────────────────────────

#[test]
fn claude_code_slug_resolves_to_anthropic_even_without_toml_kind() {
    // Regression: previously fell through to the generic kind/slug match,
    // where "claude_code" doesn't match "anthropic" case-insensitively and
    // silently resolved to Provider::OpenAi.
    assert!(matches!(resolve_provider_kind("claude_code", None), Provider::Anthropic));
}

#[test]
fn codex_slug_resolves_to_openai_even_without_toml_kind() {
    assert!(matches!(resolve_provider_kind("codex", None), Provider::OpenAi));
}

#[test]
fn claude_code_slug_ignores_a_conflicting_toml_kind() {
    // Built-in slugs are hardcoded regardless of what a stale/wrong TOML entry says.
    assert!(matches!(resolve_provider_kind("claude_code", Some("openai")), Provider::Anthropic));
}

#[test]
fn unknown_slug_falls_back_to_kind_field() {
    assert!(matches!(resolve_provider_kind("my-custom-provider", Some("anthropic")), Provider::Anthropic));
    assert!(matches!(resolve_provider_kind("my-custom-provider", Some("openai")), Provider::OpenAi));
}

#[test]
fn unknown_slug_with_no_kind_falls_back_to_slug_name() {
    assert!(matches!(resolve_provider_kind("anthropic", None), Provider::Anthropic));
    assert!(matches!(resolve_provider_kind("some-openai-compatible-server", None), Provider::OpenAi));
}

// ── ProviderEntry with models deserialization ─────────────────────────────

#[test]
fn provider_entry_with_models_toml() {
    let toml = r#"
kind    = "openai"
api_key = "sk-test"
model   = "xiaomi/mimo-v2.5"
base_url = "https://gateway.example.com/v1/chat/completions"

[extra_headers]
X-Path = "/zapagent"

[models."xiaomi/mimo-v2.5"]
name      = "mimo-v2.5"
reasoning = true
context   = 1000000
output    = 128000

[models."xiaomi/mimo-v2.5-pro"]
name    = "mimo-v2.5-pro"
context = 500000
"#;
    let e: ProviderEntry = toml::from_str(toml).unwrap();
    assert_eq!(e.kind.as_deref(), Some("openai"));
    assert_eq!(e.model.as_deref(), Some("xiaomi/mimo-v2.5"));
    assert_eq!(e.extra_headers.get("X-Path").map(|s| s.as_str()), Some("/zapagent"));
    assert_eq!(e.models.len(), 2);

    let m1 = e.models.get("xiaomi/mimo-v2.5").unwrap();
    assert!(m1.reasoning);
    assert_eq!(m1.context, Some(1_000_000));
    assert_eq!(m1.output, Some(128_000));

    let m2 = e.models.get("xiaomi/mimo-v2.5-pro").unwrap();
    assert!(!m2.reasoning);
    assert_eq!(m2.context, Some(500_000));
    assert!(m2.output.is_none());
}

#[test]
fn provider_entry_empty_models_is_fine() {
    let toml = r#"kind = "openai""#;
    let e: ProviderEntry = toml::from_str(toml).unwrap();
    assert!(e.models.is_empty());
}

// ── Config::save_to round-trip with models and extra_headers ─────────────
// Calls the real Config::save_to() so that any change to the serialization
// logic is caught here rather than drifting silently.

#[test]
fn save_round_trips_models_and_extra_headers() {
    let mut models = HashMap::new();
    models.insert("xiaomi/mimo-v2.5".to_string(), ModelEntry {
        name: Some("mimo-v2.5".to_string()),
        reasoning: true,
        context: Some(1_000_000),
        output: Some(128_000),
    });
    models.insert("xiaomi/mimo-v2.5-pro".to_string(), ModelEntry {
        name: Some("mimo-v2.5-pro".to_string()),
        reasoning: false,
        context: Some(500_000),
        output: None,
    });

    let mut extra_headers = HashMap::new();
    extra_headers.insert("X-Path".to_string(), "/zapagent".to_string());

    let mut config = Config { provider_slug: "gomodel".to_string(), ..Default::default() };
    config.all_providers.insert("gomodel".to_string(), ProviderEntry {
        kind: Some("openai".to_string()),
        api_key: Some("sk-test".to_string()),
        model: Some("xiaomi/mimo-v2.5".to_string()),
        context_window: None,
        base_url: Some("https://gw.example.com/v1/chat/completions".to_string()),
        credential_method: None,
        auth_header: None,
        extra_headers,
        models,
        tier: None,
    });

    // Write via the real Config::save_to().
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.toml");
    config.save_to(&path).expect("save_to failed");

    // Parse back and assert round-trip fidelity.
    let contents = std::fs::read_to_string(&path).unwrap();
    #[derive(serde::Deserialize)]
    struct Outer { providers: HashMap<String, ProviderEntry> }
    let parsed: Outer = toml::from_str(&contents)
        .unwrap_or_else(|e| panic!("TOML parse failed:\n{contents}\nError: {e}"));
    let restored = parsed.providers.get("gomodel").unwrap();

    assert_eq!(restored.api_key.as_deref(), Some("sk-test"));
    assert_eq!(restored.model.as_deref(), Some("xiaomi/mimo-v2.5"));
    assert_eq!(restored.base_url.as_deref(), Some("https://gw.example.com/v1/chat/completions"));
    assert_eq!(restored.extra_headers.get("X-Path").map(|s| s.as_str()), Some("/zapagent"));
    assert_eq!(restored.models.len(), 2);

    let m1 = restored.models.get("xiaomi/mimo-v2.5").unwrap();
    assert!(m1.reasoning);
    assert_eq!(m1.context, Some(1_000_000));
    assert_eq!(m1.output, Some(128_000));
    assert_eq!(m1.name.as_deref(), Some("mimo-v2.5"));

    let m2 = restored.models.get("xiaomi/mimo-v2.5-pro").unwrap();
    assert!(!m2.reasoning);
    assert_eq!(m2.context, Some(500_000));
    assert!(m2.output.is_none());
    assert_eq!(m2.name.as_deref(), Some("mimo-v2.5-pro"));
}

#[test]
fn save_round_trips_provider_slug_and_permission_mode() {
    // Verifies the top-level fields are written correctly by save_to().
    let config = Config {
        provider_slug: "my-gw".to_string(),
        permission_mode: PermissionMode::Auto,
        ..Default::default()
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.toml");
    config.save_to(&path).expect("save_to failed");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("provider        = \"my-gw\""), "slug missing:\n{contents}");
    assert!(contents.contains("permission_mode = \"auto\""), "permission_mode missing:\n{contents}");
}

// ── configured_context_limit priority ────────────────────────────────────

fn ctx_limit(config: &Config) -> usize {
    let entry = config.all_providers.get(&config.provider_slug);
    let model_ctx = entry
        .and_then(|e| e.models.get(&config.model))
        .and_then(|m| m.context);
    model_ctx
        .or_else(|| entry.and_then(|e| e.context_window))
        .or_else(|| default_context_window_for_provider(
            &config.provider_slug,
            entry.and_then(|e| e.kind.as_deref()),
        ))
        .unwrap_or(32_768)
}

#[test]
fn model_level_context_beats_provider_level() {
    let mut config = Config { provider_slug: "gw".to_string(), model: "big-model".to_string(), ..Default::default() };
    let mut models = HashMap::new();
    models.insert("big-model".to_string(), make_model_entry(Some(1_000_000), false));
    config.all_providers.insert("gw".to_string(), ProviderEntry {
        context_window: Some(32_768),
        models,
        ..Default::default()
    });
    assert_eq!(ctx_limit(&config), 1_000_000);
}

#[test]
fn falls_back_to_provider_context_when_model_not_in_map() {
    let mut config = Config { provider_slug: "gw".to_string(), model: "other-model".to_string(), ..Default::default() };
    let mut models = HashMap::new();
    models.insert("big-model".to_string(), make_model_entry(Some(1_000_000), false));
    config.all_providers.insert("gw".to_string(), ProviderEntry {
        context_window: Some(64_000),
        models,
        ..Default::default()
    });
    assert_eq!(ctx_limit(&config), 64_000);
}

#[test]
fn falls_back_to_provider_context_when_model_has_no_context_field() {
    let mut config = Config { provider_slug: "gw".to_string(), model: "reasoning-model".to_string(), ..Default::default() };
    let mut models = HashMap::new();
    models.insert("reasoning-model".to_string(), make_model_entry(None, true));
    config.all_providers.insert("gw".to_string(), ProviderEntry {
        context_window: Some(128_000),
        models,
        ..Default::default()
    });
    assert_eq!(ctx_limit(&config), 128_000);
}

#[test]
fn explicit_qwen3_8b_tier_counts_as_slm() {
    let mut config = Config {
        provider_slug: "local".to_string(),
        model: "anything".to_string(),
        ..Default::default()
    };
    config.all_providers.insert("local".to_string(), ProviderEntry {
        tier: Some("qwen3_8b".to_string()),
        ..Default::default()
    });

    assert!(is_slm_tier(&config));
    assert!(is_qwen3_8b_tier(&config));
}

#[test]
fn auto_detects_local_qwen3_8b_tier() {
    let config = Config {
        model: "qwen3:8b".to_string(),
        base_url: Some("http://localhost:11434/v1/chat/completions".to_string()),
        ..Default::default()
    };

    assert!(is_slm_tier(&config));
    assert!(is_qwen3_8b_tier(&config));
}

#[test]
fn frontier_qwen3_8b_is_not_treated_as_local_slm() {
    let config = Config {
        model: "qwen3:8b".to_string(),
        base_url: Some("https://api.openai.com/v1/chat/completions".to_string()),
        ..Default::default()
    };

    assert!(!is_slm_tier(&config));
    assert!(!is_qwen3_8b_tier(&config));
}

#[test]
fn local_non_qwen_slm_does_not_trigger_qwen3_8b_mode() {
    let config = Config {
        model: "gemma3:9b".to_string(),
        base_url: Some("http://127.0.0.1:11434/v1/chat/completions".to_string()),
        ..Default::default()
    };

    assert!(is_slm_tier(&config));
    assert!(!is_qwen3_8b_tier(&config));
}

// ── disabled_tools / disabled_skills ─────────────────────────────────────

#[test]
fn disabled_tools_and_skills_parsed_from_toml() {
    let toml_str = r#"
        api_key = "test"
        model = "claude-sonnet-4-6"
        disabled_tools = ["shell", "web_fetch"]
        disabled_skills = ["deploy"]
    "#;
    let file: FileConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(file.disabled_tools, vec!["shell", "web_fetch"]);
    assert_eq!(file.disabled_skills, vec!["deploy"]);
}

#[test]
fn disabled_tools_and_skills_default_to_empty() {
    let toml_str = r#"api_key = "test""#;
    let file: FileConfig = toml::from_str(toml_str).unwrap();
    assert!(file.disabled_tools.is_empty());
    assert!(file.disabled_skills.is_empty());
}

// ── model_routes ──────────────────────────────────────────────────────────

#[test]
fn model_routes_parsed_from_toml() {
    let toml_str = r#"
        api_key = "test"
        model = "claude-sonnet-4-6"
        [model_routes]
        coding = "codex/gpt-5.5"
        review = "claude-opus-4-7"
    "#;
    let file: FileConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(file.model_routes.get("coding").map(|s| s.as_str()), Some("codex/gpt-5.5"));
    assert_eq!(file.model_routes.get("review").map(|s| s.as_str()), Some("claude-opus-4-7"));
}

#[test]
fn model_routes_default_to_empty() {
    let toml_str = r#"api_key = "test""#;
    let file: FileConfig = toml::from_str(toml_str).unwrap();
    assert!(file.model_routes.is_empty());
}
