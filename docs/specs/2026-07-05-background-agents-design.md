# Background agents — `/bg`, `/agents` (design)

Date: 2026-07-05

## Problem

A dev wants to fire off several independent tasks — each potentially on a
different model — without babysitting each one in a separate terminal, and
without losing the ability to keep chatting in the main session while they run.

zap already has two adjacent pieces that don't connect today:

- **Per-turn model routing** (`model_routes` config + `task_classifier.rs` +
  `session/routing.rs`): auto-picks a model for the *current* turn based on
  keyword classification. Sequential, single active model at a time.
- **`spawn_agent` tool** (`tools/agent.rs` + `agent_core::run_subagent`):
  lets the *model* spawn an independent sub-session with its own history and
  full tool access, in parallel with sibling `spawn_agent` calls in the same
  response — but it's synchronous from the user's point of view (the parent
  turn blocks on `.await` until all spawned sub-agents finish) and always
  inherits the parent's model.
- **Scheduler** (`session/scheduler.rs`): runs tokio background tasks, but
  they queue goals onto the *same* session/history/model, serialized.

None of these let a *user* say "run this on model X, in the background, and
let me check on it later." This spec adds that as `/bg` + `/agents`, built
almost entirely out of the existing `run_subagent` plumbing.

## Non-goals

- A visual dashboard/sidebar. Text commands in the transcript are sufficient
  for now (explicitly deprioritized by the user — UI polish can follow later).
- Cross-process / detached-OS-process execution. Background agents are
  in-process tokio tasks tied to the lifetime of the TUI session that spawned
  them (dies with that process) — a deliberate scope choice, not an oversight.
- A new `PermissionMode` variant or new `agent.toml` permission surface.

## Architecture

### New module: `src/session/background_agent.rs`

```rust
pub struct BackgroundAgent {
    pub id: String,           // short 4-hex slug, same style as scheduled jobs
    pub goal: String,
    pub model: String,
    pub session_id: i64,      // row in the existing `sessions` table
    pub status: BgStatus,
    pub started_at: chrono::DateTime<Local>,
    pub handle: tokio::task::JoinHandle<()>,
}

pub enum BgStatus {
    Running,
    Done { summary: String, files_changed: Vec<String>, turns: u32 },
    Failed(String),
    Killed,
}
```

Lives in `App.background_agents: Vec<BackgroundAgent>` (TUI state), mirroring
the existing `App.scheduled_jobs` pattern. Scoped to the current process —
no new SQLite table for the registry itself.

### Model resolution

Reuses the exact lookup `session/routing.rs::route_for_turn` already does,
just invoked outside the turn loop:

```rust
fn resolve_bg_model(goal: &str, explicit: Option<String>, config: &Config) -> String {
    explicit.unwrap_or_else(|| {
        let t = task_classifier::classify(goal);
        config.model_routes.get(t.as_str()).cloned().unwrap_or_else(|| config.model.clone())
    })
}
```

`--model <slug>` always wins; omitted falls back to `task_classifier::classify`
+ `model_routes`, falling back again to the session's current default model.

### Spawn flow

Extends `agent_core::run_subagent`'s existing pattern, but detached instead
of awaited inline:

1. `persistence::save_session(goal, model, cwd)` → `session_id`. Existing
   function, existing `sessions` table — **no schema migration**.
2. Clone `Config` → `sub_config`: set `model`, `is_subagent = true` (reused —
   see Permission section below), and propagate `agent_depth` /
   `spawn_depth` exactly as `run_subagent` does today, so the existing
   recursion-depth cap applies here too (a `/bg`'d agent can't `/bg`
   infinitely, and can't call `spawn_agent` past the existing depth limit).
3. `tokio::spawn`:
   - `Session::new(&sub_config)`, then `session.handle_user_turn(goal)`.
   - On completion, extract `{summary, files_changed, turns}` — factor this
     out of `run_subagent`'s existing inline extraction (lines ~341–369 of
     `agent_core.rs`) into a shared helper `agent_core::extract_result(&Session)`
     used by both `run_subagent` and this path.
   - `persistence::save_messages(session_id, ...)` — final transcript durable
     in SQLite even though the live registry entry disappears when the process
     exits. `/sessions` can still find it later.
   - Send `TuiEvent::BackgroundAgentDone { id, status }` over the existing
     tui-event channel (same channel `Notice` / `ScheduledFire` already use).
