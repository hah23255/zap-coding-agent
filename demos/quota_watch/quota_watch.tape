# ⚡ zap — usage-window watcher demo (Codex + Claude 5h/7d sidebar)
#
# Timing:
#   Title card      12s
#   Phase card       8s
#   TUI + 1 turn    ~25s ← real Codex call, sidebar quota panel populates
#   Sidebar hold     8s  ← pause on the live quota % so it reads clearly
#   Summary card    18s
#   Total          ~70s (pad with room to breathe when editing in voiceover)
#
# Pre-flight:
#   - zap binary must be on PATH and codesigned (zap-install)
#   - Run from repo root: VHS_NO_SANDBOX=1 vhs demos/quota_watch/quota_watch.tape
#   - Uses the real `codex` provider (already configured in ~/.agent.toml) —
#     this makes one real, cheap API call and will show your real usage %.

Output demos/quota_watch/quota_watch.mp4

Set FontSize 14
Set Width 1400
Set Height 800
Set Theme "TokyoNight"
Set TypingSpeed 40ms

Hide
Type `export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"`
Enter
Sleep 200ms
Ctrl+L
Show

# ── 1. Title card ──────────────────────────────────────────────────────────────
Sleep 500ms
Type "demos/quota_watch/scenarios/_title.sh"
Sleep 400ms
Enter
Sleep 12s

# ── 2. Phase card ──────────────────────────────────────────────────────────────
Ctrl+L
Sleep 300ms
Type `demos/quota_watch/scenarios/_phase.sh "Step 1" "Live in the sidebar" "zap checks Codex's real usage headers on every reply." "For Claude it reads the same data Claude Code's own statusLine uses." "Both show up as a color-coded 5h / 7d readout — no separate dashboard."`
Sleep 400ms
Enter
Sleep 8s

# ── 3. Launch TUI, ask one real question (account's default codex model —
#       o4-mini/gpt-4o returned 400 for this account, so this uses whatever
#       ~/.agent.toml already has configured and known-working) ───────────────
Ctrl+L
Sleep 300ms
Type "AGENT_PERMISSION_MODE=auto zap"
Sleep 600ms
Enter
Sleep 5s

Type "/new"
Sleep 400ms
Enter
Sleep 1s

Type "In one sentence, what does src/quota_watch.rs do?"
Sleep 600ms
Enter
Sleep 90s

# ── 4. Hold on the sidebar — quota panel now populated ────────────────────────
Sleep 6s

# ── 5. Exit ────────────────────────────────────────────────────────────────────
Type "/exit"
Sleep 400ms
Enter
Sleep 4s

# ── 6. Summary card ────────────────────────────────────────────────────────────
Ctrl+L
Sleep 500ms
Type "demos/quota_watch/scenarios/_summary.sh"
Sleep 400ms
Enter
Sleep 18s
