# ⚡ zap — two code-intelligence examples
#
# Example 1: symbol navigation   — find_definition("url_for")      1 call vs 7+
# Example 2: blast-radius        — who_calls("make_response")       1 call vs grep 83 files
#
# Recording starts when TUI is visible; bash setup is hidden.
# Two TUI sessions with scene cards between them.
#
# Usage (run from demos/code_indexing/):
#   VHS_NO_SANDBOX=1 vhs two_examples.tape
#
# Real time: ~135s  →  ~90s video at PlaybackSpeed 1.5

Output two_examples.mp4
Set Width 1400
Set Height 820
Set FontSize 13
Set Theme "TokyoNight"
Set PlaybackSpeed 1.5
Set TypingSpeed 55ms

# ── Hidden setup: PATH + cd only ─────────────────────────────────────────────
Hide
Type `export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"`
Enter
Sleep 200ms
Type "cd flask"
Enter
Sleep 200ms
Ctrl+L
Show

# ── Title card (clean terminal, TokyoNight background) ────────────────────────
Type "../scenarios/_title.sh"
Enter
Sleep 5s

# ── Example 1 scene card ─────────────────────────────────────────────────────
Ctrl+L
Sleep 200ms
Type "../scenarios/_scene_nav.sh"
Enter
Sleep 5s

# ── TUI session 1 ─────────────────────────────────────────────────────────────
Ctrl+L
Sleep 200ms
Type "AGENT_PERMISSION_MODE=auto zap"
Enter
# Wait for TUI to fully render (sidebar, input box, domain pick if any)
Sleep 7s

Type "Where is url_for defined in this codebase? Give me the file and line number."
Sleep 700ms
Enter
# find_definition hits the index — LLM just formats the answer
# Real wait ~45s; at 1.5x = 30s video
Sleep 45s

Type "/exit"
Enter
Sleep 2s

# ── Example 2 scene card ─────────────────────────────────────────────────────
Ctrl+L
Sleep 200ms
Type "../scenarios/_scene_blast.sh"
Enter
Sleep 5s

# ── TUI session 2 ─────────────────────────────────────────────────────────────
Ctrl+L
Sleep 200ms
Type "AGENT_PERMISSION_MODE=auto zap"
Enter
Sleep 7s

Type "I am about to refactor make_response. What functions in this codebase call it? List each with file and line number."
Sleep 700ms
Enter
# who_calls hits the index — more callers to list so slightly longer answer
# Real wait ~60s; at 1.5x = 40s video
Sleep 60s

Type "/exit"
Enter
Sleep 2s

# ── Closing summary card ──────────────────────────────────────────────────────
Ctrl+L
Sleep 200ms
Type "../scenarios/_close_two.sh"
Enter
Sleep 7s
