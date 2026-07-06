/// Subscription usage-window tracking for Codex and Claude: publishes a live
/// number to the TUI sidebar on every check, and warns once per 5-hour/weekly
/// window when usage crosses `WARN_THRESHOLD_PCT`.
///
/// - **Codex**: real response headers (`x-codex-primary-used-percent`,
///   `x-codex-secondary-used-percent`), checked on every response.
/// - **Claude**: no official CLI flag or endpoint exists (anthropics/claude-code
///   issues #20399, #38380, #44328 are all open feature requests) — but
///   Anthropic's undocumented `/api/oauth/usage` endpoint returns the same data
///   Claude Code's own official `statusLine` feature exposes as
///   `rate_limits.five_hour` / `.seven_day` (see code.claude.com/docs/en/statusline).
///   Confirmed working live with a real Claude Code OAuth token.
///
/// The Claude endpoint is not documented or contracted by Anthropic — treat it
/// as best-effort: every failure mode (missing credentials, network error, 401,
/// schema drift) is swallowed silently so it can never delay or break an actual
/// coding turn, and re-checks are throttled so it's not hit on every message.
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const WARN_THRESHOLD_PCT: f32 = 80.0;

static CODEX_FIVE_HOUR_WARN:  AtomicU8 = AtomicU8::new(0);
static CODEX_SEVEN_DAY_WARN:  AtomicU8 = AtomicU8::new(0);
static CLAUDE_FIVE_HOUR_WARN: AtomicU8 = AtomicU8::new(0);
static CLAUDE_SEVEN_DAY_WARN: AtomicU8 = AtomicU8::new(0);

/// Inspect Codex's usage-percent headers, publish them to the sidebar, and
/// warn once per window if either crosses `WARN_THRESHOLD_PCT`.
pub fn check_codex_usage(headers: &reqwest::header::HeaderMap) {
    let five_hour = parse_pct_header(headers, "x-codex-primary-used-percent");
    let seven_day = parse_pct_header(headers, "x-codex-secondary-used-percent");

    publish_quota_update("codex", five_hour, seven_day, None);
    check_threshold(five_hour, "5-hour", &CODEX_FIVE_HOUR_WARN, "Codex", "claude_code");
    check_threshold(seven_day, "weekly", &CODEX_SEVEN_DAY_WARN, "Codex", "claude_code");
}

fn parse_pct_header(headers: &reqwest::header::HeaderMap, name: &str) -> Option<f32> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<f32>().ok())
}

/// Shared threshold logic for both providers: warn once per crossing, re-arm
/// once usage drops back under threshold (or the field goes missing — most
/// likely the window rolled over and the field just isn't reported yet).
fn check_threshold(
    pct: Option<f32>,
    window_label: &str,
    warned: &AtomicU8,
    provider_label: &str,
    switch_target: &str,
) {
    let Some(pct) = pct else {
        warned.store(0, Ordering::Relaxed);
        return;
    };
    if pct < WARN_THRESHOLD_PCT {
        warned.store(0, Ordering::Relaxed);
        return;
    }
    // swap(1) returns the previous value — only the first crossing warns.
    if warned.swap(1, Ordering::Relaxed) == 1 {
        return;
    }

    let msg = format!(
        "⚠ {provider_label} {window_label} usage at {pct:.0}% — switch providers now \
         (`/provider {switch_target}` or another) to avoid an interruption mid-task."
    );
    crate::zap_warn!("{}", msg);
    if crate::tui::channel::is_tui_mode() {
        crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::Warning(msg));
    }
}

/// Push a live reading to the sidebar (`App::quota_*` fields, drawn in
/// `tui::render::layout::draw_sidebar`). No-op outside TUI mode.
fn publish_quota_update(
    provider: &str,
    five_hour_pct: Option<f32>,
    seven_day_pct: Option<f32>,
    resets_at: Option<String>,
) {
    if crate::tui::channel::is_tui_mode() {
        crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::QuotaUpdate {
            provider: provider.to_string(),
            five_hour_pct,
            seven_day_pct,
            resets_at,
        });
    }
}

// ── Claude (Anthropic OAuth usage endpoint) ───────────────────────────────────

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_RECHECK_INTERVAL: Duration = Duration::from_secs(300);

static CLAUDE_LAST_CHECK: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

