//! Handlers for the /schedule and /unschedule TUI slash commands.

use anyhow::Result;

use super::app::{App, MsgRole, UiBlock, UiMessage};

fn schedules_path() -> std::path::PathBuf {
    crate::project::zap_dir().join("scheduled_jobs.json")
}

pub(super) fn persist_jobs(app: &App) {
    let path = schedules_path();
    let jobs: Vec<crate::session::scheduler::PersistedScheduledJob> =
        app.scheduled_jobs.iter().map(|j| j.persisted()).collect();
    if let Ok(json) = serde_json::to_string_pretty(&jobs) {
        let _ = std::fs::write(path, json);
    }
}

fn load_jobs() -> Vec<crate::session::scheduler::PersistedScheduledJob> {
    let path = schedules_path();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn spawn_job(
    name: String,
    goal: String,
    spec: crate::session::scheduler::ScheduleSpec,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        match spec {
            crate::session::scheduler::ScheduleSpec::EveryInterval { interval_secs } => {
                let repeat = std::time::Duration::from_secs(interval_secs);
                loop {
                    tokio::time::sleep(repeat).await;
                    crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::ScheduledFire {
                        name: name.clone(),
                        goal: goal.clone(),
                    });
                }
            }
            crate::session::scheduler::ScheduleSpec::OnceAt { time } => {
                let wall = crate::session::scheduler::parse_wallclock(&time)
                    .expect("persisted once-at schedule should parse");
                if let Some(next) = crate::session::scheduler::next_wallclock_run(chrono::Local::now(), wall) {
                    if let Ok(until) = (next - chrono::Local::now()).to_std() {
                        tokio::time::sleep(until).await;
                        crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::ScheduledFire {
                            name,
                            goal,
                        });
                    }
                }
            }
            crate::session::scheduler::ScheduleSpec::DailyAt { time } => {
                let wall = crate::session::scheduler::parse_wallclock(&time)
                    .expect("persisted daily schedule should parse");
                loop {
                    let now = chrono::Local::now();
                    let Some(next) = crate::session::scheduler::next_wallclock_run(now, wall) else {
                        break;
                    };
                    let Ok(until) = (next - chrono::Local::now()).to_std() else {
                        continue;
                    };
                    tokio::time::sleep(until).await;
                    crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::ScheduledFire {
                        name: name.clone(),
                        goal: goal.clone(),
                    });
                }
            }
        }
    })
}

pub(super) fn load_persisted_schedules(app: &mut App) {
    for persisted in load_jobs() {
        let handle = spawn_job(
            persisted.name.clone(),
            persisted.goal.clone(),
            persisted.spec.clone(),
        );
        let last_run_at = persisted
            .last_run_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Local));
        app.scheduled_jobs.push(crate::session::scheduler::ScheduledJob {
            name: persisted.name,
            goal: persisted.goal,
            spec: persisted.spec,
            handle,
            fire_count: persisted.fire_count,
            last_run_at,
        });
    }
}

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
                               /schedule daily 17:30 generate EOD summary\n  \
                               /schedule 1h run security scan"
                        .to_string(),
                )],
            });
        } else {
            let now = chrono::Local::now();
            let lines: Vec<String> = app
                .scheduled_jobs
                .iter()
                .map(|j| {
                    let next = j
                        .spec
                        .next_run_after(now)
                        .map(crate::session::scheduler::format_run_time)
                        .unwrap_or_else(|| "unknown".to_string());
                    let last = j
                        .last_run_at
                        .map(crate::session::scheduler::format_run_time)
                        .unwrap_or_else(|| "never".to_string());
                    let mode = if j.spec.repeats() { "recurring" } else { "one-shot" };
                    format!(
                        "  \u{23f0} {} — {} — {} — next {} — last {} — fired {} time(s)",
                        j.name,
                        j.spec.label(),
                        mode,
                        next,
                        last,
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

    let (schedule_text, goal) = if let Some(rest) = arg.strip_prefix("daily ") {
        let mut parts = rest.splitn(2, ' ');
        let time = parts.next().unwrap_or("").trim();
        let goal = parts.next().unwrap_or("").trim();
        (format!("daily {time}"), goal)
    } else {
        let mut parts = arg.splitn(2, ' ');
        let schedule_text = parts.next().unwrap_or("").trim().to_string();
        let goal = parts.next().unwrap_or("").trim();
        (schedule_text, goal)
    };

    if goal.is_empty() {
        app.messages.push(UiMessage {
            role: MsgRole::Assistant,
            blocks: vec![UiBlock::Text(
                "Usage: /schedule <interval|HH:MM|daily HH:MM> <goal>\n  \
                 Examples: 30m, 1h, 2h30m, 17:30, daily 17:30"
                    .to_string(),
            )],
        });
        app.auto_scroll = true;
        return Ok(false);
    }

    let Some(spec) = crate::session::scheduler::ScheduleSpec::parse(&schedule_text) else {
        app.messages.push(UiMessage {
            role: MsgRole::Assistant,
            blocks: vec![UiBlock::Text(format!(
                "Unknown schedule '{schedule_text}'. Try: 30m, 1h, 2h30m, 17:30, or daily 17:30"
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

    let handle = spawn_job(name.clone(), goal.to_string(), spec.clone());

    let label = spec.label();
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
        spec,
        handle,
        fire_count: 0,
        last_run_at: None,
    });
    persist_jobs(app);
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
        persist_jobs(app);
        app.messages.push(UiMessage {
            role: MsgRole::Assistant,
            blocks: vec![UiBlock::Text(format!(
                "\u{23f0} Cancelled '{name}' (fired {} time(s)).",
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
