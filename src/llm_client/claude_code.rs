use anyhow::Result;
use async_trait::async_trait;
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use super::{ApiResponse, BeforeOutput, ContentBlock, LlmProvider, Message, Usage};

pub struct ClaudeCodeClient {
    model: String,
    suppress_stream: bool,
    /// Claude Code permission mode passed as `--permission-mode`.
    /// Mapped from zap's own mode: auto → bypassPermissions, ask/deny → acceptEdits.
    permission_mode: &'static str,
    /// claude CLI session id captured from the init event. When present,
    /// subsequent turns are sent with `--resume <id>` and only the new user
    /// message is transmitted — claude keeps the conversation state itself.
    /// Without this, replaying the full history as separate stream-json user
    /// events makes claude re-answer every past message on every turn.
    session_id: Mutex<Option<String>>,
}

impl ClaudeCodeClient {
    pub fn new(model: String, suppress_stream: bool, zap_auto: bool) -> Self {
        Self {
            model,
            suppress_stream,
            permission_mode: if zap_auto { "bypassPermissions" } else { "acceptEdits" },
            session_id: Mutex::new(None),
        }
    }
}

/// Finds the claude binary from PATH or common brew/system install locations.
/// Cached — probing costs a `claude --version` subprocess (~0.5s), which must
/// not be paid on every turn.
fn find_claude() -> &'static str {
    static FOUND: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    FOUND.get_or_init(|| {
        let candidates: &[&str] = &["claude", "/opt/homebrew/bin/claude", "/usr/local/bin/claude"];
        for &c in candidates {
            if Command::new(c).arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
                return c;
            }
        }
        "claude"
    })
}

/// Content blocks of one message rendered to Anthropic-format JSON blocks.
/// Tool calls/results are flattened to text; images pass through as base64.
fn blocks_to_json(msg: &Message) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut text_parts: Vec<&str> = Vec::new();
    for b in &msg.content {
        match b {
            ContentBlock::Text { text } => text_parts.push(text.as_str()),
            ContentBlock::ToolResult { content, .. } => text_parts.push(content.as_str()),
            ContentBlock::Image { media_type, data } => {
                out.push(serde_json::json!({
                    "type": "image",
                    "source": {"type": "base64", "media_type": media_type, "data": data}
                }));
            }
            _ => {}
        }
    }
    let text = text_parts.join("\n").trim().to_string();
    if !text.is_empty() {
        out.push(serde_json::json!({"type": "text", "text": text}));
    }
    out
}

fn plain_text(msg: &Message) -> String {
    msg.content.iter().filter_map(|b| match b {
        ContentBlock::Text { text } => Some(text.as_str()),
        ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
        _ => None,
    }).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Encode the conversation as a SINGLE stream-json user event.
///
/// `claude -p --input-format stream-json` treats every user event as a separate
/// prompt and answers each one, so the whole send must collapse to one event.
///
/// - `resuming == true`: only messages after the last assistant reply are sent
///   (claude already has the rest of the conversation via --resume).
/// - `resuming == false`: earlier turns are inlined as a plain-text transcript
///   preamble, followed by the current user message (with any images).
fn encode_single_event(messages: &[Message], resuming: bool) -> String {
    let last_assistant = messages.iter().rposition(|m| m.role == "assistant");
    let tail_start = last_assistant.map(|i| i + 1).unwrap_or(0);

    let mut blocks: Vec<serde_json::Value> = Vec::new();

    if !resuming && tail_start > 0 {
        let mut transcript = String::from(
            "Prior conversation (for context — do not re-answer these):\n\n");
        for msg in &messages[..tail_start] {
            let text = plain_text(msg);
            if text.is_empty() { continue; }
            let who = if msg.role == "assistant" { "Assistant" } else { "User" };
            transcript.push_str(&format!("{who}: {text}\n\n"));
        }
        transcript.push_str("Now respond to the following message:");
        blocks.push(serde_json::json!({"type": "text", "text": transcript}));
    }

    for msg in &messages[tail_start..] {
        if msg.role != "user" { continue; }
        blocks.extend(blocks_to_json(msg));
    }

    if blocks.is_empty() {
        blocks.push(serde_json::json!({"type": "text", "text": "(continue)"}));
    }

    let event = serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": blocks}
    });
    let mut out = serde_json::to_string(&event).unwrap_or_default();
    out.push('\n');
    out
}

