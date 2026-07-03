#!/usr/bin/env bash
# TUI render evidence for transcript space + fuller collapsed tool previews.
# This is a PTY-based smoke/evidence script, not a full live-provider conversation test.
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ZAP="$(cd "$(dirname "$ZAP")" 2>/dev/null && pwd)/$(basename "$ZAP")"

TMP=$(make_project)
ARTIFACT_DIR="$SCRIPT_DIR/artifacts"
mkdir -p "$ARTIFACT_DIR"
STAMP=$(date +%Y%m%d-%H%M%S)
CAPTURE="$ARTIFACT_DIR/tui-render-$STAMP.txt"
SUMMARY="$ARTIFACT_DIR/tui-render-$STAMP.summary.txt"
trap "rm -rf $TMP" EXIT

info "TUI render evidence: launch real zap TUI in PTY and capture screen output"

EXIT_CODE=0
(cd "$TMP" && expect -f - >"$CAPTURE" 2>&1 <<EXPECT
set timeout 25
spawn $ZAP
sleep 2
send "\r"
sleep 1
send "\033"
sleep 2
send "/exit\r"
expect eof
EXPECT
) || EXIT_CODE=$?

if grep -q "couldn't execute" "$CAPTURE" 2>/dev/null || [ "$EXIT_CODE" -ne 0 ]; then
  fail "TUI render evidence capture" "expect/spawn failed; see $CAPTURE"
  summary
  exit 1
fi

{
  echo "capture_file=$CAPTURE"
  echo "repo=$(pwd)"
  echo "binary=$ZAP"
  echo "verified_source_strings:"
  grep -n 'Constraint::Length(5)' src/tui/render/mod.rs || true
  grep -n 'Constraint::Length(2)' src/tui/render/mod.rs || true
  grep -n 'Constraint::Length(4)' src/tui/render/mod.rs || true
  grep -n 'Ctrl+O to expand' src/tui/render/messages.rs || true
  grep -n 'preview_cap = 3usize' src/tui/render/messages.rs || true
  echo
  echo "capture_tail:"
  tail -n 40 "$CAPTURE" | cat -v
} > "$SUMMARY"

pass "TUI render evidence captured: $SUMMARY"
summary
