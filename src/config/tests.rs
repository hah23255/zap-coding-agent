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

// ── Config::save round-trip with models ───────────────────────────────────

#[test]
fn save_round_trips_models_and_extra_headers() {
    let mut config = Config { provider_slug: "gomodel".to_string(), ..Default::default() };

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
    });

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.toml");
    let mut f = std::fs::File::create(&path).unwrap();
    use std::io::Write;

    writeln!(f, "provider = \"gomodel\"").unwrap();
    writeln!(f, "permission_mode = \"ask\"").unwrap();
    writeln!(f).unwrap();

    let entry = config.all_providers.get("gomodel").unwrap();
    writeln!(f, "[providers.gomodel]").unwrap();
    writeln!(f, "kind     = {:?}", entry.kind.as_deref().unwrap()).unwrap();
    writeln!(f, "model    = {:?}", entry.model.as_deref().unwrap()).unwrap();
    writeln!(f, "api_key  = {:?}", entry.api_key.as_deref().unwrap()).unwrap();
    writeln!(f, "base_url = {:?}", entry.base_url.as_deref().unwrap()).unwrap();
    writeln!(f).unwrap();

    writeln!(f, "[providers.gomodel.extra_headers]").unwrap();
    writeln!(f, "X-Path = {:?}", "/zapagent").unwrap();
    writeln!(f).unwrap();

    let mut model_ids: Vec<&String> = entry.models.keys().collect();
    model_ids.sort();
    for mid in model_ids {
        let m = &entry.models[mid];
        writeln!(f, r#"[providers.gomodel.models."{mid}"]"#).unwrap();
        if let Some(ref n) = m.name { writeln!(f, "name = {:?}", n).unwrap(); }
        if m.reasoning { writeln!(f, "reasoning = true").unwrap(); }
        if let Some(c) = m.context { writeln!(f, "context = {c}").unwrap(); }
        if let Some(o) = m.output { writeln!(f, "output = {o}").unwrap(); }
        writeln!(f).unwrap();
    }

    let contents = std::fs::read_to_string(&path).unwrap();
    #[derive(serde::Deserialize)]
    struct Outer { providers: HashMap<String, ProviderEntry> }
    let parsed: Outer = toml::from_str(&contents).unwrap();
    let restored = parsed.providers.get("gomodel").unwrap();

    assert_eq!(restored.extra_headers.get("X-Path").map(|s| s.as_str()), Some("/zapagent"));
    assert_eq!(restored.models.len(), 2);
    let m1 = restored.models.get("xiaomi/mimo-v2.5").unwrap();
    assert!(m1.reasoning);
    assert_eq!(m1.context, Some(1_000_000));
    assert_eq!(m1.output, Some(128_000));
    let m2 = restored.models.get("xiaomi/mimo-v2.5-pro").unwrap();
    assert!(!m2.reasoning);
    assert_eq!(m2.context, Some(500_000));
    assert!(m2.output.is_none());
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
fn model_list_from_map_is_correct() {
    let mut models = HashMap::new();
    models.insert("alpha".to_string(), make_model_entry(Some(8_000), false));
    models.insert("beta".to_string(), make_model_entry(Some(32_000), true));
    let entry = ProviderEntry { models, ..Default::default() };
    let mut ids: Vec<&str> = entry.models.keys().map(|s| s.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["alpha", "beta"]);
    assert!(entry.models["beta"].reasoning);
    assert!(!entry.models["alpha"].reasoning);
}
