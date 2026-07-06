use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures::StreamExt;

use super::{ApiResponse, BeforeOutput, ContentBlock, Message, Usage, redact_token, send_with_retry, LlmProvider};

const CODEX_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const CODEX_REFRESH_URL:   &str = "https://auth.openai.com/oauth/token";
const CODEX_CLIENT_ID:     &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REFRESH_SKEW_SECS:   i64  = 30;

pub struct CodexClient {
    http:            reqwest::Client,
    model:           String,
    suppress_stream: bool,
}

impl CodexClient {
    pub fn new(model: String, suppress_stream: bool) -> Self {
        Self { http: crate::http::client().clone(), model, suppress_stream }
    }
}

// ── Credential loading ────────────────────────────────────────────────────────

/// Returns (access_token, account_id) from CODEX_ACCESS_TOKEN env or ~/.codex/auth.json.
/// Refreshes the token if it is expired or near expiry, writing back to auth.json.
async fn load_credentials(http: &reqwest::Client) -> Result<(String, Option<String>)> {
    // Fast path: explicit env var (CI / manual override)
    if let Ok(tok) = std::env::var("CODEX_ACCESS_TOKEN") {
        if !tok.is_empty() {
            return Ok((tok, None));
        }
    }

    let auth_path = {
        let home = std::env::var("CODEX_HOME").ok().unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".codex").to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        std::path::PathBuf::from(home).join("auth.json")
    };

    let raw = std::fs::read_to_string(&auth_path).with_context(|| {
        format!(
            "Codex auth file not found at {}. Run `codex login` first.",
            auth_path.display()
        )
    })?;
    let mut data: serde_json::Value =
        serde_json::from_str(&raw).context("failed to parse ~/.codex/auth.json")?;

    if data["auth_mode"].as_str() != Some("chatgpt") {
        anyhow::bail!(
            "~/.codex/auth.json has auth_mode {:?} — expected \"chatgpt\". Run `codex login`.",
            data["auth_mode"]
        );
    }

    let access_token = data["tokens"]["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No access_token in ~/.codex/auth.json. Run `codex login`."))?
        .to_string();
    let account_id = data["tokens"]["account_id"].as_str().map(|s| s.to_string());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Token still valid with skew margin — use it directly
    if jwt_exp(&access_token).map(|e| now < e - REFRESH_SKEW_SECS).unwrap_or(false) {
        return Ok((access_token, account_id));
    }

    // Token expired — refresh
    let refresh_token = data["tokens"]["refresh_token"]
        .as_str()
        .ok_or_else(|| {
            anyhow::anyhow!("No refresh_token in ~/.codex/auth.json. Run `codex login`.")
        })?
        .to_string();

    let resp = http
        .post(CODEX_REFRESH_URL)
        .json(&serde_json::json!({
            "client_id":     CODEX_CLIENT_ID,
            "grant_type":    "refresh_token",
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .context("Codex token refresh request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Codex token refresh failed (HTTP {}): {}. Run `codex login`.",
            status, body
        );
    }

    let new_tok: serde_json::Value =
        resp.json().await.context("failed to parse Codex token refresh response")?;
    let new_access = new_tok["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Token refresh response missing access_token."))?
        .to_string();

    // Write updated tokens back atomically
    if let Some(obj) = data["tokens"].as_object_mut() {
        obj.insert("access_token".into(), serde_json::json!(new_access));
        if let Some(v) = new_tok["id_token"].as_str() {
            obj.insert("id_token".into(), serde_json::json!(v));
        }
        if let Some(v) = new_tok["refresh_token"].as_str() {
            obj.insert("refresh_token".into(), serde_json::json!(v));
        }
    }
    let tmp = auth_path.with_extension("json.tmp");
    if let Ok(serialised) = serde_json::to_string_pretty(&data) {
        let _ = std::fs::write(&tmp, serialised).and_then(|_| std::fs::rename(&tmp, &auth_path));
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&auth_path, std::fs::Permissions::from_mode(0o600));
        }
    }

    Ok((new_access, account_id))
}

fn jwt_exp(token: &str) -> Option<i64> {
    let b64 = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(b64).ok()?;
    let payload: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    payload["exp"].as_i64()
}

// ── Request encoding ──────────────────────────────────────────────────────────

