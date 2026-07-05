# Background Agents (`/bg`, `/agents`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `/bg <goal> [--model <slug>]` and `/agents` (list / `view <id>` / `kill <id>`) so a user can fire off several independent tasks — each optionally on a different model — as detached background agents inside the running TUI, and monitor them without blocking the main conversation.

**Architecture:** Reuse the existing model-invoked sub-agent machinery (`agent_core::run_subagent`, backed by a fresh `Session` with its own history/model) but make it user-invoked and non-blocking: `tokio::spawn` instead of `.await`, tracked in a new `App.background_agents` registry. Model selection reuses the exact lookup `session::routing::route_for_turn` already does (`task_classifier` + `model_routes`), just invoked outside the turn loop. Completion is reported back to the TUI over the existing `TuiEvent` channel. Along the way, this fixes a real latent bug: destructive shell commands under `is_subagent = true` currently queue an interactive prompt that can never be answered (no controlling terminal) — instead of hanging, they now auto-deny with a clear reason.

**Tech Stack:** Existing stack only — `tokio::spawn` (already used by `session/scheduler.rs`), the existing SQLite `sessions`/`session_messages` tables (already durable; no migration), the existing `TuiEvent` mpsc channel (`tui/channel.rs`). No new crates.

**Key discoveries that shape this plan (read before starting):**
- `agent_core::run_subagent` (agent_core.rs:312) already builds an independent `Session` with its own history for a given goal, and already extracts `{summary, files_changed, turns, tool_calls}` from it inline. Task 4 pulls that extraction into a shared, testable function so both `run_subagent` and the new background-agent path use one implementation.
- `Session::new` (session/mod.rs:225) currently sets `session_id = 0` (not persisted) whenever `config.is_subagent` is true — this is intentional for model-invoked sub-agents (avoids bloating `agent.db` with short-lived rows), but it would also silently defeat background-agent transcript persistence if left as-is, since background agents need `is_subagent = true` too (for banner suppression and the permission fix below). Task 2 adds a second, independent config flag so background agents get persisted while model-invoked sub-agents don't.
- `session/tools.rs`'s `force_prompt` check (session/tools.rs:82-93) queues a destructive shell command for interactive approval regardless of permission mode. `run_subagent` forces `PermissionMode::Auto` specifically because "no controlling terminal, prompting would deadlock" — but this destructive-pattern check bypasses that entirely today. Task 3 fixes it, gated on `is_subagent` (which covers both model-invoked sub-agents and the new background agents).
- `/model` (session/commands/session_mgmt.rs:100) and `session::routing::route_for_turn` both switch models by cloning `Config`, overwriting only `.model`, and calling `llm_client::create_client`. Neither touches `.provider`/`.provider_slug`. This means model switching — including `/bg --model <slug>` — only works within the *currently configured provider*; it cannot switch to a different provider's CLI-passthrough client (e.g. `codex`) from a different active provider. This is existing, accepted behavior, not a gap introduced here.
- `/schedule`/`/unschedule` (tui/schedule_handler.rs) are TUI-only — there is no CLI-mode (`--cli`) dispatch for them at all, and the project already has a cheap e2e pattern for this exact situation: assert the command falls through to "unknown command" in CLI mode instead of trying to drive the real TUI (tests/e2e/test_scheduler.sh). `/bg`/`/agents` follow the same pattern.
- IDs: rather than adding a new dependency for random IDs, `App` gets a simple monotonic `next_bg_id: u32` counter formatted as a plain decimal string ("1", "2", "3", ...). Simpler than a hex slug, no collision handling needed, and easy to type in `/agents view 1`.
- Live token-by-token progress for a *running* background agent is intentionally out of scope (would require new shared-mutable-state plumbing threaded through `session/turn.rs`, which the user explicitly deprioritized — "UI is not so imp... implement feature, UI can be added later"). `/agents view <id>` on a running agent shows elapsed time only; full detail (summary, files changed, turn/tool counts) appears once it's `done`.

---

## File Map

| File | Action | Responsibility |
|------|--------|-----------------|
| `src/config/mod.rs` | Modify | Add `is_background_agent: bool`, `max_background_agents: usize` to `Config`; `max_background_agents: Option<usize>` to `FileConfig`; wire through `Config::load` and the test `Default` impl |
| `src/config/tests.rs` | Modify | Tests for `max_background_agents` TOML parsing + default |
| `src/session/mod.rs` | Modify | Add `should_persist_session()` helper; fix the `session_id` gate in `Session::new`; add `pub mod background_agent;` |
| `src/session/agent_loop_tests.rs` | Modify | Tests for `should_persist_session`, the destructive-pattern fix, and `agent_core::extract_result` |
| `src/session/tools.rs` | Modify | Auto-deny destructive shell commands under `is_subagent` instead of queuing an unanswerable prompt |
| `src/agent_core.rs` | Modify | Extract `SubagentResult` + `extract_result()` out of `run_subagent`'s inline logic |
| `src/tui/channel.rs` | Modify | Add `BgOutcome` enum + `TuiEvent::BackgroundAgentDone` variant |
| `src/session/background_agent.rs` | Create | `BackgroundAgent`, `BgStatus`, `resolve_bg_model`, `parse_bg_args`, `spawn()` |
| `src/tui/app.rs` | Modify | `App.background_agents`, `App.next_bg_id` fields; handle `TuiEvent::BackgroundAgentDone` in `apply_event` |
| `src/tui/background_handler.rs` | Create | `/bg` and `/agents` (list/view/kill) command handlers |
| `src/tui/mod.rs` | Modify | `mod background_handler;` |
| `src/tui/turn_handler.rs` | Modify | Dispatch `/bg` and `/agents` |
| `src/tui/commands/mod.rs` | Modify | `SLASH_COMMANDS` entries + picker test |
| `src/session/commands/info.rs` | Modify | `/help` entries |
| `tests/e2e/test_background_agents.sh` | Create | CLI-mode fallthrough e2e test (mirrors `test_scheduler.sh`) |
| `FEATURES.md` | Modify | Feature registry entry |

---

## Task 1: Config fields — `is_background_agent`, `max_background_agents`

**Files:**
- Modify: `src/config/mod.rs`
- Modify: `src/config/tests.rs`

- [ ] **Step 1: Write the failing tests**

Add to `src/config/tests.rs`, right after the existing `model_routes_default_to_empty` test (after line 360):

```rust
#[test]
fn max_background_agents_parsed_from_toml() {
    let toml_str = r#"
        api_key = "test"
        max_background_agents = 3
    "#;
    let file: FileConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(file.max_background_agents, Some(3));
}

#[test]
fn max_background_agents_defaults_to_five_when_absent() {
    let toml_str = r#"api_key = "test""#;
    let file: FileConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(file.max_background_agents.unwrap_or(5), 5);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib config::tests::max_background_agents -- --nocapture`
Expected: FAIL — `no field \`max_background_agents\` on type \`FileConfig\``

- [ ] **Step 3: Add the fields**

In `src/config/mod.rs`, in the `Config` struct, add after the `is_subagent` field (around line 127-130):

```rust
    /// True when this config is for a sub-agent session. Suppresses startup banners
    /// and other output that would interleave with the parent session's output.
    pub is_subagent: bool,
    /// True when this sub-agent session was spawned by `/bg` (user-invoked,
    /// detached) rather than the model-invoked `spawn_agent` tool. Unlike plain
    /// sub-agents, background agents DO persist a `sessions` row — see
    /// `session::should_persist_session`.
    pub is_background_agent: bool,
    /// Nesting depth of this session: 0 = top-level, 1 = first sub-agent, etc.
    /// Incremented by run_subagent; never persisted to disk.
    pub spawn_depth: u8,
```

Add after the `model_routes` field (the last field in `Config`, around line 196):

