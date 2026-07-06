# Usage-window watcher — Voiceover Script
# Video length: ~68s  |  Suggested playback: 1x
# Format: [timestamp] — what's on screen, then your words

---

## [0:00] — Title card (~5s)

"Claude Pro and ChatGPT Codex both meter you on a rolling 5-hour window.
... You find out it ran out when your session just stops answering.
... zap watches both, live, right in the sidebar."

---

## [0:05] — Codex launches, question sent

[TUI boots, question typed and sent]

"One real question through Codex."

---

## [~0:30] — Codex quota panel visible

[sidebar shows `quota (codex)` — 5h 3%, 7d 70%]

"There — `quota (codex)`. Five-hour window, seven-day window,
... straight from Codex's own response headers. No dashboard, no extra command."

[pause 1-2s]

---

## [~0:38] — Switch to Claude Code

[zap relaunches with the claude_code provider]

"Same thing on Claude."

---

## [~0:53] — Claude quota panel visible

[sidebar shows `quota (claude)` — 5h 42%, 7d 20%, reset countdown]

"`quota (claude)` — same idea. Anthropic doesn't expose a CLI flag for this yet,
... so zap reads the same data Claude Code's own status line uses.
... Even the reset countdown."

[pause 1-2s]

---

## [~1:00] — Summary card (~8s)

"Cross eighty percent, either one, and zap warns you once — not every turn —
... and tells you to switch before you actually hit the wall.
... One window. Two subscriptions. Zero surprises."

---

## Delivery notes

- Tone: calm, factual. Let the real numbers do the talking.
- Two money shots: the `quota (codex)` panel appearing (~0:30) and the
  `quota (claude)` panel appearing (~0:53) — hold 1-2s of silence on each.
- Don't say "Claude Max" — say "Claude Pro" (or just "Claude").
- Don't overclaim the Claude endpoint's official status — "zap reads the same
  data Claude Code's own status line uses" is accurate; avoid stronger language.
- If your own re-recording runs long on the Codex wait (its response latency
  varies — sometimes ~5s, sometimes 15s+), that's normal; the tape budgets 22s
  to stay safe. Trim in editing if your take finishes early.