/// Called at the top of every `claude_code` turn. Naturally fires "at session
/// start" (first turn has no last-check yet) and re-checks every 5 minutes
/// during a long session, without needing separate startup wiring.
pub async fn check_claude_usage_if_stale() {
    {
        let cell = CLAUDE_LAST_CHECK.get_or_init(|| Mutex::new(None));
        let Ok(mut guard) = cell.lock() else { return };
        if guard.map(|last| last.elapsed() < CLAUDE_RECHECK_INTERVAL).unwrap_or(false) {
            return;
        }
        *guard = Some(Instant::now());
    }

    let Some(token) = claude_access_token() else { return };
    let Ok(resp) = crate::http::client()
        .get(CLAUDE_USAGE_URL)
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .timeout(Duration::from_secs(3))
        .send()
        .await
    else {
        return;
    };
    if !resp.status().is_success() {
        return;
    }
    let Ok(body) = resp.json::<serde_json::Value>().await else { return };

    let five_hour  = body["five_hour"]["utilization"].as_f64().map(|v| v as f32);
    let seven_day  = body["seven_day"]["utilization"].as_f64().map(|v| v as f32);
    let resets_at  = body["five_hour"]["resets_at"].as_str().map(|s| s.to_string());

    publish_quota_update("claude", five_hour, seven_day, resets_at);
    check_threshold(five_hour, "5-hour", &CLAUDE_FIVE_HOUR_WARN, "Claude", "codex");
    check_threshold(seven_day, "weekly", &CLAUDE_SEVEN_DAY_WARN, "Claude", "codex");
}

/// Reads Claude Code CLI's own OAuth access token — never a zap credential.
/// macOS stores it in Keychain; other platforms (and older Claude Code
/// versions) fall back to `~/.claude/.credentials.json`. Same shape either way:
/// `{"claudeAiOauth": {"accessToken": "..."}}`.
fn claude_access_token() -> Option<String> {
    if let Ok(tok) = std::env::var("CLAUDE_ACCESS_TOKEN") {
        if !tok.is_empty() {
            return Some(tok);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("security")
            .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
            .output()
        {
            if out.status.success() {
                if let Ok(raw) = String::from_utf8(out.stdout) {
                    if let Some(t) = extract_access_token(raw.trim()) {
                        return Some(t);
                    }
                }
            }
        }
    }

    let home = dirs::home_dir()?;
    let raw = std::fs::read_to_string(home.join(".claude/.credentials.json")).ok()?;
    extract_access_token(&raw)
}

fn extract_access_token(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    v["claudeAiOauth"]["accessToken"].as_str().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn headers_with(name: &str, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    #[test]
    fn parses_valid_and_rejects_missing_or_bad_headers() {
        assert_eq!(parse_pct_header(&headers_with("x-codex-primary-used-percent", "42.5"), "x-codex-primary-used-percent"), Some(42.5));
        assert_eq!(parse_pct_header(&HeaderMap::new(), "x-codex-primary-used-percent"), None);
        assert_eq!(parse_pct_header(&headers_with("x-codex-primary-used-percent", "not-a-number"), "x-codex-primary-used-percent"), None);
    }

    #[test]
    fn warns_once_per_window_then_rearms_after_reset() {
        let warned = AtomicU8::new(0);

        // Below threshold: no state change expected on repeated calls.
        check_threshold(Some(50.0), "5-hour", &warned, "Codex", "claude_code");
        assert_eq!(warned.load(Ordering::Relaxed), 0);

        // Crosses threshold: first call flips the flag.
        check_threshold(Some(85.0), "5-hour", &warned, "Codex", "claude_code");
        assert_eq!(warned.load(Ordering::Relaxed), 1);

        // Still high: flag stays set (no duplicate warning, verified by swap returning 1).
        check_threshold(Some(90.0), "5-hour", &warned, "Codex", "claude_code");
        assert_eq!(warned.load(Ordering::Relaxed), 1);

        // Window rolled over: flag clears, ready to warn again next time it crosses.
        check_threshold(Some(10.0), "5-hour", &warned, "Codex", "claude_code");
        assert_eq!(warned.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn missing_pct_is_a_silent_noop_and_clears_the_flag() {
        let warned = AtomicU8::new(1);
        check_threshold(None, "5-hour", &warned, "Codex", "claude_code");
        assert_eq!(warned.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn extracts_access_token_from_credentials_shape() {
        let raw = r#"{"claudeAiOauth":{"accessToken":"tok-abc123","refreshToken":"r"}}"#;
        assert_eq!(extract_access_token(raw), Some("tok-abc123".to_string()));
        assert_eq!(extract_access_token("not json"), None);
        assert_eq!(extract_access_token("{}"), None);
    }
}
