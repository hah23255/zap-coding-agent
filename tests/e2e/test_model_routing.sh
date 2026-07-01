#!/usr/bin/env bash
# T20 — Model routing: model_routes config routes tasks to configured models.
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

TMP=$(make_project)
trap "rm -rf $TMP" EXIT

info "T20: model_routes config causes routing notice for coding task"
# Write a config with model_routes that routes coding to a different model slug.
cat > "$TMP/.agent.toml" <<'EOF'
api_key = "test-key"
model = "claude-sonnet-4-6"
[model_routes]
coding = "claude-haiku-4-5-20251001"
EOF

OUT=$(cd "$TMP" && printf 'implement a hello function\n/exit\n' | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true

if echo "$OUT" | grep -qiE "routing.*coding|◎.*coding|Routing.*haiku"; then
    pass "T20 model routing notice shown for coding task"
else
    fail "T20 model routing notice shown for coding task" "$(echo "$OUT" | tail -20)"
fi

info "T20b: model_routes not triggered for non-matching task type"
OUT2=$(cd "$TMP" && printf 'hi there\n/exit\n' | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true

if echo "$OUT2" | grep -qiE "◎.*routing|Routing.*haiku"; then
    fail "T20b routing notice NOT shown for greeting" "$(echo "$OUT2" | tail -10)"
else
    pass "T20b routing notice not shown for non-matching input"
fi

summary
