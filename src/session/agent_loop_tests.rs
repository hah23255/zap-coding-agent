//! Deterministic agent-loop tests against [`crate::llm_client::mock::MockClient`].
//!
//! These cover the contract of [`Session::handle_user_turn`] — that it makes the
//! right number of LLM calls, executes tools when the model asks, terminates on
//! `end_turn`, and is bounded by `MAX_TURNS` for runaway tool loops.

use std::io::Write;

use serde_json::json;

use crate::config::{Config, PermissionMode, ProviderEntry, CODEX_CONTEXT_WINDOW};
use crate::llm_client::mock::MockClient;
use crate::llm_client::{ContentBlock, LlmProvider};

use super::{Session, MAX_TURNS};

fn test_config() -> Config {
    Config {
        model: "test-model".to_string(),
        permission_mode: PermissionMode::Auto,
        is_subagent: true,
        budget: None,
        ..Default::default()
    }
}

#[test]
fn codex_provider_uses_400k_context_for_any_model_name() {
    for model in ["gpt-5.5", "gpt-5.5-codex", "some-future-codex-model"] {
        let mut config = test_config();
        config.provider_slug = "codex".to_string();
        config.model = model.to_string();

        assert_eq!(super::configured_context_limit(&config), CODEX_CONTEXT_WINDOW);
    }
}

#[test]
fn codex_kind_uses_400k_context_even_with_custom_slug() {
    let mut config = test_config();
    config.provider_slug = "codex_alt".to_string();
    config.model = "gpt-5.5".to_string();
    config.all_providers.insert("codex_alt".to_string(), ProviderEntry {
        kind: Some("codex".to_string()),
        ..Default::default()
    });

    assert_eq!(super::configured_context_limit(&config), CODEX_CONTEXT_WINDOW);
}

#[test]
fn configured_provider_context_window_overrides_model_guess() {
    let mut config = test_config();
    config.provider_slug = "custom".to_string();
    config.model = "unknown-model".to_string();
    config.all_providers.insert("custom".to_string(), ProviderEntry {
        context_window: Some(123_456),
        ..Default::default()
    });

    assert_eq!(super::configured_context_limit(&config), 123_456);
}

#[tokio::test]
async fn single_text_turn_makes_one_call_and_appends_assistant_message() {
    let mock = MockClient::with_script(vec![MockClient::text("hello back")]);
    let session_client: Box<dyn LlmProvider> = Box::new(mock.clone());
    let mut session = Session::new_for_test(&test_config(), session_client).expect("session ctor");

    session.handle_user_turn("hi").await.expect("turn ran");

    assert_eq!(mock.call_count(), 1, "exactly one LLM call for a text-only turn");
    // user + assistant
    assert_eq!(session.messages.len(), 2);
    assert_eq!(session.messages[0].role, "user");
    assert_eq!(session.messages[1].role, "assistant");
    let assistant_text = session.messages[1].content.iter().find_map(|b| {
        if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None }
    });
    assert_eq!(assistant_text, Some("hello back"));
}

#[tokio::test]
async fn context_fill_pct_uses_windowed_history_not_full_log() {
    let mock = MockClient::with_script(vec![]);
    let session_client: Box<dyn LlmProvider> = Box::new(mock.clone());
    let mut session = Session::new_for_test(&test_config(), session_client).expect("session ctor");

    // One huge old turn that will fall outside the default 8-turn window...
    session.messages.push(crate::llm_client::Message::user_text("x".repeat(200_000)));
    session.messages.push(crate::llm_client::Message {
        role: "assistant".to_string(),
        content: vec![ContentBlock::Text { text: "ok".to_string() }],
    });
    // ...followed by enough small turns to push it out of the window.
    for i in 0..9 {
        session.messages.push(crate::llm_client::Message::user_text(format!("turn {i}")));
        session.messages.push(crate::llm_client::Message {
            role: "assistant".to_string(),
            content: vec![ContentBlock::Text { text: "ok".to_string() }],
        });
    }

    // The full unwindowed log is dominated by the huge first turn (sanity check)...
    assert!(session.estimated_context_tokens() > 30_000, "sanity: full log should be huge");
    // ...but context_fill_pct() — what's displayed, and what the auto-compact
    // trigger is now based on — only reflects the windowed (last 8 user turns)
    // view, which excludes that huge turn entirely. Before this fix, the
    // auto-compact trigger used the full count above instead, so it could fire
    // at a percentage the status bar never showed.
    let pct = session.context_fill_pct();
    assert!(pct < 5, "windowed view should be tiny, got {pct}%");
}

