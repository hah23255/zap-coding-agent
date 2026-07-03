#!/usr/bin/env bash
# Real provider-backed TUI verification: launch zap in tmux, use the user's saved
# provider config, build a real code index, ask for a real symbol lookup, and capture
# the rendered pane so Ctrl+O expansion and multi-line tool rendering are exercised
# as an end user would.
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ARTIFACT_DIR="$SCRIPT_DIR/artifacts"
mkdir -p "$ARTIFACT_DIR"
STAMP=$(date +%Y%m%d-%H%M%S)
SESSION="zap_real_tui_$STAMP"
CAPTURE="$ARTIFACT_DIR/tui-real-provider-$STAMP.txt"
SUMMARY="$ARTIFACT_DIR/tui-real-provider-$STAMP.summary.txt"
RAW="$ARTIFACT_DIR/tui-real-provider-$STAMP.raw.txt"
ZAP_BIN="${ZAP_BIN:-$REPO_ROOT/target/debug/zap}"

cleanup() {
  tmux kill-session -t "$SESSION" 2>/dev/null || true
}
trap cleanup EXIT

info "Real TUI evidence: launch zap with saved provider config in tmux"

cd "$REPO_ROOT"
tmux new-session -d -s "$SESSION" "cd '$REPO_ROOT' && ZAP_TRUST_PROJECT=1 AGENT_PERMISSION_MODE=auto '$ZAP_BIN'"
tmux set-option -t "$SESSION" history-limit 20000
sleep 4

# Accept onboarding/provider picker defaults if they appear.
tmux send-keys -t "$SESSION" Enter
sleep 2

# Build the real code index in-session so index-backed tool flows are available.
tmux send-keys -t "$SESSION" /index Enter
sleep 25

# Ask for a concrete symbol lookup that should trigger an index-backed tool call.
tmux send-keys -t "$SESSION" "where is project_trusted defined? show the exact tool query" Enter
sleep 25

# Expand latest tool output preview.
tmux send-keys -t "$SESSION" C-o
sleep 3

# Capture the visible pane and full scrollback.
tmux capture-pane -p -S -5000 -t "$SESSION" > "$RAW"
tmux capture-pane -e -p -S -5000 -t "$SESSION" > "$CAPTURE"

tmux send-keys -t "$SESSION" C-q
sleep 1
tmux send-keys -t "$SESSION" C-q
sleep 1

{
  echo "capture_file=$CAPTURE"
  echo "raw_file=$RAW"
  echo "repo=$REPO_ROOT"
  echo "binary=$ZAP_BIN"
  echo "checks:"
  echo "- indexed command sent: /index"
  echo "- prompt sent: where is project_trusted defined? show the exact tool query"
  echo "- expand sent: Ctrl+O"
  echo
  echo "grep_hits:"
  grep -n '"symbol": "project_trusted"' "$RAW" || true
  grep -n "project_trusted" "$RAW" || true
  grep -n "Ctrl+O" "$RAW" || true
  echo
  echo "tail:"
  tail -n 80 "$RAW"
} > "$SUMMARY"

pass "Real provider-backed TUI evidence captured: $SUMMARY"
summary