```rust
    /// Per-task-type model overrides. Keys: "coding", "review", "explain", "search".
    /// Values: model slugs. Set in ~/.agent.toml as:
    /// [model_routes]
    /// coding = "codex/gpt-5.5"
    pub model_routes: HashMap<String, String>,
    /// Maximum number of `/bg` background agents allowed to run concurrently.
    /// Set in ~/.agent.toml as: max_background_agents = 5
    pub max_background_agents: usize,
```

In `FileConfig`, add after the `model_routes` field (around line 253):

```rust
    #[serde(default)]
    model_routes:    HashMap<String, String>,
    max_background_agents: Option<usize>,
```

In `Config::load()`, add near `let model_routes = file.model_routes;` (around line 416):

```rust
        let disabled_tools  = file.disabled_tools;
        let disabled_skills = file.disabled_skills;
        let model_routes    = file.model_routes;
        let max_background_agents = file.max_background_agents.unwrap_or(5);
```

And in the same function's final `Ok(Self { ... })` (around line 418-425), change:

```rust
        Ok(Self {
            permission_mode, sandbox, api_key, model, provider, base_url,
            output_format: OutputFormat::Text, agent_depth: 3, is_subagent: false, spawn_depth: 0,
            proxy, no_proxy, ca_bundle, tls_skip_verify, timeout_secs,
            budget: None, skill_paths, skill_token_budget, context_paths, allowed_paths, additional_dirs, disable_stream, skip_domain_prompt: false, tui_mode: false,
            tool_profile, provider_slug, all_providers,
            disabled_tools, disabled_skills, model_routes,
        })
```

to:

```rust
        Ok(Self {
            permission_mode, sandbox, api_key, model, provider, base_url,
            output_format: OutputFormat::Text, agent_depth: 3, is_subagent: false,
            is_background_agent: false, spawn_depth: 0,
            proxy, no_proxy, ca_bundle, tls_skip_verify, timeout_secs,
            budget: None, skill_paths, skill_token_budget, context_paths, allowed_paths, additional_dirs, disable_stream, skip_domain_prompt: false, tui_mode: false,
            tool_profile, provider_slug, all_providers,
            disabled_tools, disabled_skills, model_routes, max_background_agents,
        })
```

Finally, in the `#[cfg(test)] impl Default for Config` block (around line 563-597), add `is_background_agent: false,` right after `is_subagent: false,` and `max_background_agents: 5,` right after `model_routes: HashMap::new(),`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib config::tests:: -- --nocapture`
Expected: PASS (all `config::tests::*` tests, including the two new ones)

- [ ] **Step 5: Run diagnostics**

Run `get_diagnostics` on `src/config/mod.rs`. Expected: no errors (every other struct literal that builds a `Config` must now also set the two new fields — the compiler will point them out if any are missed; `Config` is only constructed via `Config::load()` and the test `Default` impl, both updated above, so this should be clean).

- [ ] **Step 6: Commit**

```bash
git add src/config/mod.rs src/config/tests.rs
git commit -m "$(cat <<'EOF'
feat(config): add is_background_agent and max_background_agents

Foundation for /bg background agents: a distinct flag from is_subagent
(so background-agent sessions persist to SQLite while model-invoked
sub-agents still don't) and a concurrency cap, both following the same
serde-default pattern as skill_token_budget/model_routes.
EOF
)"
```

---

## Task 2: Fix session persistence gate for background agents

**Files:**
- Modify: `src/session/mod.rs`
- Modify: `src/session/agent_loop_tests.rs`

- [ ] **Step 1: Write the failing test**

Add to `src/session/agent_loop_tests.rs`, after the existing `configured_provider_context_window_overrides_model_guess` test (around line 62):

```rust
#[test]
fn should_persist_session_true_for_top_level_and_background_agents() {
    let mut cfg = test_config();
    cfg.is_subagent = false;
    assert!(super::should_persist_session(&cfg), "top-level sessions must persist");

    cfg.is_subagent = true;
    cfg.is_background_agent = true;
    assert!(super::should_persist_session(&cfg), "/bg agents must persist their transcript");
}

