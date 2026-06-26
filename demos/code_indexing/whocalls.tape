# ⚡ zap — Call-Graph Navigation demo
#
# Scenario: "What calls make_response in Flask?"
# Shows who_calls() hitting the SQLite symbol index — 1 tool call vs grep across 83 files.
#
# Usage (run from demos/code_indexing/):
#   VHS_NO_SANDBOX=1 vhs whocalls.tape
#
# Total real time: ~85s → ~57s video at PlaybackSpeed 1.5

Output whocalls.mp4
Set Width 1400
Set Height 820
Set FontSize 13
Set Theme "TokyoNight"
Set PlaybackSpeed 1.5
Set TypingSpeed 60ms

Hide
Type `export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"`
Enter
Sleep 300ms
Type "cd flask"
Enter
Ctrl+L
Show

# ── 1. Title card ─────────────────────────────────────────────────────────────
Sleep 400ms
Type "../scenarios/_title_whocalls.sh"
Enter
Sleep 5s

# ── 2. Scene explanation ──────────────────────────────────────────────────────
Ctrl+L
Sleep 200ms
Type "../scenarios/_scene_whocalls.sh"
Enter
Sleep 5s

# ── 3. TUI session ────────────────────────────────────────────────────────────
Ctrl+L
Sleep 200ms
Type "AGENT_PERMISSION_MODE=auto zap"
Enter
# Wait for TUI to boot
Sleep 6s

# The question: natural developer question before a refactor
Type "Before I refactor make_response — what functions in this codebase call it? List them with file and line."
Sleep 800ms
Enter

# Wait for LLM: who_calls() hits the index (fast), then LLM writes the answer
# Real wait ~65s; at 1.5x playback = ~43s video
Sleep 65s

# Exit TUI
Type "/exit"
Enter
Sleep 2s

# ── 4. Closing card ───────────────────────────────────────────────────────────
Type "clear && ../scenarios/_close_whocalls.sh"
Enter
Sleep 6s
