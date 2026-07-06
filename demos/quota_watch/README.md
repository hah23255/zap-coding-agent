# zap — usage-window watcher demo

Shows the sidebar `quota` panel for **both** providers: live 5-hour/weekly
usage percentages for Codex (real response headers) and Claude (Anthropic's
undocumented `/api/oauth/usage` endpoint, same data Claude Code's own
`statusLine` feature uses), plus the one-time 80%-threshold warning banner.
~68 seconds, punchy cut — no long dead-air waits.

## Files

- `quota_watch.tape` — VHS script. Boots real `zap` twice in this repo: once
  on `codex`, once on `AGENT_PROVIDER=claude_code`, holding on each sidebar
  once its quota panel populates.
- `scenarios/_title.sh`, `_summary.sh` — printf-styled terminal cards for the
  hook and recap. `_phase.sh` is left in the folder but unused in the current
  cut (dropped to keep it tight — feel free to reintroduce for a longer edit).
- `VOICEOVER.md` — timestamped narration script for adding voiceover audio
  after recording.
- `quota_watch.mp4` — rendered output (not committed — root `.gitignore`
  excludes `*.mp4`; lives locally only).

## Re-recording it

```
zap-install   # make sure the binary on PATH has your latest changes
VHS_NO_SANDBOX=1 vhs demos/quota_watch/quota_watch.tape
```

Makes two real, live API calls (Codex + Claude Code) and shows your actual
account's usage percentages. No fixture repo needed — it runs directly
against this repo since the demo is about zap itself, not code navigation.

**Heads up if you re-record:**

- **Model matters for Codex.** `~/.agent.toml`'s configured model
  (`gpt-5.4` at time of writing) is the only one confirmed to work for this
  account — `gpt-4o` and `o4-mini` both returned `400 Bad Request` when tried
  as an `AGENT_MODEL` override. Don't override the model unless you've
  confirmed it works first (a quick raw `curl` against
  `https://chatgpt.com/backend-api/codex/responses` is faster to check than a
  full re-render).
- **Codex's response latency varies a lot.** The quota panel populates on the
  *first* LLM round-trip (headers arrive before the full answer streams), but
  how long that first round-trip takes for this account's model ranged from
  ~5s to 16s+ across test runs — not the full answer time, just getting past
  the very first response. The tape budgets a 22s wait to stay safe; if your
  own re-recording needs longer, that's the number to bump (see the comment
  in `quota_watch.tape`). Don't cut it below ~15s or the quota panel may not
  have appeared yet when the hold/cancel fires.
- **Claude Code's usage check fires before the subprocess even spawns**, so
  it's reliably fast (~2-5s) regardless of how slow Claude's own answer is.
- **`/exit` while a turn is busy is silently ignored.** Press `Ctrl+C` first
  to cancel the in-flight turn (returns to idle), then `/exit` works
  normally. The tape already does this — don't remove it if you edit further.
- **Running zap in this repo directory writes to `.zap/context.md` and
  `.zap/session_log.md`.** After recording, restore them so the demo's
  throwaway questions don't get logged as real session history:
  `git checkout HEAD -- .zap/context.md .zap/session_log.md .zap/understanding.md`
