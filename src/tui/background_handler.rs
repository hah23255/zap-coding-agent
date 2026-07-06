//! Handlers for the /bg and /agents TUI slash commands.

use anyhow::Result;

use super::app::{App, MsgRole, UiBlock, UiMessage};
use crate::config::Config;
use crate::session::background_agent::{self, BgStatus};

fn notice(app: &mut App, text: String) {
    app.messages.push(UiMessage { role: MsgRole::Assistant, blocks: vec![UiBlock::Text(text)] });
    app.auto_scroll = true;
}

fn elapsed_label(started_at: chrono::DateTime<chrono::Local>) -> String {
    let secs = (chrono::Local::now() - started_at).num_seconds().max(0) as u64;
    if secs == 0 {
        "0s".to_string()
    } else {
        crate::session::scheduler::format_interval_secs(secs)
    }
}

/// Handle `/bg <goal> [--model <slug>]`. Returns `Ok(false)` always (no exit needed).
pub(super) fn handle_bg(app: &mut App, cmd: &str, config: &Config) -> Result<bool> {
    let arg = cmd.strip_prefix("/bg").unwrap_or("").trim();
    if arg.is_empty() {
        notice(app, "Usage: /bg <goal> [--model <slug>]".to_string());
        return Ok(false);
    }

    let running = app.background_agents.iter()
        .filter(|a| matches!(a.status, BgStatus::Running))
        .count();
    if running >= config.max_background_agents {
        notice(app, format!(
            "✗ Cannot start: {running} background agents already running (max_background_agents = {})",
            config.max_background_agents
        ));
        return Ok(false);
    }

    let (goal, explicit_model) = background_agent::parse_bg_args(arg);
    if goal.is_empty() {
        notice(app, "Usage: /bg <goal> [--model <slug>]".to_string());
        return Ok(false);
    }

    app.next_bg_id += 1;
    let id = app.next_bg_id.to_string();
    let agent = background_agent::spawn(id.clone(), goal, explicit_model, config);
    let model = agent.model.clone();
    app.background_agents.push(agent);

    notice(app, format!(
        "Started agent {id} ({model}) — /agents to check. \
         ⚠ has known bugs: while it's actively streaming, it may briefly affect this \
         session's display (see FEATURES.md)."
    ));
    Ok(false)
}

/// Handle `/agents`, `/agents view <id>`, `/agents kill <id>`. Returns `Ok(false)` always.
pub(super) fn handle_agents(app: &mut App, cmd: &str) -> Result<bool> {
    let arg = cmd.strip_prefix("/agents").unwrap_or("").trim();

    if let Some(id) = arg.strip_prefix("view ") {
        let id = id.trim();
        let Some(agent) = app.background_agents.iter().find(|a| a.id == id) else {
            notice(app, format!("No background agent '{id}'. Use /agents to see active ones."));
            return Ok(false);
        };
        let text = match &agent.status {
            BgStatus::Running => format!(
                "{id} — running — {} elapsed — {}",
                elapsed_label(agent.started_at), agent.goal
            ),
            BgStatus::Done { summary, files_changed, turns, tool_calls } => format!(
                "{id} — done — {turns} turn(s), {tool_calls} tool call(s){}\n{summary}",
                if files_changed.is_empty() {
                    String::new()
                } else {
                    format!("\nfiles changed: {}", files_changed.join(", "))
                }
            ),
            BgStatus::Failed(err) => format!("{id} — failed — {err}"),
            BgStatus::Killed        => format!("{id} — killed — {}", agent.goal),
        };
        notice(app, text);
        return Ok(false);
    }

    if let Some(id) = arg.strip_prefix("kill ") {
        let id = id.trim();
        let Some(pos) = app.background_agents.iter().position(|a| a.id == id) else {
            notice(app, format!("No background agent '{id}'. Use /agents to see active ones."));
            return Ok(false);
        };
        if !matches!(app.background_agents[pos].status, BgStatus::Running) {
            notice(app, format!("Agent '{id}' is not running (status already final)."));
            return Ok(false);
        }
        app.background_agents[pos].handle.abort();
        app.background_agents[pos].status = BgStatus::Killed;
        notice(app, format!("Killed agent {id}."));
        return Ok(false);
    }

    if app.background_agents.is_empty() {
        notice(app, "No background agents. Usage: /bg <goal> [--model <slug>]".to_string());
        return Ok(false);
    }

    let mut lines = vec!["ID    MODEL               STATUS    ELAPSED  GOAL".to_string()];
    for a in &app.background_agents {
        let status = match &a.status {
            BgStatus::Running     => "running",
            BgStatus::Done { .. } => "done",
            BgStatus::Failed(_)   => "failed",
            BgStatus::Killed      => "killed",
        };
        lines.push(format!(
            "{:<5} {:<19} {:<9} {:<8} {}",
            a.id, a.model, status, elapsed_label(a.started_at), a.goal
        ));
    }
    notice(app, lines.join("\n"));
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_agent(id: &str, status: BgStatus) -> background_agent::BackgroundAgent {
        background_agent::BackgroundAgent {
            id: id.to_string(),
            goal: "existing task".to_string(),
            model: "test-model".to_string(),
            status,
            started_at: chrono::Local::now(),
            handle: tokio::spawn(async {}),
        }
    }

    fn last_notice(app: &App) -> String {
        let UiBlock::Text(text) = &app.messages.last().unwrap().blocks[0] else {
            panic!("expected text block");
        };
        text.clone()
    }

    #[tokio::test]
    #[allow(clippy::field_reassign_with_default)]
    async fn handle_bg_rejects_when_at_cap() {
        let mut app = App::new("test-model", "main");
        let mut config = Config::default();
        config.max_background_agents = 1;
        app.background_agents.push(dummy_agent("1", BgStatus::Running));

        handle_bg(&mut app, "/bg another task", &config).unwrap();

        assert_eq!(app.background_agents.len(), 1, "should not have spawned a second agent");
        assert!(last_notice(&app).contains("Cannot start"));
    }

    #[tokio::test]
    #[allow(clippy::field_reassign_with_default)]
    async fn handle_bg_allows_below_cap() {
        let mut app = App::new("test-model", "main");
        let mut config = Config::default();
        config.max_background_agents = 5;

        handle_bg(&mut app, "/bg write tests for the parser", &config).unwrap();

        assert_eq!(app.background_agents.len(), 1);
        assert!(last_notice(&app).contains("Started agent 1"));
    }

    #[tokio::test]
    async fn handle_agents_view_reports_unknown_id() {
        let mut app = App::new("test-model", "main");
        handle_agents(&mut app, "/agents view 99").unwrap();
        assert!(last_notice(&app).contains("No background agent '99'"));
    }

    #[tokio::test]
    async fn handle_agents_kill_aborts_running_agent() {
        let mut app = App::new("test-model", "main");
        app.background_agents.push(dummy_agent("1", BgStatus::Running));

        handle_agents(&mut app, "/agents kill 1").unwrap();

        assert!(matches!(app.background_agents[0].status, BgStatus::Killed));
        assert!(last_notice(&app).contains("Killed agent 1"));
    }

    #[tokio::test]
    async fn handle_agents_kill_rejects_already_finished_agent() {
        let mut app = App::new("test-model", "main");
        app.background_agents.push(dummy_agent("1", BgStatus::Done {
            summary: "done".to_string(), files_changed: vec![], turns: 1, tool_calls: 1,
        }));

        handle_agents(&mut app, "/agents kill 1").unwrap();

        assert!(last_notice(&app).contains("not running"));
    }
}