4. Push the `BackgroundAgent` (holding the `JoinHandle`) into
   `App.background_agents`.

## Permission & safety

### The fix (also repairs an existing latent bug)

`run_subagent` forces `PermissionMode::Auto` with the comment "no controlling
terminal, prompting would deadlock" — but `session/tools.rs`'s `force_prompt`
check (driven by `tools::shell::destructive_pattern`) pushes destructive shell
commands (`rm -rf`, `git push --force`, `DROP TABLE`, ...) onto the
interactive prompt queue **regardless of permission mode**. Today, if a
spawned sub-agent's model attempts one of these, it hangs forever waiting for
a prompt that can never be answered.

Fix in `session/tools.rs`: when `session.config.is_subagent` is true (already
set for model-invoked sub-agents, and now also set for `/bg` agents — both
are "no controlling terminal" situations), a `destructive_pattern` match skips
the prompt queue and returns an immediate tool error:

```
blocked: {reason} — destructive commands require interactive approval,
not available in an unattended sub-agent.
```

The model sees this as an ordinary tool failure and can react instead of
hanging. This is a real bug fix that happens to be required for `/bg` too —
worth its own commit, separate from the `/bg` feature commit.

Everything else about sub-agent permissions is unchanged: `Auto` mode,
reads/writes/shell allowed, only the destructive subset is affected. No new
`PermissionMode` variant, no new config key for this.

## Commands

### `/bg <goal> [--model <slug>]`

- Reject if `count(status == Running) >= config.max_background_agents`:
  `✗ Cannot start: N background agents already running (max_background_agents = N)`.
- Resolve model, spawn, print: `Started agent a1b2 (codex/gpt-5.5) — /agents to check.`

### `/agents`

Table in the transcript:

```
ID    MODEL             STATUS    ELAPSED  GOAL
a1b2  codex/gpt-5.5      running   2m14s    refactor auth middleware
c3d4  claude-sonnet-5    done      0m48s    write tests for parser
```

### `/agents view <id>`

- `Running`: last known turn count + last tool call name (lightweight —
  no live token streaming; avoids needing shared mutable transcript state
  mid-run).
- `Done` / `Failed`: the `{summary, files_changed, turns}` captured at
  completion.

### `/agents kill <id>`

`handle.abort()` — same mechanism `/unschedule` already uses for
`ScheduledJob`. Marks `Killed`; partial transcript already persisted up to
that point remains as-is.

### Completion notice

On finish, `TuiEvent::BackgroundAgentDone` → `apply_event` appends one line
to the main transcript, without touching the active conversation otherwise:

```
✓ Background agent a1b2 finished: "refactor auth middleware" (codex/gpt-5.5, 4m02s)
   /agents view a1b2 for details
```

(or `✗ ... failed: <short reason>` for `Failed`.)

## Config

`Config` / `FileConfig` gain:

```toml
max_background_agents = 5   # default 5, same serde-default pattern as model_routes
```

## Testing

- Unit: `resolve_bg_model` — explicit override wins, falls back to
  `model_routes`, falls back to default model; mirrors existing
  `task_classifier` / `routing.rs` test style.
- Unit: cap enforcement — `/bg` rejected at `max_background_agents`, accepted
  below it.
- Unit: `session/tools.rs` destructive-pattern fix — a `destructive_pattern`
  match under `is_subagent = true` returns a tool error immediately rather
  than queuing a prompt (regression test for the bug fix; this alone should
  land with a test proving the *old* behavior would have hung).
- E2E (shell harness, following `tests/e2e/test_model_routing.sh` style):
  `/bg` a trivial goal with an explicit `--model`, poll `/agents` until
  `done`, assert the completion notice and `/agents view` output.
- Manual: `/bg` two goals with different `--model` values concurrently,
  confirm both complete independently and `/agents` shows both, confirm
  `/agents kill <id>` actually stops an in-flight one.

## Files touched (expected)

`src/session/background_agent.rs` (new), `src/agent_core.rs` (extract shared
result-extraction helper), `src/session/tools.rs` (destructive-pattern fix),
`src/session/routing.rs` (expose model-resolution helper or duplicate the
small lookup), `src/config/mod.rs`, `src/config/tests.rs`,
`src/tui/app.rs`, `src/tui/turn_handler.rs` or `src/tui/commands/mod.rs`
(slash command wiring), `src/tui/mod.rs` (`TuiEvent::BackgroundAgentDone` +
`apply_event`), `FEATURES.md`.
