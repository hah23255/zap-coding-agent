//! In-session scheduler: data model and interval/time parsing.

/// A registered scheduled job. Dropping the `handle` does NOT abort the task;
/// call `handle.abort()` explicitly when unscheduling.
pub struct ScheduledJob {
    /// Identifier used by /unschedule. Derived from the goal text if not given.
    pub name:         String,
    /// Text submitted as a user turn each time the job fires.
    pub goal:         String,
    /// Human-readable schedule string for /schedule list ("30m", "1h", "17:30").
    pub interval_str: String,
    /// Background Tokio task. Abort to cancel.
    pub handle:       tokio::task::JoinHandle<()>,
    /// How many times the job has fired this session.
    pub fire_count:   u32,
}

/// Parse an interval string into a `std::time::Duration`.
/// Accepts: "30s", "5m", "2h", "1h30m", "90m".
/// Returns `None` for unrecognised formats.
pub fn parse_interval(s: &str) -> Option<std::time::Duration> {
    let s = s.trim().to_lowercase();
    let mut total_secs: u64 = 0;
    let mut num_buf   = String::new();
    let mut matched   = false;

    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num_buf.push(ch);
        } else {
            let n: u64 = num_buf.parse().ok()?;
            num_buf.clear();
            match ch {
                'h' => { total_secs += n * 3600; matched = true; }
                'm' => { total_secs += n * 60;   matched = true; }
                's' => { total_secs += n;         matched = true; }
                _   => return None,
            }
        }
    }
    if !num_buf.is_empty() { return None; } // trailing digits with no unit
    if !matched || total_secs == 0 { return None; }
    Some(std::time::Duration::from_secs(total_secs))
}

/// Parse a wall-clock time string "HH:MM" into a `chrono::NaiveTime`.
/// Returns `None` for anything that doesn't match "HH:MM" exactly
/// (both hours and minutes must be zero-padded to two digits).
pub fn parse_wallclock(s: &str) -> Option<chrono::NaiveTime> {
    let s = s.trim();
    // Strict format check: exactly "DD:DD" (5 chars, colon at index 2).
    if s.len() != 5 || s.as_bytes()[2] != b':' {
        return None;
    }
    if !s[..2].chars().all(|c| c.is_ascii_digit())
        || !s[3..].chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    chrono::NaiveTime::parse_from_str(s, "%H:%M").ok()
}

/// Given a schedule string, return a human label for display.
/// "30m" → "every 30m", "17:30" → "daily at 17:30"
pub fn schedule_label(s: &str) -> String {
    if parse_wallclock(s).is_some() {
        format!("daily at {s}")
    } else {
        format!("every {s}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn parse_minutes() {
        assert_eq!(parse_interval("30m"), Some(Duration::from_secs(1800)));
        assert_eq!(parse_interval("1m"),  Some(Duration::from_secs(60)));
        assert_eq!(parse_interval("90m"), Some(Duration::from_secs(5400)));
    }

    #[test]
    fn parse_hours() {
        assert_eq!(parse_interval("1h"),    Some(Duration::from_secs(3600)));
        assert_eq!(parse_interval("2h"),    Some(Duration::from_secs(7200)));
        assert_eq!(parse_interval("1h30m"), Some(Duration::from_secs(5400)));
    }

    #[test]
    fn parse_seconds() {
        assert_eq!(parse_interval("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_interval("5s"),  Some(Duration::from_secs(5)));
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert_eq!(parse_interval(""),      None);
        assert_eq!(parse_interval("30"),    None); // no unit
        assert_eq!(parse_interval("0m"),    None); // zero
        assert_eq!(parse_interval("1x"),    None); // bad unit
        assert_eq!(parse_interval("abc"),   None);
    }

    #[test]
    fn parse_wallclock_valid() {
        assert!(parse_wallclock("17:30").is_some());
        assert!(parse_wallclock("09:00").is_some());
        assert!(parse_wallclock("00:00").is_some());
    }

    #[test]
    fn parse_wallclock_invalid() {
        assert!(parse_wallclock("25:00").is_none());
        assert!(parse_wallclock("30m").is_none());
        assert!(parse_wallclock("9:5").is_none()); // no zero-padding
    }

    #[test]
    fn schedule_label_formats() {
        assert_eq!(schedule_label("30m"),   "every 30m");
        assert_eq!(schedule_label("1h"),    "every 1h");
        assert_eq!(schedule_label("17:30"), "daily at 17:30");
    }
}
