const ANTHROPIC_DEFAULT_URL: &str = "https://api.anthropic.com/v1/messages";
const OPENAI_DEFAULT_BASE: &str = "https://api.openai.com";

pub fn normalize_anthropic_url(base_url: Option<&str>) -> String {
    match base_url {
        Some(u) => {
            let u = u.trim_end_matches('/');
            if u.ends_with("/messages") { u.to_string() }
            else { format!("{}/v1/messages", u) }
        }
        None => ANTHROPIC_DEFAULT_URL.to_string(),
    }
}

pub fn normalize_openai_url(base_url: Option<&str>) -> String {
    match base_url {
        Some(u) => {
            let u = u.trim_end_matches('/');
            if u.ends_with("/chat/completions") { u.to_string() }
            else if u.ends_with("/v1") { format!("{}/chat/completions", u) }
            else { format!("{}/v1/chat/completions", u) }
        }
        None => format!("{}/v1/chat/completions", OPENAI_DEFAULT_BASE),
    }
}

/// Fetch available models from an OpenAI-compatible `/models` endpoint.
///
/// `base_url` is a chat completions endpoint (e.g. `http://localhost:1234/v1/chat/completions`).
/// Returns model IDs on success, or an empty vec if the request fails.
pub fn fetch_openai_compatible_models(base_url: &str) -> Vec<String> {
    fetch_openai_compatible_models_with_auth(base_url, None, &Default::default())
}

/// Same as `fetch_openai_compatible_models` but sends an Authorization Bearer
/// header and any `extra_headers` — needed for gated /v1/models endpoints.
pub fn fetch_openai_compatible_models_with_auth(
    base_url: &str,
    api_key: Option<&str>,
    extra_headers: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let base = base_url
        .trim_end_matches('/')
        .strip_suffix("/chat/completions")
        .unwrap_or(base_url);
    let url = format!("{}/models", base.trim_end_matches('/'));

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let mut req = client.get(&url);
    if let Some(key) = api_key {
        if !key.is_empty() && !extra_headers.contains_key("Authorization") {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }
    for (k, v) in extra_headers {
        req = req.header(k, v);
    }
    match req.send() {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>() {
                Ok(json) => {
                    if let Some(arr) = json["data"].as_array() {
                        arr.iter()
                            .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                            .collect()
                    } else {
                        Vec::new()
                    }
                }
                Err(_) => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ANTHROPIC_DEFAULT_URL, OPENAI_DEFAULT_BASE, normalize_anthropic_url, normalize_openai_url};

    #[test]
    fn openai_full_endpoint_used_as_is() {
        assert_eq!(
            normalize_openai_url(Some("https://api.deepseek.com/v1/chat/completions")),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    #[test]
    fn openai_base_url_gets_path_appended() {
        assert_eq!(
            normalize_openai_url(Some("https://api.deepseek.com")),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    #[test]
    fn openai_trailing_slash_trimmed() {
        assert_eq!(
            normalize_openai_url(Some("https://api.groq.com/openai/v1/chat/completions/")),
            "https://api.groq.com/openai/v1/chat/completions"
        );
    }

    #[test]
    fn openai_v1_base_gets_path_appended() {
        assert_eq!(
            normalize_openai_url(Some("https://api.mistral.ai/v1")),
            "https://api.mistral.ai/v1/chat/completions"
        );
    }

    #[test]
    fn openai_lm_studio_full_url() {
        assert_eq!(
            normalize_openai_url(Some("http://localhost:1234/v1/chat/completions")),
            "http://localhost:1234/v1/chat/completions"
        );
    }

    #[test]
    fn openai_none_uses_default() {
        assert_eq!(
            normalize_openai_url(None),
            format!("{}/v1/chat/completions", OPENAI_DEFAULT_BASE)
        );
    }

    #[test]
    fn anthropic_full_endpoint_used_as_is() {
        assert_eq!(
            normalize_anthropic_url(Some("https://my-gateway.corp/v1/messages")),
            "https://my-gateway.corp/v1/messages"
        );
    }

    #[test]
    fn anthropic_base_url_gets_path_appended() {
        assert_eq!(
            normalize_anthropic_url(Some("https://my-gateway.corp")),
            "https://my-gateway.corp/v1/messages"
        );
    }

    #[test]
    fn anthropic_trailing_slash_trimmed() {
        assert_eq!(
            normalize_anthropic_url(Some("https://my-gateway.corp/v1/messages/")),
            "https://my-gateway.corp/v1/messages"
        );
    }

    #[test]
    fn anthropic_none_uses_default() {
        assert_eq!(normalize_anthropic_url(None), ANTHROPIC_DEFAULT_URL);
    }
}
