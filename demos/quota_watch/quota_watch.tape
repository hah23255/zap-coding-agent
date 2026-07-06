# ⚡ zap — usage-window watcher demo (60-70s, both providers)
#
# Codex's response latency before the first API round-trip (which is when the
# quota headers arrive — well before the full answer streams) varies a lot for
# this account's model: sometimes ~5s, sometimes 16s+. 22s is a safe margin
# observed across several live test runs. Claude Code's own usage check fires
# before the claude subprocess even spawns, so it's reliably fast (~2-5s).
#
# Timing (visible):
#   Title card        5s
#   Codex quota      ~30s ← real call; wait is generous on purpose (variance)
#   Claude quota     ~13s ← reliably fast
#   Summary card      8s
#   Total           ~60-65s
#
# Pre-flight:
#   - zap binary must be on PATH and codesigned (zap-install)
#   - Run from repo root: VHS_NO_SANDBOX=1 vhs demos/quota_watch/quota_watch.tape
#   - Uses the real `codex` and `claude_code` providers — two real, cheap calls.

Output demos/quota_watch/quota_watch.mp4

Set FontSize 15
Set Width 1400
Set Height 800
Set Theme "TokyoNight"
Set TypingSpeed 30ms

Hide
Type `export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"`
Enter
Sleep 200ms
Ctrl+L
Show

# ── 1. Title card ──────────────────────────────────────────────────────────────
Sleep 300ms
Type "demos/quota_watch/scenarios/_title.sh"
Sleep 300ms
Enter
Sleep 5s

# ── 2. Codex ───────────────────────────────────────────────────────────────────
Ctrl+L
Sleep 300ms
Type "AGENT_PERMISSION_MODE=auto zap"
Sleep 400ms
Enter
Sleep 3s

Type "/new"
Sleep 300ms
Enter
Sleep 800ms

Type "In one sentence, what does src/quota_watch.rs do?"
Sleep 400ms
Enter
Sleep 22s

# Hold on the Codex quota panel
Sleep 3s

Ctrl+C
Sleep 500ms
Type "/exit"
Sleep 300ms
Enter
Sleep 1s

# ── 3. Claude Code ─────────────────────────────────────────────────────────────
Ctrl+L
Sleep 300ms
Type "AGENT_PROVIDER=claude_code AGENT_PERMISSION_MODE=auto zap"
Sleep 400ms
Enter
Sleep 3s

Type "/new"
Sleep 300ms
Enter
Sleep 800ms

Type "Say hi in 3 words."
Sleep 400ms
Enter
Sleep 8s

# Hold on the Claude quota panel
Sleep 2s

Ctrl+C
Sleep 500ms
Type "/exit"
Sleep 300ms
Enter
Sleep 1s

# ── 4. Summary card ────────────────────────────────────────────────────────────
Ctrl+L
Sleep 300ms
Type "demos/quota_watch/scenarios/_summary.sh"
Sleep 300ms
Enter
Sleep 8s
