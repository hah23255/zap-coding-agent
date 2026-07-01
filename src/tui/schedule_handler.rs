//! Handlers for the /schedule and /unschedule TUI slash commands.

use anyhow::Result;
use chrono::Timelike as _;

use super::app::{App, MsgRole, UiBlock, UiMessage};

/// Handle `/schedule` and `/schedule list` commands.
/// Returns `Ok(false)` always (no exit needed).
pub(super) fn handle_schedule(app: &mut App, cmd: &str) -> Result<bool> {
    let arg = cmd.strip_prefix("/schedule").unwrap_or("").trim();

    if arg.is_empty() || arg == "list" {
        if app.scheduled_jobs.is_empty() {
            app.messages.push(UiMessage {
                role: MsgRole::User,
                blocks: vec![UiBlock::Text(
                    "No scheduled jobs. Usage: /schedule <interval> <goal>\n  \
                     Examples: /schedule 30m fetch splunk insights\n  \
                               /schedule 17:30 generate EOD summary\n  \
                               /schedule 1h run security scan"
                        .to_string(),
                )],
            });
        } else {
            let lines: Vec<String> = app
                .scheduled_jobs
                .iter()
                .map(|j| {
                    format!(
                        "  \u{23f0} {} \u{2014} {} \u{2014} fired {} time(s)",
                        j.name,
                        crate::session::scheduler::schedule_label(&j.interval_str),
                        j.fire_count
                    )
                })
                .collect();
            app.messages.push(UiMessage {
                role: MsgRole::Assistant,
                blocks: vec![UiBlock::Text(format!(
                    "Scheduled jobs:\n{}",
                    lines.join("\n")
                ))],
            });
        }
        app.auto_scroll = true;
        return Ok(false);
    }

    // Parse: first token is interval, rest is goal.
    let mut parts = arg.splitn(2, ' ');
    let interval_str = parts.next().unwrap_or("").trim();
    let goal = parts.next().unwrap_or("").trim();

    if goal.is_empty() {
        app.messages.push(UiMessage {
            role: MsgRole::Assistant,
            blocks: vec![UiBlock::Text(
                "Usage: /schedule <interval> <goal>\n  \
                 Intervals: 30m, 1h, 2h30m, 17:30 (wall-clock daily)"
                    .to_string(),
            )],
        });
        app.auto_scroll = true;
        return Ok(false);
    }

    // Determine sleep duration — relative interval or wall-clock.
    let duration_result: Option<std::time::Duration> = {
        if let Some(dur) = crate::session::scheduler::parse_interval(interval_str) {
            Some(dur)
        } else if let Some(wall) = crate::session::scheduler::parse_wallclock(interval_str) {
            let now = chrono::Local::now().time();
            let secs: u64 = if now < wall {
                (wall - now).num_seconds().unsigned_abs()
            } else {
                // Past today → fire tomorrow.
                let until_midnight = (chrono::NaiveTime::from_hms_opt(23, 59, 59).unwrap() - now)
                    .num_seconds()
                    .unsigned_abs()
                    + 1;
                until_midnight
                    + wall.hour() as u64 * 3600
                    + wall.minute() as u64 * 60
                    + wall.second() as u64
            };
            Some(std::time::Duration::from_secs(secs))
        } else {
            None
        }
    };

    let Some(duration) = duration_result else {
        app.messages.push(UiMessage {
            role: MsgRole::Assistant,
            blocks: vec![UiBlock::Text(format!(
                "Unknown interval '{interval_str}'. Try: 30m, 1h, 2h30m, or 17:30"
            ))],
        });
        app.auto_scroll = true;
        return Ok(false);
    };

    // Derive a job name from the goal (first 20 chars, alphanumeric/dash only).
    let name: String = goal
        .chars()
        .take(20)
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string();

    // Reject duplicates — /unschedule uses the name as a key.
    if app.scheduled_jobs.iter().any(|j| j.name == name) {
        app.messages.push(UiMessage {
            role: MsgRole::Assistant,
            blocks: vec![UiBlock::Text(format!(
                "A job named '{name}' already exists. /unschedule {name} first."
            ))],
        });
        app.auto_scroll = true;
        return Ok(false);
    }

    // Spawn background task.
    let fire_name = name.clone();
    let fire_goal = goal.to_string();
    let fire_interval = crate::session::scheduler::parse_interval(interval_str);
    let interval_str_owned = interval_str.to_string();

    let handle = tokio::spawn(async move {
        tokio::time::sleep(duration).await;
        crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::ScheduledFire {
            name: fire_name.clone(),
            goal: fire_goal.clone(),
        });
        // For interval schedules, keep repeating.
        if let Some(repeat) = fire_interval {
            loop {
                tokio::time::sleep(repeat).await;
                crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::ScheduledFire {
                    name: fire_name.clone(),
                    goal: fire_goal.clone(),
                });
            }
        }
        // Wall-clock jobs fire once; user re-schedules for next occurrence.
    });

    let label = crate::session::scheduler::schedule_label(&interval_str_owned);
    app.messages.push(UiMessage {
        role: MsgRole::Assistant,
        blocks: vec![UiBlock::Text(format!(
            "\u{23f0} Scheduled '{name}' to run {label}.\n  /unschedule {name} to cancel  \u{00b7}  /schedule list to see all"
        ))],
    });
    app.auto_scroll = true;
    app.scheduled_jobs.push(crate::session::scheduler::ScheduledJob {
        name,
        goal: goal.to_string(),
        interval_str: interval_str_owned,
        handle,
        fire_count: 0,
    });
    Ok(false)
}

/// Handle `/unschedule <name>` command.
/// Returns `Ok(false)` always (no exit needed).
pub(super) fn handle_unschedule(app: &mut App, cmd: &str) -> Result<bool> {
    let name = cmd.strip_prefix("/unschedule").unwrap_or("").trim();
    if name.is_empty() {
        app.messages.push(UiMessage {
            role: MsgRole::Assistant,
            blocks: vec![UiBlock::Text(
                "Usage: /unschedule <name>  (use /schedule list to see names)".to_string(),
            )],
        });
        app.auto_scroll = true;
        return Ok(false);
    }
    if let Some(pos) = app.scheduled_jobs.iter().position(|j| j.name == name) {
        let job = app.scheduled_jobs.remove(pos);
        job.handle.abort();
        app.messages.push(UiMessage {
            role: MsgRole::Assistant,
            blocks: vec![UiBlock::Text(format!(
                "\u{23f0} Cancelled '{name}' (fired {} time(s) this session).",
                job.fire_count
            ))],
        });
    } else {
        app.messages.push(UiMessage {
            role: MsgRole::Assistant,
            blocks: vec![UiBlock::Text(format!(
                "No scheduled job named '{name}'. Use /schedule list to see active jobs."
            ))],
        });
    }
    app.auto_scroll = true;
    Ok(false)
}