#[tokio::test]
async fn context_fill_pct_includes_dropped_summary() {
    let mock = MockClient::with_script(vec![]);
    let session_client: Box<dyn LlmProvider> = Box::new(mock.clone());
    let mut session = Session::new_for_test(&test_config(), session_client).expect("session ctor");

    let pct_before = session.context_fill_pct();
    session.dropped_summary = "y".repeat(40_000);
    let pct_after = session.context_fill_pct();
    assert!(pct_after > pct_before, "dropped_summary should count toward context_fill_pct (before: {pct_before}%, after: {pct_after}%)");
}

#[tokio::test]
async fn one_tool_round_executes_tool_and_loops_back() {
    // Stage a temp file the model "asks" to read.
    let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
    writeln!(tmp, "line one").unwrap();
    writeln!(tmp, "line two").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mock = MockClient::with_script(vec![
        MockClient::tool_call("call_1", "read_file", json!({ "path": path })),
        MockClient::text("done reading"),
    ]);
    let session_client: Box<dyn LlmProvider> = Box::new(mock.clone());
    let mut session = Session::new_for_test(&test_config(), session_client).expect("session ctor");

    session.handle_user_turn("read it").await.expect("turn ran");

    assert_eq!(mock.call_count(), 2, "tool round = one LLM call + one follow-up");

    // Messages: user, assistant (tool_use), user (tool_result), assistant (final text).
    assert_eq!(session.messages.len(), 4, "user + tool_use + tool_result + assistant");

    let tool_result = session.messages[2].content.iter().find_map(|b| {
        if let ContentBlock::ToolResult { content, tool_use_id } = b {
            Some((tool_use_id.as_str(), content.as_str()))
        } else { None }
    });
    let (tool_use_id, body) = tool_result.expect("tool_result block present");
    assert_eq!(tool_use_id, "call_1");
    assert!(body.contains("line one"), "tool actually read the file: {body}");
    assert!(body.contains("line two"));

    // The second LLM call must have included the tool_result message.
    let calls = mock.recorded_calls();
    let second_call_msgs = &calls[1].messages;
    let has_tool_result = second_call_msgs.iter().any(|m| {
        m.content.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. }))
    });
    assert!(has_tool_result, "second call must carry the tool result back to the model");
}

#[tokio::test]
async fn runaway_tool_calls_stop_at_max_turns() {
    // Seed enough tool calls that the loop would run forever without the cap.
    let mut script: Vec<crate::llm_client::ApiResponse> = (0..MAX_TURNS + 5)
        .map(|i| MockClient::tool_call(format!("call_{i}"), "read_file", json!({ "path": "/dev/null" })))
        .collect();
    // A trailing text — should never be reached.
    script.push(MockClient::text("should not see this"));

    let mock = MockClient::with_script(script);
    let session_client: Box<dyn LlmProvider> = Box::new(mock.clone());
    let mut session = Session::new_for_test(&test_config(), session_client).expect("session ctor");

    session.handle_user_turn("loop please").await.expect("turn ran");

    // The loop body runs MAX_TURNS times — one LLM call per iteration.
    assert_eq!(
        mock.call_count(),
        MAX_TURNS,
        "loop must stop exactly at MAX_TURNS",
    );
}

