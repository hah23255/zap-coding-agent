//! In-session scheduler: data model and interval/time parsing.

use chrono::{Local, NaiveTime, TimeZone as _};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScheduleSpec {
    EveryInterval { interval_secs: u64 },
    OnceAt { time: String },
    DailyAt { time: String },
}

impl ScheduleSpec {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("daily ") {
            let time = parse_wallclock(rest.trim())?;
            return Some(Self::DailyAt {
                time: time.format("%H:%M").to_string(),
            });
        }
        if let Some(dur) = parse_interval(s) {
            return Some(Self::EveryInterval {
                interval_secs: dur.as_secs(),
            });
        }
        let time = parse_wallclock(s)?;
        Some(Self::OnceAt {
            time: time.format("%H:%M").to_string(),
        })
    }

    pub fn display(&self) -> String {
        match self {
            Self::EveryInterval { interval_secs } => format_interval_secs(*interval_secs),
            Self::OnceAt { time } => time.clone(),
            Self::DailyAt { time } => format!("daily {time}"),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::EveryInterval { interval_secs } => {
                format!("every {}", format_interval_secs(*interval_secs))
            }
            Self::OnceAt { time } => format!("at {time}"),
            Self::DailyAt { time } => format!("daily at {time}"),
        }
    }

    pub fn next_run_after(&self, now: chrono::DateTime<Local>) -> Option<chrono::DateTime<Local>> {
        match self {
            Self::EveryInterval { interval_secs } => {
                let secs = i64::try_from(*interval_secs).ok()?;
                Some(now + chrono::Duration::seconds(secs))
            }
            Self::OnceAt { time } => next_wallclock_run(now, parse_wallclock(time)?),
            Self::DailyAt { time } => next_wallclock_run(now, parse_wallclock(time)?),
        }
    }

    pub fn repeats(&self) -> bool {
        !matches!(self, Self::OnceAt { .. })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedScheduledJob {
    pub name: String,
    pub goal: String,
    pub spec: ScheduleSpec,
    #[serde(default)]
    pub fire_count: u32,
    #[serde(default)]
    pub last_run_at: Option<String>,
}

/// A registered scheduled job. Dropping the `handle` does NOT abort the task;
/// call `handle.abort()` explicitly when unscheduling.
pub struct ScheduledJob {
    /// Identifier used by /unschedule. Derived from the goal text if not given.
    pub name: String,
    /// Text submitted as a user turn each time the job fires.
    pub goal: String,
    /// Parsed schedule semantics.
    pub spec: ScheduleSpec,
    /// Background Tokio task. Abort to cancel.
    pub handle: tokio::task::JoinHandle<()>,
    /// How many times the job has fired this session.
    pub fire_count: u32,
    /// Most recent successful fire time in local time.
    pub last_run_at: Option<chrono::DateTime<Local>>,
}

impl ScheduledJob {
    pub fn persisted(&self) -> PersistedScheduledJob {
        PersistedScheduledJob {
            name: self.name.clone(),
            goal: self.goal.clone(),
            spec: self.spec.clone(),
            fire_count: self.fire_count,
            last_run_at: self.last_run_at.map(|dt| dt.to_rfc3339()),
        }
    }
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

pub fn next_wallclock_run(
    now: chrono::DateTime<Local>,
    wall: NaiveTime,
) -> Option<chrono::DateTime<Local>> {
    let today = now.date_naive();
    let today_dt = Local.from_local_datetime(&today.and_time(wall)).single()?;
    if today_dt > now {
        Some(today_dt)
    } else {
        let tomorrow = today.checked_add_days(chrono::Days::new(1))?;
        Local.from_local_datetime(&tomorrow.and_time(wall)).single()
    }
}

pub fn format_interval_secs(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    let mut out = String::new();
    if hours > 0 {
        out.push_str(&format!("{hours}h"));
    }
    if minutes > 0 {
        out.push_str(&format!("{minutes}m"));
    }
    if seconds > 0 {
        out.push_str(&format!("{seconds}s"));
    }
    out
}

pub fn format_run_time(dt: chrono::DateTime<Local>) -> String {
    dt.format("%Y-%m-%d %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn parse_minutes() {
        assert_eq!(parse_interval("30m"), Some(Duration::from_secs(1800)));
        assert_eq!(parse_interval("1m"), Some(Duration::from_secs(60)));
        assert_eq!(parse_interval("90m"), Some(Duration::from_secs(5400)));
    }

    #[test]
    fn parse_hours() {
        assert_eq!(parse_interval("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_interval("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_interval("1h30m"), Some(Duration::from_secs(5400)));
    }

    #[test]
    fn parse_seconds() {
        assert_eq!(parse_interval("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_interval("5s"), Some(Duration::from_secs(5)));
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert_eq!(parse_interval(""), None);
        assert_eq!(parse_interval("30"), None);
        assert_eq!(parse_interval("0m"), None);
        assert_eq!(parse_interval("1x"), None);
        assert_eq!(parse_interval("abc"), None);
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
        assert!(parse_wallclock("9:5").is_none());
    }

    #[test]
    fn schedule_spec_parses_expected_forms() {
        assert_eq!(
            ScheduleSpec::parse("30m"),
            Some(ScheduleSpec::EveryInterval { interval_secs: 1800 })
        );
        assert_eq!(
            ScheduleSpec::parse("17:30"),
            Some(ScheduleSpec::OnceAt {
                time: "17:30".to_string(),
            })
        );
        assert_eq!(
            ScheduleSpec::parse("daily 17:30"),
            Some(ScheduleSpec::DailyAt {
                time: "17:30".to_string(),
            })
        );
    }

    #[test]
    fn schedule_spec_labels_match_behavior() {
        assert_eq!(
            ScheduleSpec::EveryInterval { interval_secs: 1800 }.label(),
            "every 30m"
        );
        assert_eq!(
            ScheduleSpec::OnceAt {
                time: "17:30".to_string(),
            }
            .label(),
            "at 17:30"
        );
        assert_eq!(
            ScheduleSpec::DailyAt {
                time: "17:30".to_string(),
            }
            .label(),
            "daily at 17:30"
        );
    }

    #[test]
    fn format_interval_secs_round_trips_common_values() {
        assert_eq!(format_interval_secs(30), "30s");
        assert_eq!(format_interval_secs(1800), "30m");
        assert_eq!(format_interval_secs(5400), "1h30m");
    }
}
