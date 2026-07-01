#!/usr/bin/env bash
# T05 — Session persistence: context.md and session_log.md written at session end.
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

TMP=$(make_project)
trap "rm -rf $TMP" EXIT

info "T05: run a session with at least one real turn, then exit"
OUT=$(cd "$TMP" && printf '/exit\n' | timeout "$TIMEOUT" "$ZAP" --goal "list the files in src/ and say done" --auto --cli 2>&1) || true

info "T05a: context.md written after session"
if [ -f "$TMP/.zap/context.md" ]; then
    pass "T05a .zap/context.md created"
    info "  $(head -5 "$TMP/.zap/context.md")"
else
    fail "T05a .zap/context.md created" "file not found in $TMP/.zap/"
fi

info "T05b: session_log.md written after session"
if [ -f "$TMP/.zap/session_log.md" ]; then
    pass "T05b .zap/session_log.md created"
    info "  $(head -3 "$TMP/.zap/session_log.md")"
else
    fail "T05b .zap/session_log.md created" "file not found in $TMP/.zap/"
fi

info "T05c: context.md contains a timestamp line"
if [ -f "$TMP/.zap/context.md" ] && grep -qE "^[0-9]{4}-[0-9]{2}-[0-9]{2}" "$TMP/.zap/context.md"; then
    pass "T05c context.md has timestamp"
else
    fail "T05c context.md has timestamp" "$(head -10 "$TMP/.zap/context.md" 2>/dev/null)"
fi

info "T05d: session_log.md contains Next: line (requires LLM)"
if [ -z "${AGENT_API_KEY:-}" ] && [ -z "${ANTHROPIC_API_KEY:-}" ] && \
   ! curl -sf http://localhost:1234/v1/models >/dev/null 2>&1 && \
   ! curl -sf http://localhost:11434/api/tags >/dev/null 2>&1; then
    info "  T05d skipped — no LLM available"
elif [ -f "$TMP/.zap/session_log.md" ] && grep -qE "^Next:" "$TMP/.zap/session_log.md"; then
    pass "T05d session_log.md has Next: line"
else
    fail "T05d session_log.md has Next: line" "$(head -10 "$TMP/.zap/session_log.md" 2>/dev/null)"
fi

info "T06: second session shows Last: banner"
# Session 2 in same project dir — should see the prior context from session 1
OUT2=$(cd "$TMP" && printf '/exit\n' | timeout "$TIMEOUT" "$ZAP" \
  --goal "continue" --auto --cli 2>&1) || true
if echo "$OUT2" | grep -qi "Last:"; then
    pass "T06 second session shows Last: banner"
else
    fail "T06 second session shows Last: banner" "$(echo "$OUT2" | head -10)"
fi

info "T07: session_log accumulates entries across two sessions"
entry_count=$(grep -c "^## Session" "$TMP/.zap/session_log.md" 2>/dev/null || echo 0)
if [ "$entry_count" -ge 2 ]; then
    pass "T07 session_log has >= 2 entries ($entry_count found)"
else
    fail "T07 session_log has >= 2 entries" "only $entry_count entries: $(cat "$TMP/.zap/session_log.md" 2>/dev/null | head -5)"
fi

info "T08: CLI shows topic-shift nudge after 3 turns on different subject"
TMP_T08=$(make_project)
trap "rm -rf $TMP_T08" EXIT

# 3 turns about Rust/files, then 1 cooking question (>=40 chars, zero word overlap).
# The nudge fires at the start of the 4th handle_user_turn (before the LLM call),
# so T08 passes even when no LLM is configured.
OUT_T08=$(cd "$TMP_T08" && printf '%s\n' \
  "list all the files in the src directory please" \
  "show me the files in the src directory again" \
  "display those src directory files one final time" \
  "what ingredients do I need to make fresh pasta dough at home" \
  "/exit" | \
  timeout "$TIMEOUT" "$ZAP" --auto --cli 2>&1) || true

if echo "$OUT_T08" | grep -qi "new topic\|fork\|branch"; then
    pass "T08 CLI shows topic-shift nudge on subject change"
else
    fail "T08 CLI shows topic-shift nudge on subject change" \
         "$(echo "$OUT_T08" | grep -i "topic\|fork\|branch\|nudge" | head -5)"
fi

summary