#[tokio::test]
async fn edit_ledger_appears_on_next_turn() {
    // ── Arrange ──────────────────────────────────────────────────────────
    let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
    writeln!(tmp, "original").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mock = MockClient::with_script(vec![
        // Turn 1 iter 0: model asks to write the file.
        MockClient::tool_call("call_1", "write_file", json!({ "path": path, "content": "modified" })),
        // Turn 1 iter 1: after tool executes, model ends the turn.
        MockClient::text("file written"),
        // Turn 2: model responds to question about edits.
        MockClient::text("you edited a file"),
    ]);
    let session_client: Box<dyn LlmProvider> = Box::new(mock.clone());
    let mut session = Session::new_for_test(&test_config(), session_client).expect("session ctor");

    // ── Act ──────────────────────────────────────────────────────────────
    session.handle_user_turn("write to the config").await.expect("turn 1");
    session.handle_user_turn("what files did we edit?").await.expect("turn 2");

    // ── Assert ───────────────────────────────────────────────────────────
    let calls = mock.recorded_calls();
    // calls[0] = turn 1 first LLM call (before tool exec — no ledger yet)
    // calls[1] = turn 1 second LLM call (same effective_system, still no ledger)
    // calls[2] = turn 2 first LLM call (ledger should be present)
    assert!(calls.len() >= 3, "expected at least 3 recorded calls");

    // Turn 1 calls: effective_system was computed before the tool ran, so
    // edited_files was empty — neither call should contain the ledger.
    assert!(
        !calls[0].system.contains("Edit Ledger"),
        "turn 1 call 0 should not have edit ledger yet"
    );
    assert!(
        !calls[1].system.contains("Edit Ledger"),
        "turn 1 call 1 should not have edit ledger yet"
    );

    // Turn 2: effective_system is recomputed with edited_files now populated.
    let turn2_system = &calls[2].system;
    assert!(
        turn2_system.contains("Edit Ledger"),
        "turn 2 system prompt should contain edit ledger header"
    );
    assert!(
        turn2_system.contains(&path),
        "edit ledger should mention the edited path"
    );
    assert!(
        turn2_system.contains("turn 1"),
        "edit ledger should record turn 1 as the edit turn"
    );
    assert!(
        turn2_system.contains("1 op"),
        "edit ledger should show op count"
    );
}

#[tokio::test]
async fn edit_ledger_persists_after_turns_slide_out_of_window() {
    // ── Arrange ──────────────────────────────────────────────────────────
    let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
    writeln!(tmp, "original").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    const TOTAL_TURNS: usize = 13;  // window=8, so turn 1 slides out by turn 9+

    let mut script: Vec<crate::llm_client::ApiResponse> = Vec::with_capacity(TOTAL_TURNS + 1);
    // Turn 1: tool call + follow-up text
    script.push(MockClient::tool_call(
        "call_1", "write_file",
        json!({ "path": path, "content": "modified" }),
    ));
    script.push(MockClient::text("file written"));
    // Turns 2..TOTAL_TURNS: one text response each
    for i in 2..=TOTAL_TURNS {
        script.push(MockClient::text(format!("done turn {i}")));
    }

    let mock = MockClient::with_script(script);
    let session_client: Box<dyn LlmProvider> = Box::new(mock.clone());
    let mut session = Session::new_for_test(&test_config(), session_client).expect("session ctor");

    // ── Act ──────────────────────────────────────────────────────────────
    session.handle_user_turn("edit the auth config file").await.expect("turn 1");
    for i in 2..=TOTAL_TURNS {
        // Non-casual prompts so the ledger is injected every turn.
        let input = format!("Refactor module {i} for technical debt");
        session.handle_user_turn(&input).await.expect("turn N");
    }

    // ── Assert ───────────────────────────────────────────────────────────
    let calls = mock.recorded_calls();
    let last_system = &calls.last().expect("at least one call").system;
    assert!(
        last_system.contains("Edit Ledger"),
        "system prompt after {} turns should contain edit ledger header",
        TOTAL_TURNS,
    );
    assert!(
        last_system.contains(&path),
        "edit ledger should still mention file edited on turn 1, after window slid"
    );
    assert!(
        last_system.contains("turn 1"),
        "edit ledger should record turn 1 as first_turn, even after window slid"
    );
}

