#!/usr/bin/env bash
# T30 — In-session scheduler: /schedule and /unschedule are TUI-only commands.
# In CLI mode they fall through to Unknown command — test that behaviour is stable.
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

TMP=$(make_project)
trap "rm -rf $TMP" EXIT

info "T30a: /schedule with unknown interval shows unknown command in CLI mode"
OUT=$(cd "$TMP" && printf '/schedule 99x do something\n/exit\n' | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true
if echo "$OUT" | grep -qi "unknown command /schedule"; then
    pass "T30a /schedule unknown interval shows unknown command"
else
    fail "T30a /schedule unknown interval shows unknown command" "$(echo "$OUT" | tail -10)"
fi

info "T30b: /schedule list shows unknown command in CLI mode"
OUT=$(cd "$TMP" && printf '/schedule list\n/exit\n' | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true
if echo "$OUT" | grep -qi "unknown command /schedule"; then
    pass "T30b /schedule list shows unknown command in CLI mode"
else
    fail "T30b /schedule list shows unknown command in CLI mode" "$(echo "$OUT" | tail -10)"
fi

info "T30c: /unschedule shows unknown command in CLI mode"
OUT=$(cd "$TMP" && printf '/unschedule nonexistent\n/exit\n' | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true
if echo "$OUT" | grep -qi "unknown command /unschedule"; then
    pass "T30c /unschedule shows unknown command in CLI mode"
else
    fail "T30c /unschedule shows unknown command in CLI mode" "$(echo "$OUT" | tail -10)"
fi

summary
