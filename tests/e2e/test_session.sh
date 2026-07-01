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

summary
