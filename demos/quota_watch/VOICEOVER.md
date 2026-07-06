# Usage-window watcher — Voiceover Script
# Video length: ~2:39 (raw recording, untrimmed)  |  Suggested playback: 1x (no speed-up needed)
# Format: [timestamp] — what's on screen, then your words

Editing note up front: the middle section (~0:33–2:03) is a real, live Codex
call — the recording waits out an actual ~90-second reasoning response so the
sidebar quota panel populates with genuinely live data, not a mock. That's a
long stretch of "thinking..." spinner on screen. Recommend **time-lapsing or
cutting that segment down to ~15-20s** in the edit (speed ramp or a jump cut
right after the question is sent, landing on the completed answer + quota
panel) unless you want the realism of an unedited wait. The script below
assumes you trim it.

---

## [0:00] — Title card (~12s)

[let it land for 2 seconds before speaking]

"Claude Max and ChatGPT Codex both meter you on a rolling 5-hour window.
... You usually find out it ran out the hard way —
... your session just stops answering."

[pause 1s]

"zap watches both windows live, right in the sidebar,
... and warns you before you hit the wall — not after."

---

## [0:14] — Phase card (~8s)

"Here's how — Codex's real usage headers, checked on every reply.
... For Claude, the same data its own status line uses, rechecked every few minutes."

---

## [0:24] — zap launches, `/new`, question typed (~10s)

[TUI boots, tools/skills load]

"Let's see it live. Fresh session —
... and one quick question."

[question sent: "In one sentence, what does src/quota_watch.rs do?"]

---

## [0:33] — Thinking / real Codex call in flight

**If trimmed in edit:** cut here, resume at the completed answer.

"Real API call, real response — this isn't staged."

[if left untrimmed, let this run silently or add a beat here — don't over-narrate a spinner]

---

## [~0:50 after your edit] — Answer completes, sidebar populated

[assistant text visible; sidebar right side now shows a new `quota (codex)` section]

"There it is —
... `quota (codex)`. Five-hour window: one percent.
... Seven-day window: sixty-nine percent.
... Live numbers, straight from Codex's own response headers.
... No separate dashboard, no extra command — it's just there."

[pause 2s, let the viewer read the sidebar]

"Cross eighty percent and zap warns you —
... once per window, not spammed every turn —
... and tells you to switch providers before you actually hit the wall."

---

## [~1:00] — Exit, summary card (~18s)

[TUI closes, summary card appears]

"Codex: real response headers, checked every reply.
... Claude: the same data its own status line uses — Anthropic doesn't
... expose a CLI flag for this yet, so zap reads it directly, every five minutes."

[pause 1s]

"Best-effort, by design — if either check ever fails, it just stays quiet.
... It never blocks or breaks a real turn."

[pause 2s]

"One window. Two subscriptions. Zero surprises."

[let the card hold, then fade]

---

## Delivery notes

- Tone: calm, factual — this is a utility feature, not a hype feature. Let the
  real numbers (1%, 69%) do the talking; don't oversell.
- The **money shot** is the sidebar `quota (codex)` panel appearing — hold on
  it for a good 2-3 seconds of silence before continuing.
- If you keep the full untrimmed wait, say nothing for most of it — a long
  silent "thinking" spinner with no narration reads as honest, not boring.
- Don't claim the Claude endpoint is officially supported — the script says
  "zap reads it directly" and "Anthropic doesn't expose a CLI flag for this
  yet," which is accurate. Avoid stronger language than that.
