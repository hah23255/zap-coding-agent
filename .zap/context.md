# Session Context

<!-- auto-written by zap at session end — edit freely -->

## Last updated
2026-07-06 18:23 — Session #559

## What was being worked on
Investigated why the `claude_code` provider (routes through the local `claude` CLI
subprocess) produced worse output than running `claude` directly, fixed the root
causes, and added proactive 5-hour/weekly usage-window warnings for both Codex
and Claude.

## Files touched
  - src/context_manager.rs (new `build_claude_code_system_prompt` — lean prompt without zap's own tool vocabulary)
  - src/session/mod.rs (route `claude_code` provider to the new lean prompt)
  - src/llm_client/claude_code.rs (fixed `Ask`/`Deny` permission-mode collapse; added `warn_if_usage_limit` + wired `quota_watch::check_claude_usage_if_stale`)
  - src/llm_client/mod.rs (pass full `PermissionMode` into `ClaudeCodeClient::new` instead of a bool)
  - src/llm_client/codex.rs (wired in `quota_watch::check_codex_usage`)
  - src/lib.rs (registered new `quota_watch` module)
  - src/quota_watch.rs (new — proactive Codex + Claude 5h/weekly usage watcher, one-time warning per window crossing)

## What's next
- Root cause found and fixed: `ClaudeCodeClient::send` never forwarded zap's tool
  schemas to the `claude` subprocess (`_tools` param unused), yet zap's full
  system prompt (built for API-driven providers) told the model to follow a
  strict tool order for tools (`code_map`, `edit_file`, `batch_edit`,
  `spawn_agent`, etc.) that don't exist in that process. Fixed by giving
  `claude_code` its own lean prompt (identity, ZAP.md/understanding.md, memory,
  safety rules, git status — no tool-policy noise). Verify in a live session
  that output quality now matches running `claude` bare.
- Fixed a real safety bug alongside it: zap's `Ask` and `Deny` permission modes
  both silently mapped to Claude Code's `acceptEdits` (auto-accept every edit).
  Now: `Auto`→`bypassPermissions`, `Ask`→`default`, `Deny`→`plan` (read-only).
  `Ask` still isn't a live per-edit approval prompt in zap's TUI yet — that needs
  the subprocess's stdin kept open for the whole turn (currently closed after one
  write) plus parsing Claude Code's own permission-request stream-json events and
  bridging them into the existing `PermissionPromptRequest`/`take_perm_request`
  channel (`src/tui/channel.rs`). Real feature, not started.
- 5-hour/weekly usage warnings, both now proactive:
  - **Codex**: real, documented-by-precedent response headers
    (`x-codex-primary-used-percent`, `x-codex-secondary-used-percent`) checked on
    every response in `codex.rs`.
  - **Claude**: no official CLI flag exists (anthropics/claude-code#20399, #38380,
    #44328 all open feature requests) — but Anthropic's undocumented
    `https://api.anthropic.com/api/oauth/usage` endpoint (the same data source
    Claude Code's own official `statusLine` feature exposes as
    `rate_limits.five_hour`/`.seven_day`, per code.claude.com/docs/en/statusline)
    was tested live this session with the user's real Claude Code OAuth token
    (read from macOS Keychain, service "Claude Code-credentials") and confirmed
    working — returned real `five_hour`/`seven_day` utilization + reset
    timestamps over HTTP 200. Implemented as `quota_watch::check_claude_usage_if_stale`,
    called at the top of every `claude_code` turn, throttled to once per 5 min,
    every failure mode swallowed silently (best-effort side channel, must never
    block or break a real turn). This endpoint is unofficial/undocumented and
    could change without notice — that risk was flagged to and accepted by the user.
- A Medium/LinkedIn article draft about the system-prompt/tool-vocabulary bug was
  written to the scratchpad (not in the repo) — user has it open in their IDE;
  may want a pass before publishing, and may want the usage-window story added
  as a second section/follow-up post.