#[async_trait]
impl LlmProvider for ClaudeCodeClient {
    async fn send(
        &self,
        system: &str,
        messages: &[Message],
        _tools: &[serde_json::Value],
        before_output: Option<BeforeOutput>,
        _thinking_budget: u32,
    ) -> Result<ApiResponse> {
        let mut before_output = before_output;
        let mut highlighter = crate::stream_highlighter::StreamHighlighter::new();
        highlighter.suppress_print = crate::tui::channel::is_tui_mode();

        let resume_id = self.session_id.lock().ok().and_then(|g| g.clone());
        let stdin_data = encode_single_event(messages, resume_id.is_some());
        let claude_bin = find_claude();

        let mut cmd = Command::new(claude_bin);
        cmd.arg("--print")
            .args(["--output-format", "stream-json"])
            .arg("--verbose")
            .args(["--input-format", "stream-json"])
            .args(["--model", &self.model])
            .args(["--permission-mode", self.permission_mode]);

        if let Some(ref sid) = resume_id {
            cmd.args(["--resume", sid]);
        }

        if !system.is_empty() {
            // Rust's Command::arg avoids shell quoting entirely — long strings are fine.
            cmd.args(["--append-system-prompt", system]);
        }

        cmd.stdin(Stdio::piped())
           .stdout(Stdio::piped())
           .stderr(Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to launch claude CLI: {e}. Is Claude Code installed?"))?;

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(stdin_data.as_bytes());
            // Drop closes stdin, signalling EOF to claude.
        }

        let stdout = child.stdout.take()
            .ok_or_else(|| anyhow::anyhow!("No stdout from claude process"))?;

        // Capture stderr in the background — surfaced when claude produces no output.
        let stderr_buf = child.stderr.take().map(|se| {
            std::thread::spawn(move || {
                use std::io::Read as _;
                let mut buf = String::new();
                let _ = std::io::BufReader::new(se).read_to_string(&mut buf);
                buf
            })
        });

        // Read subprocess stdout in a blocking thread and forward lines via channel.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        tokio::task::spawn_blocking(move || {
            use std::io::BufRead as _;
            for l in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if !l.is_empty() { let _ = tx.send(l); }
            }
        });

        let mut full_text = String::new();
        let mut prev_len = 0usize;
        let mut stop_reason = "end_turn".to_string();
        let mut usage = Usage::default();
        let mut result_is_error = false;

        while let Some(line) = rx.recv().await {
            let Ok(ev) = serde_json::from_str::<serde_json::Value>(&line) else { continue };

            match ev["type"].as_str().unwrap_or("") {
                "system" if ev["subtype"].as_str() == Some("init") => {
                    if let Some(sid) = ev["session_id"].as_str() {
                        if let Ok(mut g) = self.session_id.lock() { *g = Some(sid.to_string()); }
                    }
                }

                "assistant" => {
                    // Collect text from all content blocks in this event.
                    let text: String = ev["message"]["content"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|b| {
                                    if b["type"].as_str() == Some("text") {
                                        b["text"].as_str().map(|s| s.to_string())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("")
                        })
                        .unwrap_or_default();

                    // Each assistant event carries that message's full text so far;
                    // across messages the text restarts. Append only what's new,
                    // treating a shorter text as the start of a new message.
                    if text.len() < prev_len {
                        prev_len = 0;
                        if !full_text.is_empty() && !full_text.ends_with('\n') {
                            full_text.push_str("\n\n");
                        }
                    }
                    if text.len() > prev_len {
                        let chunk = text[prev_len..].to_string();
                        crate::remote_channel::send_chunk(&chunk);
                        if !self.suppress_stream {
                            if let Some(cb) = before_output.take() { cb(); }
                            highlighter.push(&chunk);
                        }
                        full_text.push_str(&chunk);
                        prev_len = text.len();
                    }

                    if let Some(u) = ev["message"]["usage"].as_object() {
                        usage.input_tokens  = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        usage.output_tokens += u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        usage.cache_read_tokens  = u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        usage.cache_write_tokens = u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    }
                }

                "result" => {
                    if let Some(r) = ev["stop_reason"].as_str() { stop_reason = r.to_string(); }
                    result_is_error = ev["is_error"].as_bool().unwrap_or(false)
                        || ev["subtype"].as_str().map(|s| s.starts_with("error")).unwrap_or(false);
                    // Session id also appears on the result event.
                    if let Some(sid) = ev["session_id"].as_str() {
                        if let Ok(mut g) = self.session_id.lock() { *g = Some(sid.to_string()); }
                    }
                    // Final usage (authoritative totals from claude).
                    if let Some(u) = ev["usage"].as_object() {
                        if let Some(v) = u.get("input_tokens").and_then(|v| v.as_u64())  { usage.input_tokens  = v as u32; }
                        if let Some(v) = u.get("output_tokens").and_then(|v| v.as_u64()) { usage.output_tokens = v as u32; }
                        if let Some(v) = u.get("cache_read_input_tokens").and_then(|v| v.as_u64()) { usage.cache_read_tokens = v as u32; }
                    }
                    // Fallback: if no assistant events came through, use the result text.
                    if full_text.is_empty() {
                        if let Some(t) = ev["result"].as_str() {
                            full_text = t.to_string();
                            crate::remote_channel::send_chunk(&full_text);
                            if !self.suppress_stream {
                                if let Some(cb) = before_output.take() { cb(); }
                                highlighter.push(&full_text);
                            }
                        }
                    }
                }

                "system" if ev["subtype"].as_str() == Some("error") => {
                    let msg = ev["error"]["message"].as_str().unwrap_or("unknown error");
                    anyhow::bail!("Claude Code error: {msg}");
                }

                _ => {}
            }
        }

        let status = child.wait();
        let stderr_text = stderr_buf
            .and_then(|h| h.join().ok())
            .unwrap_or_default();

        if result_is_error {
            // A failed --resume (e.g. claude pruned the session) must not poison
            // every subsequent turn — drop the id so the next turn starts fresh.
            if resume_id.is_some() {
                if let Ok(mut g) = self.session_id.lock() { *g = None; }
            }
            anyhow::bail!(
                "Claude Code reported an error: {}",
                if full_text.is_empty() { stderr_text.trim().to_string() } else { full_text }
            );
        }

        if full_text.is_empty() {
            if resume_id.is_some() {
                if let Ok(mut g) = self.session_id.lock() { *g = None; }
            }
            let exit = status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
            let detail = if stderr_text.trim().is_empty() {
                String::new()
            } else {
                format!("\nclaude stderr: {}", stderr_text.trim())
            };
            anyhow::bail!(
                "Claude Code returned empty response ({exit}).{detail}\n\
                 Ensure `claude` CLI is installed and authenticated (run `claude` once to log in)."
            );
        }

        Ok(ApiResponse {
            content: vec![ContentBlock::Text { text: full_text }],
            stop_reason,
            usage: Some(usage),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> Message {
        Message { role: "user".into(), content: vec![ContentBlock::Text { text: text.into() }] }
    }
    fn assistant(text: &str) -> Message {
        Message { role: "assistant".into(), content: vec![ContentBlock::Text { text: text.into() }] }
    }

    #[test]
    fn single_event_fresh_first_turn() {
        let enc = encode_single_event(&[user("hello")], false);
        assert_eq!(enc.lines().count(), 1, "must be exactly one stream-json event");
        let v: serde_json::Value = serde_json::from_str(enc.trim()).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["message"]["content"][0]["text"], "hello");
    }

    #[test]
    fn single_event_fresh_multi_turn_inlines_transcript() {
        let msgs = [user("first"), assistant("answer one"), user("second")];
        let enc = encode_single_event(&msgs, false);
        assert_eq!(enc.lines().count(), 1, "history must collapse to one event");
        let v: serde_json::Value = serde_json::from_str(enc.trim()).unwrap();
        let text0 = v["message"]["content"][0]["text"].as_str().unwrap();
        assert!(text0.contains("first") && text0.contains("answer one"));
        let text1 = v["message"]["content"][1]["text"].as_str().unwrap();
        assert_eq!(text1, "second");
    }

    #[test]
    fn single_event_resume_sends_only_tail() {
        let msgs = [user("first"), assistant("answer one"), user("second")];
        let enc = encode_single_event(&msgs, true);
        let v: serde_json::Value = serde_json::from_str(enc.trim()).unwrap();
        let blocks = v["message"]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["text"], "second");
    }

    #[test]
    fn images_pass_through() {
        let msg = Message {
            role: "user".into(),
            content: vec![
                ContentBlock::Image { media_type: "image/png".into(), data: "QUJD".into() },
                ContentBlock::Text { text: "what is this".into() },
            ],
        };
        let enc = encode_single_event(&[msg], false);
        let v: serde_json::Value = serde_json::from_str(enc.trim()).unwrap();
        let blocks = v["message"]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "image");
        assert_eq!(blocks[0]["source"]["data"], "QUJD");
        assert_eq!(blocks[1]["type"], "text");
    }
}
