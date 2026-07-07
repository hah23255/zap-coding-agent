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
    /// Mapped from zap's own mode:
    ///   Auto → bypassPermissions (run everything, matches zap's own auto mode)
    ///   Ask  → bypassPermissions too. Verified directly (2026-07): running the
    ///          `claude` CLI with `--permission-mode default` just silently denies
    ///          the tool (shows up in the `result` event's `permission_denials`)
    ///          and has the model say "please approve the permission prompt" in
    ///          plain text — there is no prompt to approve. The `control_request`/
    ///          `can_use_tool` protocol some blog posts describe belongs to the
    ///          Agent SDK (an embedded Python/TS library) — `claude --help` on the
    ///          installed CLI has no `--permission-prompt-tool` flag at all, so
    ///          that protocol isn't reachable from a subprocess-driven integration
    ///          like this one. Ask mode is a guaranteed dead end here, so it falls
    ///          back to Auto with a one-time notice instead of silently stalling.
    ///   Deny → plan (read-only: Claude Code may explore but cannot edit files or
    ///          run mutating commands, matching zap's own Deny semantics — this one
    ///          doesn't need interactivity, so it works as intended)
    permission_mode: &'static str,
    /// claude CLI session id captured from the init event. When present,
    /// subsequent turns are sent with `--resume <id>` and only the new user
    /// message is transmitted — claude keeps the conversation state itself.
    /// Without this, replaying the full history as separate stream-json user
    /// events makes claude re-answer every past message on every turn.
    session_id: Mutex<Option<String>>,
}

impl ClaudeCodeClient {
    pub fn new(model: String, suppress_stream: bool, permission_mode: crate::config::PermissionMode) -> Self {
        use crate::config::PermissionMode;
        if matches!(permission_mode, PermissionMode::Ask) {
            let msg = "Claude Code can't prompt for per-edit approval when driven \
                       headlessly by zap — there's no channel to answer it on, so it \
                       would only stall. Using Auto (bypassPermissions) for this \
                       session instead. Use Deny for a read-only session.".to_string();
            crate::zap_warn!("{}", msg);
            if crate::tui::channel::is_tui_mode() {
                crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::Notice(msg));
            }
        }
        let permission_mode = match permission_mode {
            PermissionMode::Auto | PermissionMode::Ask => "bypassPermissions",
            PermissionMode::Deny => "plan",
        };
        Self {
            model,
            suppress_stream,
            permission_mode,
            session_id: Mutex::new(None),
        }
    }
}

/// Claude Code's `--print`/stream-json protocol has no structured field for the
/// 5-hour/weekly usage window (unlike Codex's `x-codex-*-used-percent` response
/// headers — see `quota_watch`). Anthropic doesn't expose a headless usage query
/// either (open feature requests: anthropics/claude-code#20399, #38380), so the
/// only signal available here is the wording of the error Claude Code gives back
/// once the window is already exhausted. This turns that into an explicit,
/// actionable warning instead of a bare error string.
fn warn_if_usage_limit(text: &str) {
    let lower = text.to_lowercase();
    let looks_like_quota = lower.contains("usage limit")
        || lower.contains("session limit")
        || (lower.contains("limit") && lower.contains("reset"));
    if !looks_like_quota {
        return;
    }
    let msg = format!(
        "⚠ Claude usage limit reached for this window — switch providers \
         (`/provider codex` or another) until it resets. ({})",
        text.trim()
    );
    crate::zap_warn!("{}", msg);
    if crate::tui::channel::is_tui_mode() {
        crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::Warning(msg));
    }
}