/// Encode zap's internal message list into the Responses API flat `input` array.
fn encode_input(messages: &[Message]) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "user" => {
                let texts: Vec<&str> = msg.content.iter().filter_map(|b| {
                    if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None }
                }).collect();
                let joined = texts.join("\n");
                let images: Vec<(&str, &str)> = msg.content.iter().filter_map(|b| {
                    if let ContentBlock::Image { media_type, data } = b {
                        Some((media_type.as_str(), data.as_str()))
                    } else {
                        None
                    }
                }).collect();

                if images.is_empty() {
                    if !joined.is_empty() {
                        out.push(serde_json::json!({ "role": "user", "content": joined }));
                    }
                } else if !joined.is_empty() || !images.is_empty() {
                    // Responses API multipart content: input_text + input_image parts,
                    // images as data URIs (same convention as the OpenAI direct client).
                    let mut parts: Vec<serde_json::Value> = Vec::new();
                    if !joined.is_empty() {
                        parts.push(serde_json::json!({ "type": "input_text", "text": joined }));
                    }
                    for (media_type, data) in &images {
                        parts.push(serde_json::json!({
                            "type": "input_image",
                            "image_url": format!("data:{};base64,{}", media_type, data),
                        }));
                    }
                    out.push(serde_json::json!({ "role": "user", "content": parts }));
                }
                for b in &msg.content {
                    if let ContentBlock::ToolResult { tool_use_id, content } = b {
                        out.push(serde_json::json!({
                            "type":    "function_call_output",
                            "call_id": tool_use_id,
                            "output":  content,
                        }));
                    }
                }
            }
            "assistant" => {
                // Text comes before tool calls — matches Responses API ordering
                if let Some(t) = msg.content.iter().find_map(|b| {
                    if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None }
                }) {
                    if !t.is_empty() {
                        out.push(serde_json::json!({ "role": "assistant", "content": t }));
                    }
                }
                for b in &msg.content {
                    if let ContentBlock::ToolUse { id, name, input } = b {
                        out.push(serde_json::json!({
                            "type":      "function_call",
                            "call_id":   id,
                            "name":      name,
                            "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()),
                        }));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn encode_tools(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    tools.iter().map(|t| serde_json::json!({
        "type":        "function",
        "name":        t["name"],
        "description": t["description"],
        "parameters":  t["input_schema"],
        "strict":      false,
    })).collect()
}

// ── LlmProvider impl ──────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for CodexClient {
    async fn send(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[serde_json::Value],
        before_output: Option<BeforeOutput>,
        _thinking_budget: u32,
    ) -> Result<ApiResponse> {
        let mut before_output = before_output;
        let mut highlighter = crate::stream_highlighter::StreamHighlighter::new();
        highlighter.suppress_print = crate::tui::channel::is_tui_mode();

        let (access_token, account_id) = load_credentials(&self.http).await?;

        let input   = encode_input(messages);
        let oai_tools = encode_tools(tools);

        let mut body = serde_json::json!({
            "model":        self.model,
            "input":        input,
            "instructions": system,
            "store":        false,
            "stream":       true,
        });
        if !oai_tools.is_empty() {
            body["tools"] = serde_json::json!(oai_tools);
        }

        // ── logging ───────────────────────────────────────────────────────────
        {
            let auth_val = format!("Bearer {}", access_token);
            let mut log_body = body.clone();
            if let Some(n) = log_body["tools"].as_array().map(|t| t.len()) {
                if n > 0 { log_body["tools"] = serde_json::json!(format!("<{n} tools — omitted>")); }
            }
            if let Ok(pretty) = serde_json::to_string_pretty(&log_body) {
                let curl = super::build_curl_block(
                    "codex", CODEX_RESPONSES_URL,
                    "Authorization", &auth_val, &body,
                );
                crate::log::write_llm(
                    "REQUEST [codex]",
                    &format!("POST {}\nAuthorization: {}\n\n{}{}", CODEX_RESPONSES_URL,
                        redact_token(&auth_val), pretty, curl),
                );
            }
        }

        let body_bytes = serde_json::to_vec(&body).context("failed to serialise Codex request")?;
        let account_id_clone = account_id.clone();

        let resp = send_with_retry(&self.http, |http| {
            let mut req = http
                .post(CODEX_RESPONSES_URL)
                .header("content-type", "application/json")
                .header("Authorization", format!("Bearer {}", access_token))
                .timeout(std::time::Duration::from_secs(3600))
                .body(body_bytes.clone());
            if let Some(ref id) = account_id_clone {
                req = req.header("ChatGPT-Account-ID", id);
            }
            req
        })
        .await
        .context("failed to reach Codex API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            crate::log::write_llm("ERROR [codex]", &format!("HTTP {} — {}", status, text));
            anyhow::bail!("Codex API returned {} (url: {}): {}", status, CODEX_RESPONSES_URL, text);
        }

        // 5-hour/weekly usage-window headers — warn before the window runs out
        // rather than after Codex starts rejecting requests.
        crate::quota_watch::check_codex_usage(resp.headers());

        // ── SSE stream parsing ────────────────────────────────────────────────
        let mut stream = resp.bytes_stream();
        let mut buf        = String::new();
        let mut text_acc   = String::new();
        let mut tool_calls: Vec<ContentBlock> = Vec::new();
        let mut usage_acc  = Usage::default();

        'outer: loop {
            let chunk = match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                stream.next(),
            ).await {
                Ok(Some(c)) => c,
                Ok(None)    => break 'outer,
                Err(_)      => continue,
            };
            let bytes: bytes::Bytes = chunk.context("Codex SSE stream error")?;
            buf.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim_end_matches('\r').to_string();
                buf = buf[pos + 1..].to_string();

                let Some(data) = line.strip_prefix("data: ") else { continue };
                if data == "[DONE]" { break 'outer; }

                let Ok(ev) = serde_json::from_str::<serde_json::Value>(data) else { continue };

                match ev["type"].as_str().unwrap_or("") {
                    "response.output_text.delta" => {
                        if let Some(delta) = ev["delta"].as_str() {
                            if !delta.is_empty() {
                                crate::remote_channel::send_chunk(delta);
                                if !self.suppress_stream {
                                    if let Some(cb) = before_output.take() { cb(); }
                                    highlighter.push(delta);
                                    let _ = std::io::Write::flush(&mut std::io::stdout());
                                }
                                text_acc.push_str(delta);
                            }
                        }
                    }

                    "response.output_item.done" => {
                        let item = &ev["item"];
                        if item["type"].as_str() == Some("function_call") {
                            let id   = item["call_id"].as_str().unwrap_or("").to_string();
                            let name = item["name"].as_str().unwrap_or("").to_string();
                            let args = item["arguments"].as_str().unwrap_or("{}");
                            let input: serde_json::Value =
                                serde_json::from_str(args).unwrap_or(serde_json::json!({}));
                            tool_calls.push(ContentBlock::ToolUse { id, name, input });
                        }
                    }

                    "response.completed" => {
                        let u = &ev["response"]["usage"];
                        if let Some(v) = u["input_tokens"].as_u64()  { usage_acc.input_tokens  = v as u32; }
                        if let Some(v) = u["output_tokens"].as_u64() { usage_acc.output_tokens = v as u32; }
                        // stream continues until [DONE]
                    }

                    // Ignore all other event types (response.created, response.in_progress, etc.)
                    _ => {}
                }
            }
        }

        if !text_acc.is_empty() && !self.suppress_stream {
            highlighter.flush();
        }
        if let Some(cb) = before_output.take() { cb(); }

        let stop_reason = if tool_calls.is_empty() { "end_turn".to_string() } else { "tool_use".to_string() };

        let mut content: Vec<ContentBlock> = Vec::new();
        if !text_acc.is_empty() {
            content.push(ContentBlock::Text { text: text_acc });
        }
        content.extend(tool_calls);

        let usage = if usage_acc.input_tokens > 0 || usage_acc.output_tokens > 0 {
            Some(usage_acc)
        } else {
            None
        };

        {
            let resp_val = serde_json::json!({
                "stop_reason": stop_reason,
                "usage": { "input_tokens": usage.as_ref().map(|u| u.input_tokens).unwrap_or(0),
                           "output_tokens": usage.as_ref().map(|u| u.output_tokens).unwrap_or(0) },
            });
            if let Ok(pretty) = serde_json::to_string_pretty(&resp_val) {
                crate::log::write_llm("RESPONSE [codex]", &pretty);
            }
        }

        Ok(ApiResponse { content, stop_reason, usage })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_only_user_message_stays_a_flat_string() {
        let msgs = [Message {
            role: "user".into(),
            content: vec![ContentBlock::Text { text: "hello".into() }],
        }];
        let input = encode_input(&msgs);
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"], "hello");
    }

    #[test]
    fn image_message_becomes_multipart_content() {
        let msgs = [Message {
            role: "user".into(),
            content: vec![
                ContentBlock::Text { text: "what is this".into() },
                ContentBlock::Image { media_type: "image/png".into(), data: "QUJD".into() },
            ],
        }];
        let input = encode_input(&msgs);
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
        let parts = input[0]["content"].as_array().expect("content must be an array with images");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["type"], "input_text");
        assert_eq!(parts[0]["text"], "what is this");
        assert_eq!(parts[1]["type"], "input_image");
        assert_eq!(parts[1]["image_url"], "data:image/png;base64,QUJD");
    }

    #[test]
    fn image_only_message_omits_the_text_part() {
        let msgs = [Message {
            role: "user".into(),
            content: vec![ContentBlock::Image { media_type: "image/jpeg".into(), data: "eHl6".into() }],
        }];
        let input = encode_input(&msgs);
        let parts = input[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["type"], "input_image");
        assert_eq!(parts[0]["image_url"], "data:image/jpeg;base64,eHl6");
    }
}
