# zap — usage-window watcher demo

Shows the sidebar `quota` panel: live 5-hour/weekly usage percentages for
Codex (real response headers) and Claude (Anthropic's undocumented
`/api/oauth/usage` endpoint, same data Claude Code's own `statusLine` feature
uses), plus the one-time 80%-threshold warning banner.

## Files

- `quota_watch.tape` — VHS script. Boots real `zap` in this repo, sends one
  real question through the `codex` provider, and holds on the sidebar once
  the quota panel populates.
- `scenarios/_title.sh`, `_phase.sh`, `_summary.sh` — printf-styled terminal
  cards for the hook, setup, and recap.
- `VOICEOVER.md` — timestamped narration script for adding voiceover audio
  after recording.
- `quota_watch.mp4` — rendered output (not committed — see `.gitignore`).

## Re-recording it

```
zap-install   # make sure the binary on PATH has your latest changes
VHS_NO_SANDBOX=1 vhs demos/quota_watch/quota_watch.tape
```

Uses the real `codex` provider already configured in `~/.agent.toml` — this
makes one real, live API call and shows your actual account's usage
percentages (in this recording: 5h 1%, 7d 69%). No fixture repo needed; it
runs directly against this repo since the demo is about zap itself, not code
navigation.

**Heads up if you re-record:**

- **Model matters.** `~/.agent.toml`'s configured Codex model is the only one
  confirmed to work for this account — `gpt-4o` and `o4-mini` both returned
  `400 Bad Request` when tried as an `AGENT_MODEL` override. Don't override
  the model unless you've confirmed it works first.
- **Reasoning models are slow.** The configured model took roughly 90 seconds
  to fully answer a one-sentence question. The tape budgets `Sleep 90s` after
  the question for exactly this reason — don't shorten it below what your
  account's actual model needs, or the recording will end mid-"Contemplating"
  with no quota panel ever showing (this happened twice while building this
  demo). Plan to time-lapse or cut this wait down in editing instead of
  recording it shorter.
- **Running zap in this repo directory writes to `.zap/context.md` and
  `.zap/session_log.md`.** After recording, restore them if you don't want
  the demo's throwaway question logged as real session history:
  `git checkout HEAD -- .zap/context.md .zap/session_log.md .zap/understanding.md`