// ── Image paste E2E tests ─────────────────────────────────────────────────────

/// A staged image must arrive in the LLM call as a `ContentBlock::Image` block
/// inside the user message, with the correct MIME type and base64 payload.
#[tokio::test]
async fn staged_image_included_in_llm_request() {
    use base64::Engine;

    let mock = MockClient::with_script(vec![MockClient::text("I can see the image.")]);
    let session_client: Box<dyn LlmProvider> = Box::new(mock.clone());
    let mut session = Session::new_for_test(&test_config(), session_client).expect("session ctor");

    // Synthetic payload — staged images are pre-encoded, so MIN_IMAGE_BYTES guard is irrelevant.
    let fake_payload = vec![0u8; 256];
    let b64 = base64::engine::general_purpose::STANDARD.encode(&fake_payload);
    session.staged_images.push(("image/png".to_string(), b64.clone()));

    session.handle_user_turn("what is in this image?").await.expect("turn ran");

    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1, "one LLM call expected");

    // The most recent user message is the first message in the call (fresh session).
    let user_msg = calls[0]
        .messages
        .iter()
        .find(|m| m.role == "user")
        .expect("user message must be present in LLM call");

    let image_block = user_msg.content.iter().find_map(|b| {
        if let ContentBlock::Image { media_type, data } = b {
            Some((media_type, data))
        } else {
            None
        }
    });
    let (media_type, data) = image_block.expect("Image content block must be present");
    assert_eq!(media_type, "image/png");
    assert_eq!(data, &b64, "base64 payload must be forwarded unchanged");

    // Text block must also be present.
    let has_text = user_msg.content.iter().any(|b| {
        matches!(b, ContentBlock::Text { text } if text.contains("what is in this image?"))
    });
    assert!(has_text, "user text must accompany the image block");

    // Image should precede text in the block ordering.
    let image_pos = user_msg.content.iter().position(|b| matches!(b, ContentBlock::Image { .. }));
    let text_pos  = user_msg.content.iter().position(|b| matches!(b, ContentBlock::Text { .. }));
    assert!(
        image_pos < text_pos,
        "image block must come before the text block (image={image_pos:?} text={text_pos:?})"
    );
}

/// After images are sent in turn N they must NOT be re-sent in turn N+1.
#[tokio::test]
async fn staged_images_not_resent_on_subsequent_turn() {
    use base64::Engine;

    let mock = MockClient::with_script(vec![
        MockClient::text("got the image"),
        MockClient::text("got no image"),
    ]);
    let session_client: Box<dyn LlmProvider> = Box::new(mock.clone());
    let mut session = Session::new_for_test(&test_config(), session_client).expect("session ctor");

    let fake_payload = vec![42u8; 256];
    let b64 = base64::engine::general_purpose::STANDARD.encode(&fake_payload);
    session.staged_images.push(("image/png".to_string(), b64));

    // Turn 1 — image staged
    session.handle_user_turn("describe this").await.expect("turn 1");
    assert!(session.staged_images.is_empty(), "staged_images must be cleared after the turn");

    // Turn 2 — no new image staged
    session.handle_user_turn("follow-up question").await.expect("turn 2");

    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 2);

    // Turn 2's messages include everything in session history. The second user
    // message is the "follow-up question" one — it must contain only a Text block.
    let turn2_user_msgs: Vec<_> = calls[1]
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .collect();
    let last_user = turn2_user_msgs.last().expect("at least one user message in turn 2");
    let has_image_in_turn2 = last_user.content.iter().any(|b| matches!(b, ContentBlock::Image { .. }));
    assert!(!has_image_in_turn2, "turn 2 user message must NOT contain an image block");
}