/// Extracts complete text blocks from an "assistant" event's `content` array.
/// Without `--include-partial-messages`, each block Claude Code emits is
/// already whole — never a growing delta of one in-progress block, confirmed
/// directly against the CLI (a single 881-char answer arrived as one event,
/// not split across chunks). Two blocks in the same event, or across separate
/// events sharing a message id, are independent complete units, not partial
/// pieces of a running total — the caller must not diff them by length.
fn text_blocks(content: &serde_json::Value) -> Vec<String> {
    content
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|b| b["type"].as_str() == Some("text"))
                .filter_map(|b| b["text"].as_str())
                .filter(|t| !t.is_empty())
                .map(|t| t.to_string())
                .collect()
        })
        .unwrap_or_default()
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
        // Best-effort 5-hour/weekly usage check — no-ops past the first call
        // in a session until the 5-minute recheck window elapses. See
        // `quota_watch` module docs for why this can't come from the CLI itself.
        crate::quota_watch::check_claude_usage_if_stale().await;

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
                    // Without --include-partial-messages (not passed — see cmd
                    // construction above), Claude Code does not stream text as
                    // growing deltas: each "assistant" event carries exactly one
                    // new, already-complete content block (thinking, text, or
                    // tool_use), even when several events share the same
                    // message.id across a multi-step turn. Verified directly
                    // against the real CLI (2026-07): a single 881-char answer
                    // arrived as one event with the full text, never split
                    // across growing chunks.
                    //
                    // The previous logic here diffed against a running
                    // "previous length" as if blocks were cumulative deltas of
                    // one message. That assumption is wrong for this protocol —
                    // two independent, complete text blocks (e.g. two separate
                    // "Let me check X" narration lines with no tool call
                    // between them) could end up with the second block's
                    // *length* coincidentally not shorter than the first's,
                    // so the "new message" reset never fired and the code
                    // sliced off part of the second block's own text instead of
                    // treating it as new content — producing glued-together,
                    // missing-space output like "…defined.Let me check…".
                    //
                    // Fix: treat every text block as a complete, standalone
                    // unit and just separate consecutive ones with a blank line.
                    for text in text_blocks(&ev["message"]["content"]) {
                        if !full_text.is_empty() && !full_text.ends_with('\n') {
                            full_text.push_str("\n\n");
                        }
                        crate::remote_channel::send_chunk(&text);
                        if !self.suppress_stream {
                            if let Some(cb) = before_output.take() { cb(); }
                            highlighter.push(&text);
                        }
                        full_text.push_str(&text);
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
                    warn_if_usage_limit(msg);
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
            let detail = if full_text.is_empty() { stderr_text.trim().to_string() } else { full_text };
            warn_if_usage_limit(&detail);
            anyhow::bail!("Claude Code reported an error: {detail}");
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

    /// Mirrors the accumulation loop in `send()`'s "assistant" branch, minus
    /// the streaming side effects, so the joining behavior is testable.
    fn accumulate(events: &[serde_json::Value]) -> String {
        let mut full_text = String::new();
        for ev in events {
            for text in text_blocks(&ev["message"]["content"]) {
                if !full_text.is_empty() && !full_text.ends_with('\n') {
                    full_text.push_str("\n\n");
                }
                full_text.push_str(&text);
            }
        }
        full_text
    }

    #[test]
    fn text_blocks_extracts_only_text_type_and_skips_empty() {
        let content = serde_json::json!([
            {"type": "thinking", "thinking": "pondering"},
            {"type": "text", "text": "hello"},
            {"type": "tool_use", "name": "Read", "input": {}},
            {"type": "text", "text": ""},
        ]);
        assert_eq!(text_blocks(&content), vec!["hello".to_string()]);
    }

    #[test]
    fn two_independent_text_blocks_get_a_blank_line_not_glued_together() {
        // Regression test: this exact pair (shorter block followed by a
        // longer, entirely unrelated one, no tool call in between) is what
        // corrupted output in production. The old length-diffing logic
        // treated the second block as a continuation of the first whenever
        // its length wasn't shorter, silently slicing off however many
        // characters matched the first block's length and gluing the
        // remainder straight on with no separator — e.g. dropping "Let me "
        // and producing "…defined.check how the model is passed…".
        let events = [
            serde_json::json!({"message": {"content": [
                {"type": "text", "text": "Let me find where Claude models are defined."}
            ]}}),
            serde_json::json!({"message": {"content": [
                {"type": "text", "text": "Let me check how the model is passed to the claude CLI and what auto means there."}
            ]}}),
        ];
        let result = accumulate(&events);
        assert_eq!(
            result,
            "Let me find where Claude models are defined.\n\n\
             Let me check how the model is passed to the claude CLI and what auto means there."
        );
        assert!(!result.contains("defined.Let"), "blocks must not be glued together");
    }

    #[test]
    fn thinking_and_tool_use_blocks_produce_no_text_and_no_stray_separators() {
        let events = [
            serde_json::json!({"message": {"content": [{"type": "text", "text": "Reading both files."}]}}),
            serde_json::json!({"message": {"content": [{"type": "tool_use", "name": "Read", "input": {}}]}}),
            serde_json::json!({"message": {"content": [{"type": "tool_use", "name": "Read", "input": {}}]}}),
            serde_json::json!({"message": {"content": [{"type": "text", "text": "Cargo.toml is the package manifest."}]}}),
        ];
        let result = accumulate(&events);
        assert_eq!(result, "Reading both files.\n\nCargo.toml is the package manifest.");
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
