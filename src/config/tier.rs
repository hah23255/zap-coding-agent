use super::Config;

/// Returns true when the active provider should use SLM-optimised behaviour:
/// minimal system prompt, core-only tool set, Ollama num_ctx injection, and
/// message-alternation collapsing.
///
/// Priority: explicit `tier = "slm"` / `tier = "qwen3_8b"` in provider config > auto-detection.
/// Auto-detection: localhost URL + model name containing a small-size suffix (≤13B).
pub fn is_slm_tier(config: &Config) -> bool {
    if let Some(entry) = config.all_providers.get(&config.provider_slug) {
        if let Some(tier) = &entry.tier {
            return tier.eq_ignore_ascii_case("slm") || tier.eq_ignore_ascii_case("qwen3_8b");
        }
    }
    let url = config.base_url.as_deref().unwrap_or("");
    let is_local = url.contains("localhost") || url.contains("127.0.0.1")
        || url.contains("::1") || url.contains("0.0.0.0");
    if !is_local {
        return false;
    }
    let model = config.model.to_lowercase();
    // Match size suffixes ≤13B. Anchored to avoid "109b" matching as "9b".
    let small_sizes = ["0.5b", "1b", "1.5b", "2b", "3b", "3.8b", "4b", "7b", "8b", "9b", "11b", "12b", "13b"];
    small_sizes.iter().any(|sz| {
        if let Some(pos) = model.find(sz) {
            let before_ok = pos == 0 || !model[..pos].chars().last().is_some_and(|c| c.is_ascii_digit());
            let after = model[pos + sz.len()..].chars().next();
            before_ok && matches!(after, None | Some('-') | Some('_') | Some(':') | Some('.') | Some('q') | Some('i'))
        } else {
            false
        }
    })
}

pub fn is_qwen3_8b_tier(config: &Config) -> bool {
    if let Some(entry) = config.all_providers.get(&config.provider_slug) {
        if let Some(tier) = &entry.tier {
            return tier.eq_ignore_ascii_case("qwen3_8b");
        }
    }
    if !is_slm_tier(config) {
        return false;
    }
    let model = config.model.to_lowercase();
    model.contains("qwen3") && model.contains("8b")
}
