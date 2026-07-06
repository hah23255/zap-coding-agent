#!/usr/bin/env bash
# T31 — Background agents: /bg and /agents are TUI-only commands.
# In CLI mode they fall through to Unknown command — test that behaviour is stable.
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

TMP=$(make_project)
trap "rm -rf $TMP" EXIT

info "T31a: /bg shows unknown command in CLI mode"
OUT=$(cd "$TMP" && printf '/bg refactor the auth middleware\n/exit\n' | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true
if echo "$OUT" | grep -qi "unknown command /bg"; then
    pass "T31a /bg shows unknown command in CLI mode"
else
    fail "T31a /bg shows unknown command in CLI mode" "$(echo "$OUT" | tail -10)"
fi

info "T31b: /agents shows unknown command in CLI mode"
OUT=$(cd "$TMP" && printf '/agents\n/exit\n' | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true
if echo "$OUT" | grep -qi "unknown command /agents"; then
    pass "T31b /agents shows unknown command in CLI mode"
else
    fail "T31b /agents shows unknown command in CLI mode" "$(echo "$OUT" | tail -10)"
fi

info "T31c: /agents kill <id> shows unknown command in CLI mode"
OUT=$(cd "$TMP" && printf '/agents kill 1\n/exit\n' | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true
if echo "$OUT" | grep -qi "unknown command /agents"; then
    pass "T31c /agents kill shows unknown command in CLI mode"
else
    fail "T31c /agents kill shows unknown command in CLI mode" "$(echo "$OUT" | tail -10)"
fi

summary