#[test]
fn should_persist_session_false_for_model_invoked_subagents() {
    let mut cfg = test_config();
    cfg.is_subagent = true;
    cfg.is_background_agent = false;
    assert!(!super::should_persist_session(&cfg), "spawn_agent sub-agents must not bloat agent.db");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib session::agent_loop_tests::should_persist_session -- --nocapture`
Expected: FAIL — `cannot find function \`should_persist_session\` in module \`session\`` (or similar, since `test_config()` also doesn't set `is_background_agent` yet — that's fine, `Config::default()` already gives it `false` from Task 1)

- [ ] **Step 3: Add the helper and fix the gate**

In `src/session/mod.rs`, add right after `configured_context_limit` (after line 64):

```rust
/// Whether `Session::new` should persist a row in the `sessions` table.
/// Model-invoked sub-agents (`spawn_agent` tool, `is_subagent = true`,
/// `is_background_agent = false`) deliberately don't — they're short-lived
/// internal helpers and used to bloat `agent.db` with empty "(repl)" rows.
/// `/bg` background agents also set `is_subagent = true` (for banner
/// suppression and the destructive-command permission fix) but DO need
/// their transcript to survive, so they're the one exception.
pub fn should_persist_session(config: &Config) -> bool {
    !config.is_subagent || config.is_background_agent
}
```

Then change the `session_id` derivation inside `Session::new` (around line 225-229) from:

```rust
        let session_id = if config.is_subagent {
            0
        } else {
            store.save_session("(repl)", &config.model, &cwd_str)?
        };
```

to:

```rust
        let session_id = if should_persist_session(config) {
            store.save_session("(repl)", &config.model, &cwd_str)?
        } else {
            0
        };
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib session::agent_loop_tests::should_persist_session -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run the full existing session test suite to check for regressions**

Run: `cargo test --lib session:: -- --nocapture`
Expected: PASS (in particular, `spawn_agent`-related behavior is unchanged since `is_background_agent` defaults to `false` everywhere except the new `/bg` path added in Task 6)

- [ ] **Step 6: Commit**

```bash
git add src/session/mod.rs src/session/agent_loop_tests.rs
git commit -m "$(cat <<'EOF'
fix(session): persist background-agent sessions, not just top-level ones

Session::new previously zeroed session_id (skip persistence) for any
is_subagent=true config, which would have silently thrown away /bg
transcripts too. should_persist_session() carves out the is_background_agent
exception while leaving model-invoked spawn_agent sub-agents unpersisted.
EOF
)"
```

---

## Task 3: Fix destructive-command deadlock for unattended sub-agents

**Files:**
- Modify: `src/session/tools.rs`
- Modify: `src/session/agent_loop_tests.rs`

- [ ] **Step 1: Write the failing test**

Add to `src/session/agent_loop_tests.rs`, after the `one_tool_round_executes_tool_and_loops_back` test (after line 166):

```rust
#[tokio::test]
async fn destructive_shell_command_auto_denied_for_unattended_subagent() {
    // Regression test: before this fix, a destructive shell command under
    // is_subagent=true would be queued for an interactive prompt that can
    // never be answered (no controlling terminal), hanging the turn forever.
    let mock = MockClient::with_script(vec![
        MockClient::tool_call("call_1", "shell", json!({ "command": "rm -rf build/" })),
        MockClient::text("acknowledged"),
    ]);
    let session_client: Box<dyn LlmProvider> = Box::new(mock.clone());
    let mut session = Session::new_for_test(&test_config(), session_client).expect("session ctor");

    session.handle_user_turn("clean up build artifacts").await.expect("turn ran");

    assert_eq!(mock.call_count(), 2, "must not hang waiting for an interactive prompt");

    let tool_result = session.messages[2].content.iter().find_map(|b| {
        if let ContentBlock::ToolResult { content, tool_use_id } = b {
            Some((tool_use_id.as_str(), content.as_str()))
        } else { None }
    });
    let (tool_use_id, body) = tool_result.expect("tool_result block present");
    assert_eq!(tool_use_id, "call_1");
    assert!(body.starts_with("blocked:"), "expected auto-deny message, got: {body}");
    assert!(body.contains("recursive forced deletion"), "should surface the destructive reason: {body}");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib session::agent_loop_tests::destructive_shell_command -- --nocapture`
Expected: FAIL or HANG — with the current code, this destructive command is queued in `needs_prompt`, and in test mode (not TUI, no real tty) `prompt_batch` will attempt a CLI-mode `inquire` prompt against a non-interactive test process. If it hangs, cancel with Ctrl+C and note the hang as the expected failure mode; if it errors instead, that's also acceptable evidence of the bug. Either way, do not spend more than ~30s waiting — this confirms the bug and moves to Step 3.

- [ ] **Step 3: Fix `session/tools.rs`**

In `src/session/tools.rs`, inside `execute_tool_round`, replace the `Allow` arm (lines 81-111) from:

```rust
                crate::permission_manager::QuickDecision::Allow => {
                    let force_prompt = if name == "shell" {
                        if let Some(cmd) = input["command"].as_str() {
                            crate::tools::shell::destructive_pattern(cmd)
                                .map(|reason| format!("[DESTRUCTIVE: {}]\n         {}", reason, ctx))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some(destructive_ctx) = force_prompt {
                        needs_prompt.push((id.clone(), name.clone(), destructive_ctx, input.clone()));
                    } else {
                        match self.hooks.fire_pre_tool_use(name, input) {
                            crate::hooks::HookDecision::Block(reason) => {
                                audit::record(&format!("tool_blocked name={} reason={}", name, reason))?;
                                tool_results.push(ContentBlock::ToolResult {
                                    tool_use_id: id.clone(),
                                    content:     format!("Blocked by hook: {}", reason),
                                });
                            }
                            crate::hooks::HookDecision::Allow => {
                                approved.push(ApprovedCall {
                                    id: id.clone(), name: name.clone(),
                                    input: input.clone(), ctx,
                                });
                            }
                        }
                    }
                }
```

to:

```rust
                crate::permission_manager::QuickDecision::Allow => {
                    let destructive_reason = if name == "shell" {
                        input["command"].as_str().and_then(crate::tools::shell::destructive_pattern)
                    } else {
                        None
                    };
                    if let Some(reason) = destructive_reason {
                        if self.config.is_subagent {
                            // No controlling terminal to prompt (model-invoked spawn_agent
                            // or a /bg background agent) — auto-deny instead of queuing an
                            // interactive prompt that can never be answered.
                            audit::record(&format!("tool_denied name={} id={} reason=destructive_unattended", name, id))?;
                            tool_results.push(ContentBlock::ToolResult {
                                tool_use_id: id.clone(),
                                content: format!(
                                    "blocked: {reason} — destructive commands require interactive \
                                     approval, not available in an unattended sub-agent."
                                ),
                            });
                        } else {
                            let destructive_ctx = format!("[DESTRUCTIVE: {}]\n         {}", reason, ctx);
                            needs_prompt.push((id.clone(), name.clone(), destructive_ctx, input.clone()));
                        }
                    } else {
                        match self.hooks.fire_pre_tool_use(name, input) {
                            crate::hooks::HookDecision::Block(reason) => {
                                audit::record(&format!("tool_blocked name={} reason={}", name, reason))?;
                                tool_results.push(ContentBlock::ToolResult {
                                    tool_use_id: id.clone(),
                                    content:     format!("Blocked by hook: {}", reason),
                                });
                            }
                            crate::hooks::HookDecision::Allow => {
                                approved.push(ApprovedCall {
                                    id: id.clone(), name: name.clone(),
                                    input: input.clone(), ctx,
                                });
                            }
                        }
                    }
                }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib session::agent_loop_tests::destructive_shell_command -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run the full session test suite to check for regressions**

Run: `cargo test --lib session:: -- --nocapture`
Expected: PASS — non-subagent (top-level, interactive) destructive-command behavior is untouched; only the `is_subagent = true` path changes.

- [ ] **Step 6: Commit**

```bash
git add src/session/tools.rs src/session/agent_loop_tests.rs
git commit -m "$(cat <<'EOF'
fix(session): auto-deny destructive commands for unattended sub-agents

run_subagent forces PermissionMode::Auto because sub-agents have no
controlling terminal to prompt — but the destructive-pattern check
(rm -rf, git push --force, DROP TABLE, ...) queued an interactive prompt
regardless of mode, so a sub-agent hitting one would hang forever. Now
returns an immediate tool error instead, which the model can react to.
Prerequisite for /bg background agents, which have the same problem.
EOF
)"
```

---

## Task 4: Extract shared sub-agent result extraction

**Files:**
- Modify: `src/agent_core.rs`
- Modify: `src/session/agent_loop_tests.rs`

- [ ] **Step 1: Write the failing test**

Add to `src/session/agent_loop_tests.rs`, after the destructive-command test added in Task 3:

```rust
#[tokio::test]
async fn extract_result_captures_summary_turns_tools_and_files() {
    let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
    writeln!(tmp, "original content").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mock = MockClient::with_script(vec![
        MockClient::tool_call("call_1", "write_file", json!({ "path": path, "content": "new content\n" })),
        MockClient::text("updated the file"),
    ]);
    let session_client: Box<dyn LlmProvider> = Box::new(mock.clone());
    let mut session = Session::new_for_test(&test_config(), session_client).expect("session ctor");

    session.handle_user_turn("update the file").await.expect("turn ran");

    let result = crate::agent_core::extract_result(&session);
    assert_eq!(result.turns, 1);
    assert_eq!(result.tool_calls, 1);
    assert_eq!(result.summary, "updated the file");
    assert_eq!(result.files_changed, vec![path]);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib session::agent_loop_tests::extract_result -- --nocapture`
Expected: FAIL — `no function or associated item named \`extract_result\` found in module \`agent_core\``

- [ ] **Step 3: Extract the shared helper**

In `src/agent_core.rs`, add this new public struct and function right before `run_subagent` (before line 312):

```rust
/// Structured summary of a finished sub-agent session: what it did, how much
/// work it took, and which files it touched. Shared by `run_subagent`
/// (model-invoked, synchronous) and `session::background_agent` (user-invoked
/// via `/bg`, detached).
pub struct SubagentResult {
    pub summary: String,
    pub turns: usize,
    pub tool_calls: usize,
    pub files_changed: Vec<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Extract a [`SubagentResult`] from a finished sub-agent session: the final
/// assistant text, turn/tool-call counts, and files touched (via each tool's
/// `affected_path()`, the canonical source of truth rather than hardcoded
/// tool names).
pub fn extract_result(session: &Session) -> SubagentResult {
    let turns = session.turn_count;
    let total_tools: usize = session.messages.iter()
        .flat_map(|m| &m.content)
        .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
        .count();

    let mut files_changed: Vec<String> = session.messages.iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| {
            if let ContentBlock::ToolUse { name, input, .. } = b {
                session.tools.get(name)?.affected_path(input).map(str::to_string)
            } else {
                None
            }
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    files_changed.sort_unstable();

    let summary = session.messages.iter().rev()
        .find(|m| m.role == "assistant")
        .and_then(|m| m.content.iter().find_map(|b| {
            if let ContentBlock::Text { text } = b { Some(text.clone()) } else { None }
        }))
        .unwrap_or_default();

    SubagentResult {
        summary,
        turns,
        tool_calls: total_tools,
        files_changed,
        input_tokens: session.session_usage.input_tokens,
        output_tokens: session.session_usage.output_tokens,
    }
}
```

Then replace the body of `run_subagent` from `let turns = session.turn_count;` through the `let result = serde_json::json!({...});` block (lines 341-377) with:

```rust
    let r = extract_result(&session);

    let result = serde_json::json!({
        "summary": r.summary,
        "turns": r.turns,
        "tool_calls": r.tool_calls,
        "files_changed": r.files_changed,
        "input_tokens": r.input_tokens,
        "output_tokens": r.output_tokens,
    });

    println!(
        "  {} sub-agent [L{}] done  {} turn(s)  {} tool(s){}",
        "◈".bright_cyan(),
        depth_level,
        r.turns.to_string().cyan(),
        r.tool_calls.to_string().cyan(),
        if r.files_changed.is_empty() {
            String::new()
        } else {
            format!("  changed: {}", r.files_changed.join(", ").truecolor(130, 125, 150))
        },
    );
    audit::record(&format!("subagent_end depth={} turns={} tools={}", depth_level, r.turns, r.tool_calls))?;

    Ok(result.to_string())
}
```

(This is a behavior-preserving refactor — `run_subagent`'s printed output and returned JSON are unchanged; the logic just now lives in `extract_result`.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib session::agent_loop_tests::extract_result -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run diagnostics and the broader test suite**

Run `get_diagnostics` on `src/agent_core.rs`. Expected: no errors.
Run: `cargo test --lib`
Expected: PASS (full existing suite — this task is a pure refactor plus one new test)

- [ ] **Step 6: Commit**

```bash
git add src/agent_core.rs src/session/agent_loop_tests.rs
git commit -m "$(cat <<'EOF'
refactor(agent_core): extract SubagentResult/extract_result from run_subagent

Behavior-preserving: pulls the inline summary/turns/tool_calls/files_changed
extraction into a standalone, unit-testable function so the upcoming /bg
background-agent path can reuse it instead of duplicating the logic.
EOF
)"
```

---

## Task 5: `TuiEvent::BackgroundAgentDone`

**Files:**
- Modify: `src/tui/channel.rs`

- [ ] **Step 1: Add the outcome enum and event variant**

In `src/tui/channel.rs`, add right before the `TuiEvent` enum (before line 75):

```rust
/// Outcome of a finished background agent (`/bg`), reported by the detached
/// tokio task back to the TUI event loop. `Killed` isn't represented here —
/// `/agents kill` sets that status synchronously without going through this event.
#[derive(Debug, Clone)]
pub enum BgOutcome {
    Done { summary: String, files_changed: Vec<String>, turns: usize, tool_calls: usize },
    Failed(String),
}
```

Add this variant to `TuiEvent`, right after `ScheduledFire` (after line 95):

```rust
    /// A scheduled job fired — submit `goal` as the next user turn.
    /// `name` is used for display only (shown as the bubble label).
    ScheduledFire { name: String, goal: String },
    /// A `/bg` background agent finished (or failed). `elapsed_secs` is
    /// wall-clock time since it was spawned.
    BackgroundAgentDone { id: String, goal: String, model: String, elapsed_secs: u64, outcome: BgOutcome },
```

- [ ] **Step 2: Run diagnostics**

Run `get_diagnostics` on `src/tui/channel.rs`. Expected: no errors — this variant isn't matched anywhere yet, which is fine since `apply_event`'s `match` isn't exhaustive-checked against `TuiEvent` in a way that would fail to compile from this file alone (the match lives in `app.rs`, handled in Task 7).

Run: `cargo build --lib 2>&1 | grep -A3 "non-exhaustive\|error"`
Expected: a `non-exhaustive patterns` error pointing at `App::apply_event`'s match in `src/tui/app.rs` — this confirms the new variant is wired into the type and the compiler is telling us exactly where Task 7 needs to add a case. Do not fix it here; that's Task 7.

- [ ] **Step 3: Commit**

```bash
git add src/tui/channel.rs
git commit -m "$(cat <<'EOF'
feat(tui): add TuiEvent::BackgroundAgentDone

Wires the event type only; App::apply_event handling lands in the next
commit alongside the background_agent module that sends it.
EOF
)"
```

(Committing a deliberately-not-yet-exhaustive match is fine here since this repo compiles the whole workspace per commit via CI/hooks — if your local pre-commit hook runs `cargo build` and blocks on the non-exhaustive match, fold this step into Task 7's commit instead and skip committing here.)

---

## Task 6: `src/session/background_agent.rs` — core types and pure logic

**Files:**
- Create: `src/session/background_agent.rs`
- Modify: `src/session/mod.rs`

- [ ] **Step 1: Write the failing tests**

Create `src/session/background_agent.rs` with just the test module first:

```rust
//! Background agents: `/bg` spawns an independent sub-session as a detached
//! tokio task, tracked in `App.background_agents` so `/agents` can list,
//! view, and kill them. Reuses the same `Session` + `extract_result` plumbing
//! as the model-invoked `spawn_agent` tool (`agent_core::run_subagent`) — the
//! difference is this path is user-invoked and non-blocking.

use chrono::{DateTime, Local};

use crate::config::Config;
use crate::session::task_classifier;
use crate::tui::channel::BgOutcome;

pub struct BackgroundAgent {
    pub id: String,
    pub goal: String,
    pub model: String,
    pub status: BgStatus,
    pub started_at: DateTime<Local>,
    pub handle: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone)]
pub enum BgStatus {
    Running,
    Done { summary: String, files_changed: Vec<String>, turns: usize, tool_calls: usize },
    Failed(String),
    Killed,
}

impl From<BgOutcome> for BgStatus {
    fn from(o: BgOutcome) -> Self {
        match o {
            BgOutcome::Done { summary, files_changed, turns, tool_calls } =>
                BgStatus::Done { summary, files_changed, turns, tool_calls },
            BgOutcome::Failed(e) => BgStatus::Failed(e),
        }
    }
}

/// Resolve the model for a `/bg` task: an explicit `--model` always wins;
/// otherwise falls back to the same `task_classifier` + `model_routes` lookup
/// `session::routing::route_for_turn` uses for in-session turn routing; falls
/// back again to the caller's current default model.
pub fn resolve_bg_model(goal: &str, explicit: Option<&str>, config: &Config) -> String {
    if let Some(m) = explicit {
        return m.to_string();
    }
    let task_type = task_classifier::classify(goal);
    config.model_routes.get(task_type.as_str())
        .cloned()
        .unwrap_or_else(|| config.model.clone())
}

/// Parse `/bg <goal> [--model <slug>]` into `(goal, explicit_model)`.
pub fn parse_bg_args(arg: &str) -> (String, Option<String>) {
    if let Some(idx) = arg.find("--model ") {
        let goal  = arg[..idx].trim().to_string();
        let model = arg[idx + "--model ".len()..].trim().to_string();
        (goal, if model.is_empty() { None } else { Some(model) })
    } else {
        (arg.trim().to_string(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config::default()
    }

    #[test]
    fn resolve_bg_model_explicit_wins() {
        let config = test_config();
        assert_eq!(
            resolve_bg_model("fix the bug", Some("codex/gpt-5.5"), &config),
            "codex/gpt-5.5"
        );
    }

    #[test]
    fn resolve_bg_model_falls_back_to_model_routes() {
        let mut config = test_config();
        config.model_routes.insert("coding".to_string(), "codex/gpt-5.5".to_string());
        assert_eq!(
            resolve_bg_model("fix the bug in auth.rs", None, &config),
            "codex/gpt-5.5"
        );
    }

    #[test]
    fn resolve_bg_model_falls_back_to_default_model() {
        let config = test_config(); // model_routes empty, model = "test-model"
        assert_eq!(resolve_bg_model("hi there", None, &config), "test-model");
    }

    #[test]
    fn parse_bg_args_splits_goal_and_model() {
        let (goal, model) = parse_bg_args("refactor the auth middleware --model codex/gpt-5.5");
        assert_eq!(goal, "refactor the auth middleware");
        assert_eq!(model, Some("codex/gpt-5.5".to_string()));
    }

    #[test]
    fn parse_bg_args_without_model_flag() {
        let (goal, model) = parse_bg_args("write tests for the parser");
        assert_eq!(goal, "write tests for the parser");
        assert_eq!(model, None);
    }
}
```

- [ ] **Step 2: Register the module and run to verify it fails**

In `src/session/mod.rs`, add `pub mod background_agent;` next to the other `pub mod` declarations (after line 4, `pub mod task_classifier;`).

Run: `cargo test --lib session::background_agent:: -- --nocapture`
Expected: FAIL first with a compile error if `crate::tui::channel::BgOutcome` isn't visible yet (it was added in Task 5, so this should actually compile) — if it does compile, the tests should PASS immediately since all the logic is already written in Step 1. This task is unusual in that the "failing test" step and "implementation" step are the same edit; that's acceptable for pure, self-contained logic like this.

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --lib session::background_agent:: -- --nocapture`
Expected: PASS (5 tests)

- [ ] **Step 4: Run diagnostics**

Run `get_diagnostics` on `src/session/background_agent.rs` and `src/session/mod.rs`. Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/session/background_agent.rs src/session/mod.rs
git commit -m "$(cat <<'EOF'
feat(session): background_agent module — types, model resolution, arg parsing

BackgroundAgent/BgStatus registry types plus resolve_bg_model (reuses the
task_classifier + model_routes lookup already used for in-session turn
routing) and parse_bg_args for "/bg <goal> [--model <slug>]". The actual
spawn() that runs a task on a detached tokio task lands in the next commit.
EOF
)"
```

---

## Task 7: `spawn()` — the detached background task

**Files:**
- Modify: `src/session/background_agent.rs`

- [ ] **Step 1: Add `spawn()`**

There's no meaningful way to unit-test `spawn()` itself without either mocking `Session::new` (which does real I/O and isn't designed for that — see `Session::new_for_test`'s doc comment) or spawning a real, slow, network-dependent task. This matches the existing precedent in this codebase: `session/scheduler.rs`'s `spawn_job`-equivalent (`tui/schedule_handler.rs::spawn_job`) also has no direct unit test — only the pure parsing/formatting functions around it are tested. `spawn()`'s correctness is instead covered by: the unit tests already covering `resolve_bg_model`/`parse_bg_args`/`extract_result`/`should_persist_session` (the pieces it composes), the cap-check unit test in Task 9, and manual verification in Task 12.

Add to `src/session/background_agent.rs`, after `parse_bg_args` and before the `#[cfg(test)]` block:

```rust
/// Spawn a background agent: builds an independent `Config`/`Session` for
/// `goal`, runs it to completion on a detached tokio task, and reports the
/// outcome via `TuiEvent::BackgroundAgentDone`. Returns the registry entry to
/// push into `App.background_agents` immediately — its `status` starts
/// `Running` and is updated later when the event arrives.
pub fn spawn(id: String, goal: String, explicit_model: Option<String>, config: &Config) -> BackgroundAgent {
    let model = resolve_bg_model(&goal, explicit_model.as_deref(), config);

    let mut sub_config = config.clone();
    sub_config.model               = model.clone();
    sub_config.is_subagent         = true;
    sub_config.is_background_agent = true;
    sub_config.agent_depth         = config.agent_depth.saturating_sub(1);
    sub_config.spawn_depth         = config.spawn_depth.saturating_add(1);
    sub_config.permission_mode     = crate::config::PermissionMode::Auto;

    let started_at = Local::now();
    let task_id    = id.clone();
    let task_goal  = goal.clone();
    let task_model = model.clone();

    let handle = tokio::spawn(async move {
        let run: anyhow::Result<crate::agent_core::SubagentResult> = async {
            let mut session = crate::session::Session::new(&sub_config).await?;
            session.handle_user_turn(&task_goal).await?;
            Ok(crate::agent_core::extract_result(&session))
        }.await;

        let outcome = match run {
            Ok(r) => BgOutcome::Done {
                summary:       r.summary,
                files_changed: r.files_changed,
                turns:         r.turns,
                tool_calls:    r.tool_calls,
            },
            Err(e) => BgOutcome::Failed(e.to_string()),
        };

        let elapsed_secs = (Local::now() - started_at).num_seconds().max(0) as u64;
        crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::BackgroundAgentDone {
            id: task_id,
            goal: task_goal,
            model: task_model,
            elapsed_secs,
            outcome,
        });
    });

    BackgroundAgent { id, goal, model, status: BgStatus::Running, started_at, handle }
}
```

- [ ] **Step 2: Run diagnostics**

Run `get_diagnostics` on `src/session/background_agent.rs`. Expected: no errors.

- [ ] **Step 3: Run the full background_agent test module to confirm no regressions**

Run: `cargo test --lib session::background_agent:: -- --nocapture`
Expected: PASS (same 5 tests as Task 6 — `spawn` itself isn't called by any test yet)

- [ ] **Step 4: Commit**

```bash
git add src/session/background_agent.rs
git commit -m "$(cat <<'EOF'
feat(session): background_agent::spawn — detached tokio task per /bg goal

Builds an independent Config (own model, is_subagent+is_background_agent
set, Auto permission mode, propagated agent_depth/spawn_depth so the
existing recursion cap still applies), runs it via Session::new +
handle_user_turn on tokio::spawn, and reports completion over the existing
TuiEvent channel. Not directly unit-tested (matches the existing precedent
for scheduler's spawn_job) — covered by its composed pure functions plus
manual verification.
EOF
)"
```

---

## Task 8: Wire `TuiEvent::BackgroundAgentDone` into `App`

**Files:**
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Write the failing test**

Add a new test module at the end of `src/tui/app.rs`:

```rust
#[cfg(test)]
mod background_agent_tests {
    use super::*;
    use crate::session::background_agent::{BackgroundAgent, BgStatus};
    use crate::tui::channel::{BgOutcome, TuiEvent};

    fn dummy_agent(id: &str) -> BackgroundAgent {
        BackgroundAgent {
            id: id.to_string(),
            goal: "refactor auth".to_string(),
            model: "codex/gpt-5.5".to_string(),
            status: BgStatus::Running,
            started_at: chrono::Local::now(),
            handle: tokio::spawn(async {}),
        }
    }

    #[tokio::test]
    async fn background_agent_done_updates_status_and_pushes_notice() {
        let mut app = App::new("test-model", "main");
        app.background_agents.push(dummy_agent("1"));

        app.apply_event(TuiEvent::BackgroundAgentDone {
            id: "1".to_string(),
            goal: "refactor auth".to_string(),
            model: "codex/gpt-5.5".to_string(),
            elapsed_secs: 42,
            outcome: BgOutcome::Done {
                summary: "done".to_string(),
                files_changed: vec!["src/auth.rs".to_string()],
                turns: 3,
                tool_calls: 5,
            },
        });

        let agent = app.background_agents.iter().find(|a| a.id == "1").expect("agent still present");
        assert!(matches!(agent.status, BgStatus::Done { turns: 3, tool_calls: 5, .. }));

        let last = app.messages.last().expect("a notice was pushed");
        let UiBlock::Text(text) = &last.blocks[0] else { panic!("expected text block") };
        assert!(text.contains('✓'), "success notice should use a checkmark: {text}");
        assert!(text.contains("refactor auth"));
    }

    #[tokio::test]
    async fn background_agent_failed_pushes_failure_notice() {
        let mut app = App::new("test-model", "main");
        app.background_agents.push(dummy_agent("2"));

        app.apply_event(TuiEvent::BackgroundAgentDone {
            id: "2".to_string(),
            goal: "write tests".to_string(),
            model: "claude-sonnet-5".to_string(),
            elapsed_secs: 5,
            outcome: BgOutcome::Failed("provider timeout".to_string()),
        });

        let agent = app.background_agents.iter().find(|a| a.id == "2").expect("agent still present");
        assert!(matches!(&agent.status, BgStatus::Failed(e) if e == "provider timeout"));

        let last = app.messages.last().expect("a notice was pushed");
        let UiBlock::Text(text) = &last.blocks[0] else { panic!("expected text block") };
        assert!(text.contains('✗'), "failure notice should use a cross mark: {text}");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib tui::app::background_agent_tests:: -- --nocapture`
Expected: FAIL — `no field \`background_agents\` on type \`App\`` (and a non-exhaustive-match compile error from Task 5 if you haven't hit it yet)

- [ ] **Step 3: Add the fields and event handling**

In `src/tui/app.rs`, add to the `App` struct after `scheduled_queue` (after line 405):

```rust
    /// Goals queued by the scheduler while a turn was in progress.
    /// Drained one-at-a-time after each turn completes.
    pub scheduled_queue: std::collections::VecDeque<(String, String)>,
    /// Active `/bg` background agents this TUI process has spawned. Each holds
    /// a Tokio JoinHandle; abort to cancel (`/agents kill`). Scoped to this
    /// process's lifetime — the underlying transcript still persists to SQLite.
    pub background_agents: Vec<crate::session::background_agent::BackgroundAgent>,
    /// Monotonic counter for background-agent IDs ("1", "2", ...).
    pub next_bg_id: u32,
```

In `App::new`, add after `scheduled_queue: std::collections::VecDeque::new(),` (after line 494):

```rust
            scheduled_jobs:  Vec::new(),
            scheduled_queue: std::collections::VecDeque::new(),
            background_agents: Vec::new(),
            next_bg_id: 0,
        }
    }
```

In `apply_event`, add a new match arm right after the `TuiEvent::ScheduledFire { .. }` arm closes (after the block ending around line 630 — locate the closing `}` of that arm):

```rust
            TuiEvent::BackgroundAgentDone { id, goal, model, elapsed_secs, outcome } => {
                let failed = matches!(&outcome, crate::tui::channel::BgOutcome::Failed(_));
                if let Some(agent) = self.background_agents.iter_mut().find(|a| a.id == id) {
                    agent.status = outcome.into();
                }
                let elapsed = {
                    let s = crate::session::scheduler::format_interval_secs(elapsed_secs);
                    if s.is_empty() { "0s".to_string() } else { s }
                };
                let mark = if failed { "✗" } else { "✓" };
                let verb = if failed { "failed" } else { "finished" };
                self.messages.push(UiMessage {
                    role: MsgRole::Assistant,
                    blocks: vec![UiBlock::Text(format!(
                        "{mark} Background agent {id} {verb}: \"{goal}\" ({model}, {elapsed})\n   /agents view {id} for details"
                    ))],
                });
                self.auto_scroll = true;
            }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib tui::app::background_agent_tests:: -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run diagnostics and the broader tui test suite**

Run `get_diagnostics` on `src/tui/app.rs`. Expected: no errors — the `apply_event` match is now exhaustive again.
Run: `cargo test --lib tui:: -- --nocapture`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/tui/app.rs
git commit -m "$(cat <<'EOF'
feat(tui): App.background_agents registry + BackgroundAgentDone handling

App tracks spawned /bg agents (mirrors the existing scheduled_jobs
pattern). apply_event updates the matching entry's status and appends a
one-line completion/failure notice to the transcript, without touching
the active conversation otherwise.
EOF
)"
```

---

## Task 9: `/bg` and `/agents` command handlers

**Files:**
- Create: `src/tui/background_handler.rs`
- Modify: `src/tui/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `src/tui/background_handler.rs` with the handler functions and a focused test module covering the cap check and the view/kill "not found" paths (the parts that are pure/deterministic without needing `spawn()`'s real I/O):

```rust
//! Handlers for the /bg and /agents TUI slash commands.

use anyhow::Result;

use super::app::{App, MsgRole, UiBlock, UiMessage};
use crate::config::Config;
use crate::session::background_agent::{self, BgStatus};

fn notice(app: &mut App, text: String) {
    app.messages.push(UiMessage { role: MsgRole::Assistant, blocks: vec![UiBlock::Text(text)] });
    app.auto_scroll = true;
}

fn elapsed_label(started_at: chrono::DateTime<chrono::Local>) -> String {
    let secs = (chrono::Local::now() - started_at).num_seconds().max(0) as u64;
    if secs == 0 {
        "0s".to_string()
    } else {
        crate::session::scheduler::format_interval_secs(secs)
    }
}

/// Handle `/bg <goal> [--model <slug>]`. Returns `Ok(false)` always (no exit needed).
pub(super) fn handle_bg(app: &mut App, cmd: &str, config: &Config) -> Result<bool> {
    let arg = cmd.strip_prefix("/bg").unwrap_or("").trim();
    if arg.is_empty() {
        notice(app, "Usage: /bg <goal> [--model <slug>]".to_string());
        return Ok(false);
    }

    let running = app.background_agents.iter()
        .filter(|a| matches!(a.status, BgStatus::Running))
        .count();
    if running >= config.max_background_agents {
        notice(app, format!(
            "✗ Cannot start: {running} background agents already running (max_background_agents = {})",
            config.max_background_agents
        ));
        return Ok(false);
    }

    let (goal, explicit_model) = background_agent::parse_bg_args(arg);
    if goal.is_empty() {
        notice(app, "Usage: /bg <goal> [--model <slug>]".to_string());
        return Ok(false);
    }

    app.next_bg_id += 1;
    let id = app.next_bg_id.to_string();
    let agent = background_agent::spawn(id.clone(), goal, explicit_model, config);
    let model = agent.model.clone();
    app.background_agents.push(agent);

    notice(app, format!("Started agent {id} ({model}) — /agents to check."));
    Ok(false)
}

/// Handle `/agents`, `/agents view <id>`, `/agents kill <id>`. Returns `Ok(false)` always.
pub(super) fn handle_agents(app: &mut App, cmd: &str) -> Result<bool> {
    let arg = cmd.strip_prefix("/agents").unwrap_or("").trim();

    if let Some(id) = arg.strip_prefix("view ") {
        let id = id.trim();
        let Some(agent) = app.background_agents.iter().find(|a| a.id == id) else {
            notice(app, format!("No background agent '{id}'. Use /agents to see active ones."));
            return Ok(false);
        };
        let text = match &agent.status {
            BgStatus::Running => format!(
                "{id} — running — {} elapsed — {}",
                elapsed_label(agent.started_at), agent.goal
            ),
            BgStatus::Done { summary, files_changed, turns, tool_calls } => format!(
                "{id} — done — {turns} turn(s), {tool_calls} tool call(s){}\n{summary}",
                if files_changed.is_empty() {
                    String::new()
                } else {
                    format!("\nfiles changed: {}", files_changed.join(", "))
                }
            ),
            BgStatus::Failed(err) => format!("{id} — failed — {err}"),
            BgStatus::Killed        => format!("{id} — killed — {}", agent.goal),
        };
        notice(app, text);
        return Ok(false);
    }

    if let Some(id) = arg.strip_prefix("kill ") {
        let id = id.trim();
        let Some(pos) = app.background_agents.iter().position(|a| a.id == id) else {
            notice(app, format!("No background agent '{id}'. Use /agents to see active ones."));
            return Ok(false);
        };
        if !matches!(app.background_agents[pos].status, BgStatus::Running) {
            notice(app, format!("Agent '{id}' is not running (status already final)."));
            return Ok(false);
        }
        app.background_agents[pos].handle.abort();
        app.background_agents[pos].status = BgStatus::Killed;
        notice(app, format!("Killed agent {id}."));
        return Ok(false);
    }

    if app.background_agents.is_empty() {
        notice(app, "No background agents. Usage: /bg <goal> [--model <slug>]".to_string());
        return Ok(false);
    }

    let mut lines = vec!["ID    MODEL               STATUS    ELAPSED  GOAL".to_string()];
    for a in &app.background_agents {
        let status = match &a.status {
            BgStatus::Running     => "running",
            BgStatus::Done { .. } => "done",
            BgStatus::Failed(_)   => "failed",
            BgStatus::Killed      => "killed",
        };
        lines.push(format!(
            "{:<5} {:<19} {:<9} {:<8} {}",
            a.id, a.model, status, elapsed_label(a.started_at), a.goal
        ));
    }
    notice(app, lines.join("\n"));
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_agent(id: &str, status: BgStatus) -> background_agent::BackgroundAgent {
        background_agent::BackgroundAgent {
            id: id.to_string(),
            goal: "existing task".to_string(),
            model: "test-model".to_string(),
            status,
            started_at: chrono::Local::now(),
            handle: tokio::spawn(async {}),
        }
    }

    fn last_notice(app: &App) -> String {
        let UiBlock::Text(text) = &app.messages.last().unwrap().blocks[0] else {
            panic!("expected text block");
        };
        text.clone()
    }

    #[tokio::test]
    async fn handle_bg_rejects_when_at_cap() {
        let mut app = App::new("test-model", "main");
        let mut config = Config::default();
        config.max_background_agents = 1;
        app.background_agents.push(dummy_agent("1", BgStatus::Running));

        handle_bg(&mut app, "/bg another task", &config).unwrap();

        assert_eq!(app.background_agents.len(), 1, "should not have spawned a second agent");
        assert!(last_notice(&app).contains("Cannot start"));
    }

    #[tokio::test]
    async fn handle_bg_allows_below_cap() {
        let mut app = App::new("test-model", "main");
        let mut config = Config::default();
        config.max_background_agents = 5;

        handle_bg(&mut app, "/bg write tests for the parser", &config).unwrap();

        assert_eq!(app.background_agents.len(), 1);
        assert!(last_notice(&app).contains("Started agent 1"));
    }

    #[tokio::test]
    async fn handle_agents_view_reports_unknown_id() {
        let mut app = App::new("test-model", "main");
        handle_agents(&mut app, "/agents view 99").unwrap();
        assert!(last_notice(&app).contains("No background agent '99'"));
    }

    #[tokio::test]
    async fn handle_agents_kill_aborts_running_agent() {
        let mut app = App::new("test-model", "main");
        app.background_agents.push(dummy_agent("1", BgStatus::Running));

        handle_agents(&mut app, "/agents kill 1").unwrap();

        assert!(matches!(app.background_agents[0].status, BgStatus::Killed));
        assert!(last_notice(&app).contains("Killed agent 1"));
    }

    #[tokio::test]
    async fn handle_agents_kill_rejects_already_finished_agent() {
        let mut app = App::new("test-model", "main");
        app.background_agents.push(dummy_agent("1", BgStatus::Done {
            summary: "done".to_string(), files_changed: vec![], turns: 1, tool_calls: 1,
        }));

        handle_agents(&mut app, "/agents kill 1").unwrap();

        assert!(last_notice(&app).contains("not running"));
    }
}
```

- [ ] **Step 2: Register the module and run to verify it fails**

In `src/tui/mod.rs`, add `mod background_handler;` next to `mod schedule_handler;` (after line 19).

Run: `cargo test --lib tui::background_handler:: -- --nocapture`
Expected: This should mostly compile and pass immediately since the implementation is written alongside the tests in Step 1 (same reasoning as Task 6 — this is pure/deterministic logic composed from already-implemented pieces). If `Config::default()` isn't `pub` for test use outside `crate::config`, adjust the import; it already derives `Default` under `#[cfg(test)]` per Task 1.

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --lib tui::background_handler:: -- --nocapture`
Expected: PASS (5 tests)

- [ ] **Step 4: Run diagnostics**

Run `get_diagnostics` on `src/tui/background_handler.rs` and `src/tui/mod.rs`. Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/tui/background_handler.rs src/tui/mod.rs
git commit -m "$(cat <<'EOF'
feat(tui): /bg and /agents (list/view/kill) command handlers

Mirrors the existing /schedule handler style: synchronous fns operating
directly on App, cap-enforced via config.max_background_agents. Tested:
cap rejection/acceptance, view of an unknown id, kill of a running vs
already-finished agent.
EOF
)"
```

---

## Task 10: Wire `/bg` and `/agents` dispatch + picker + help

**Files:**
- Modify: `src/tui/turn_handler.rs`
- Modify: `src/tui/commands/mod.rs`
- Modify: `src/session/commands/info.rs`

- [ ] **Step 1: Write the failing test**

Add to the test module in `src/tui/commands/mod.rs` (near `image_commands_are_listed`, after line 402):

```rust
    #[test]
    fn background_agent_commands_are_listed() {
        let cmds = filter_commands("/bg", &[]);
        assert!(cmds.iter().any(|(c, _)| c == "/bg"));

        let cmds = filter_commands("/age", &[]);
        assert!(cmds.iter().any(|(c, _)| c == "/agents"));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib tui::commands::tests::background_agent_commands_are_listed -- --nocapture`
Expected: FAIL — `/bg` and `/agents` not present in `SLASH_COMMANDS`

- [ ] **Step 3: Register in the picker**

In `src/tui/commands/mod.rs`, add to `SLASH_COMMANDS` right after `("/unschedule", ...)` (after line 54):

```rust
    ("/schedule",          "schedule a goal in the TUI"),
    ("/unschedule",        "cancel a scheduled TUI job"),
    ("/bg",                "run a goal in the background (optional --model)"),
    ("/agents",            "list background agents"),
```

- [ ] **Step 4: Wire dispatch**

In `src/tui/turn_handler.rs`, add right after the `/unschedule` handling (after line 146):

```rust
    if cmd.starts_with("/unschedule") {
        return super::schedule_handler::handle_unschedule(app, cmd);
    }

    // /bg and /agents — detached background sub-agents (independent session, own model).
    if cmd == "/bg" || cmd.starts_with("/bg ") {
        return super::background_handler::handle_bg(app, cmd, config);
    }
    if cmd == "/agents" || cmd.starts_with("/agents ") {
        return super::background_handler::handle_agents(app, cmd);
    }
```

- [ ] **Step 5: Add help text**

In `src/session/commands/info.rs`, add a new group right after the `"scheduler (TUI only)"` group (after line 71):

```rust
            ("scheduler (TUI only)", &[
                ("/schedule <interval> <goal>", "run goal every interval (30m, 1h, 17:30)"),
                ("/schedule list",              "show active scheduled jobs"),
                ("/unschedule <name>",          "cancel a scheduled job"),
            ]),
            ("background agents (TUI only)", &[
                ("/bg <goal> [--model <slug>]", "run a goal in the background, optionally on a specific model"),
                ("/agents",                     "list background agents and their status"),
                ("/agents view <id>",           "show a background agent's progress or result"),
                ("/agents kill <id>",           "cancel a running background agent"),
            ]),
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --lib tui::commands::tests::background_agent_commands_are_listed -- --nocapture`
Expected: PASS

Run: `cargo test --lib tui::commands:: -- --nocapture`
Expected: PASS (including `slash_alone_returns_all_commands`, which will now count the two new entries automatically)

- [ ] **Step 7: Run diagnostics**

Run `get_diagnostics` on all three modified files. Expected: no errors.

- [ ] **Step 8: Commit**

```bash
git add src/tui/turn_handler.rs src/tui/commands/mod.rs src/session/commands/info.rs
git commit -m "$(cat <<'EOF'
feat(tui): register /bg and /agents in dispatch, picker, and /help

Follows the project's slash-command checklist (src/default_skills/slash-commands.md):
handler wiring, picker registration, and help text land together so the
commands are actually discoverable, not just implemented.
EOF
)"
```

---

## Task 11: E2E CLI-mode fallthrough test

**Files:**
- Create: `tests/e2e/test_background_agents.sh`

- [ ] **Step 1: Write the test**

`/bg` and `/agents` are TUI-only, exactly like `/schedule`/`/unschedule` — there is no CLI-mode (`--cli`) dispatch for them. Mirror `tests/e2e/test_scheduler.sh` exactly: assert they fall through to "unknown command" in CLI mode rather than crashing or being silently accepted.

Create `tests/e2e/test_background_agents.sh`:

```bash
#!/usr/bin/env bash
# T31 — Background agents: /bg and /agents are TUI-only commands.
# In CLI mode they fall through to Unknown command — test that behaviour is stable.
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

TMP=$(make_project)
trap "rm -rf $TMP" EXIT

info "T31a: /bg shows unknown command in CLI mode"
OUT=$(cd "$TMP" && printf '/bg refactor the auth middleware\n/exit\n' | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true
if echo "$OUT" | grep -qi "unknown command /bg"; then
    pass "T31a /bg shows unknown command in CLI mode"
else
    fail "T31a /bg shows unknown command in CLI mode" "$(echo "$OUT" | tail -10)"
fi

info "T31b: /agents shows unknown command in CLI mode"
OUT=$(cd "$TMP" && printf '/agents\n/exit\n' | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true
if echo "$OUT" | grep -qi "unknown command /agents"; then
    pass "T31b /agents shows unknown command in CLI mode"
else
    fail "T31b /agents shows unknown command in CLI mode" "$(echo "$OUT" | tail -10)"
fi

info "T31c: /agents kill <id> shows unknown command in CLI mode"
OUT=$(cd "$TMP" && printf '/agents kill 1\n/exit\n' | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true
if echo "$OUT" | grep -qi "unknown command /agents"; then
    pass "T31c /agents kill shows unknown command in CLI mode"
else
    fail "T31c /agents kill shows unknown command in CLI mode" "$(echo "$OUT" | tail -10)"
fi

summary
```

- [ ] **Step 2: Make it executable and run it**

Run: `chmod +x tests/e2e/test_background_agents.sh && cargo build --release && ./tests/e2e/test_background_agents.sh`
Expected: all three cases PASS (this requires a release build so `$ZAP` resolves to a working binary — follow the same build step `test_scheduler.sh` relies on via `helpers.sh`)

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/test_background_agents.sh
git commit -m "$(cat <<'EOF'
test(e2e): T31 — /bg and /agents CLI-mode fallthrough

Mirrors test_scheduler.sh: /bg and /agents are TUI-only, so this asserts
they cleanly fall through to "unknown command" in --cli mode rather than
being silently accepted or crashing.
EOF
)"
```

---

## Task 12: Manual verification + FEATURES.md

**Files:**
- Modify: `FEATURES.md`

- [ ] **Step 1: Manual smoke test in the real TUI**

Run: `cargo run --release` (or your usual `zap` dev invocation) in a real project directory with at least one working provider configured.

Try, in order:
1. `/bg say hi and stop` — expect `Started agent 1 (...) — /agents to check.`
2. `/agents` — expect a table showing agent `1` as `running`.
3. Wait a few seconds, then `/agents` again — expect it to flip to `done` once the completion notice (`✓ Background agent 1 finished: ...`) appears in the transcript on its own.
4. `/agents view 1` — expect the captured summary/turns/tool-call count.
5. `/bg refactor something --model <a-second-configured-model-slug>` then immediately `/bg write a haiku` (no `--model`) — expect both to show as `running` concurrently in `/agents`, with different `model` columns, and your main conversation still responsive to normal typing while they run.
6. `/agents kill <id>` on one of the still-running ones — expect it to flip to `killed` and stop appearing as `running` in the cap count.
7. Push `max_background_agents` down to 1 in `~/.agent.toml` (or `.agent.toml`), restart, spawn one, then try a second — expect the `✗ Cannot start` rejection message.
8. Ask a spawned agent's goal to include something destructive (e.g. `/bg run rm -rf /tmp/some-throwaway-dir --model <slug>` against a scratch directory you don't mind risking) — expect the background agent's eventual `done`/`failed` result to reflect a blocked tool call rather than the whole thing hanging forever.

Do not proceed to Step 2 until all 8 behave as expected — this is the one part of the feature no automated test in this plan exercises end-to-end.

- [ ] **Step 2: Add the FEATURES.md entry**

Add a new entry at the top of the `## Implemented ✅` section in `FEATURES.md` (after line 8), following the existing entry format:

```markdown
### feat(session/tui): background agents — `/bg`, `/agents` (list/view/kill)

Lets a user fire off independent tasks that run in the background inside the
current TUI session, each optionally on its own model, and monitor them
without blocking the main conversation. `/bg <goal> [--model <slug>]` spawns
a detached tokio task running its own `Session` (fresh history, not shared
with the main conversation); model selection falls back to the existing
`task_classifier` + `model_routes` lookup when `--model` is omitted.
`/agents` lists active agents (id, model, status, elapsed, goal); `/agents
view <id>` shows a running agent's elapsed time or a finished agent's
summary/files-changed/turn-count; `/agents kill <id>` aborts one in flight.
Capped by `max_background_agents` (default 5). Transcripts persist to the
normal `sessions`/`session_messages` tables (findable later via
`/sessions`), even though the live `/agents` registry is scoped to the TUI
process that spawned them.

Also fixes a latent bug found while building this: destructive shell
commands (`rm -rf`, `git push --force`, `DROP TABLE`, ...) under
`is_subagent = true` previously queued an interactive approval prompt that
could never be answered (no controlling terminal), hanging the turn
forever. They now auto-deny with a clear reason instead — this also fixes
the existing model-invoked `spawn_agent` tool, not just `/bg`.

**Files:** `src/session/background_agent.rs`, `src/agent_core.rs`,
`src/session/tools.rs`, `src/session/mod.rs`, `src/config/mod.rs`,
`src/tui/app.rs`, `src/tui/background_handler.rs`, `src/tui/turn_handler.rs`,
`src/tui/channel.rs`, `src/tui/commands/mod.rs`,
`src/session/commands/info.rs`, `tests/e2e/test_background_agents.sh`

Design spec: `docs/specs/2026-07-05-background-agents-design.md`

---

```

- [ ] **Step 3: Run the full test suite one last time**

Run: `cargo test --lib`
Expected: PASS, full suite

Run: `./tests/e2e/test_background_agents.sh && ./tests/e2e/test_scheduler.sh && ./tests/e2e/test_model_routing.sh`
Expected: PASS, all three e2e scripts (confirms no interference between the new commands and the two existing features this plan builds on)

- [ ] **Step 4: Commit**

```bash
git add FEATURES.md
git commit -m "$(cat <<'EOF'
docs(features): register background agents (/bg, /agents) in FEATURES.md
EOF
)"
```

---

## Self-Review Notes

**Spec coverage:** every section of `docs/specs/2026-07-05-background-agents-design.md` is implemented — Architecture (Tasks 6-8), Permission & safety (Task 3), Commands (Tasks 9-10), Config (Task 1), Testing (Tasks 1-11 unit-test the pure/deterministic pieces; Task 12 covers the concurrent-run/kill/destructive-block scenarios the spec's "Manual" bullet called for).

**Deviations from the spec, and why (all discovered while grounding the plan in real code, not preference):**
- `BackgroundAgent` drops the `session_id: i64` field the spec listed — `Session::new` assigns a session's id internally and asynchronously inside the spawned task, so the outer registry entry literally cannot know it at construction time, and nothing in the final `/agents`/`/agents view` design actually needed it (a finished agent's summary/files/turns are carried directly in `BgStatus::Done`, not re-fetched from SQLite).
- IDs are a plain monotonic counter ("1", "2", ...) instead of a "4-hex slug" — avoids adding a `rand` dependency for no functional benefit; the spec's `a1b2`/`c3d4` were illustrative, not a requirement.
- `/agents view` on a `Running` agent shows only elapsed time, not "last known turn count + last tool call name" — true live progress would require new shared-mutable-state plumbing through `session/turn.rs`, which conflicts with the user's explicit "UI can be added later" steer during brainstorming. Full detail is available the moment it's `Done`.

**Type consistency check:** `turns`/`tool_calls` are `usize` throughout (`BgStatus`, `BgOutcome`, `agent_core::SubagentResult`) matching `Session::turn_count: usize` — the spec's draft used `u32` for `turns` in one place, corrected here.

**Placeholder scan:** none — every step has complete code, exact file paths, and exact commands.
