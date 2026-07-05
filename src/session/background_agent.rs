//! Background agents: `/bg` spawns an independent sub-session as a detached
//! tokio task, tracked in `App.background_agents` so `/agents` can list,
//! view, and kill them. Reuses the same `Session` + `extract_result` plumbing
//! as the model-invoked `spawn_agent` tool (`agent_core::run_subagent`) — the
//! difference is this path is user-invoked and non-blocking.

use chrono::{DateTime, Local};

use crate::config::Config;
use crate::session::task_classifier;
use crate::tui::channel::BgOutcome;

pub struct BackgroundAgent {
    pub id: String,
    pub goal: String,
    pub model: String,
    pub status: BgStatus,
    pub started_at: DateTime<Local>,
    pub handle: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone)]
pub enum BgStatus {
    Running,
    Done { summary: String, files_changed: Vec<String>, turns: usize, tool_calls: usize },
    Failed(String),
    Killed,
}

impl From<BgOutcome> for BgStatus {
    fn from(o: BgOutcome) -> Self {
        match o {
            BgOutcome::Done { summary, files_changed, turns, tool_calls } =>
                BgStatus::Done { summary, files_changed, turns, tool_calls },
            BgOutcome::Failed(e) => BgStatus::Failed(e),
        }
    }
}

/// Resolve the model for a `/bg` task: an explicit `--model` always wins;
/// otherwise falls back to the same `task_classifier` + `model_routes` lookup
/// `session::routing::route_for_turn` uses for in-session turn routing; falls
/// back again to the caller's current default model.
pub fn resolve_bg_model(goal: &str, explicit: Option<&str>, config: &Config) -> String {
    if let Some(m) = explicit {
        return m.to_string();
    }
    let task_type = task_classifier::classify(goal);
    config.model_routes.get(task_type.as_str())
        .cloned()
        .unwrap_or_else(|| config.model.clone())
}

/// Parse `/bg <goal> [--model <slug>]` into `(goal, explicit_model)`.
pub fn parse_bg_args(arg: &str) -> (String, Option<String>) {
    if let Some(idx) = arg.find("--model ") {
        let goal  = arg[..idx].trim().to_string();
        let model = arg[idx + "--model ".len()..].trim().to_string();
        (goal, if model.is_empty() { None } else { Some(model) })
    } else {
        (arg.trim().to_string(), None)
    }
}

/// Spawn a background agent: builds an independent `Config`/`Session` for
/// `goal`, runs it to completion on a detached tokio task, and reports the
/// outcome via `TuiEvent::BackgroundAgentDone`. Returns the registry entry to
/// push into `App.background_agents` immediately — its `status` starts
/// `Running` and is updated later when the event arrives.
pub fn spawn(id: String, goal: String, explicit_model: Option<String>, config: &Config) -> BackgroundAgent {
    let model = resolve_bg_model(&goal, explicit_model.as_deref(), config);

    let mut sub_config = config.clone();
    sub_config.model               = model.clone();
    sub_config.is_subagent         = true;
    sub_config.is_background_agent = true;
    sub_config.agent_depth         = config.agent_depth.saturating_sub(1);
    sub_config.spawn_depth         = config.spawn_depth.saturating_add(1);
    sub_config.permission_mode     = crate::config::PermissionMode::Auto;

    let started_at = Local::now();
    let task_id    = id.clone();
    let task_goal  = goal.clone();
    let task_model = model.clone();

    let handle = tokio::spawn(async move {
        let run: anyhow::Result<crate::agent_core::SubagentResult> = async {
            let mut session = crate::session::Session::new(&sub_config).await?;
            session.handle_user_turn(&task_goal).await?;
            Ok(crate::agent_core::extract_result(&session))
        }.await;

        let outcome = match run {
            Ok(r) => BgOutcome::Done {
                summary:       r.summary,
                files_changed: r.files_changed,
                turns:         r.turns,
                tool_calls:    r.tool_calls,
            },
            Err(e) => BgOutcome::Failed(e.to_string()),
        };

        let elapsed_secs = (Local::now() - started_at).num_seconds().max(0) as u64;
        crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::BackgroundAgentDone {
            id: task_id,
            goal: task_goal,
            model: task_model,
            elapsed_secs,
            outcome,
        });
    });

    BackgroundAgent { id, goal, model, status: BgStatus::Running, started_at, handle }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config::default()
    }

    #[test]
    fn resolve_bg_model_explicit_wins() {
        let config = test_config();
        assert_eq!(
            resolve_bg_model("fix the bug", Some("codex/gpt-5.5"), &config),
            "codex/gpt-5.5"
        );
    }

    #[test]
    fn resolve_bg_model_falls_back_to_model_routes() {
        let mut config = test_config();
        config.model_routes.insert("coding".to_string(), "codex/gpt-5.5".to_string());
        assert_eq!(
            resolve_bg_model("fix the bug in auth.rs", None, &config),
            "codex/gpt-5.5"
        );
    }

    #[test]
    fn resolve_bg_model_falls_back_to_default_model() {
        let config = test_config(); // model_routes empty, model = "test-model"
        assert_eq!(resolve_bg_model("hi there", None, &config), "test-model");
    }

    #[test]
    fn parse_bg_args_splits_goal_and_model() {
        let (goal, model) = parse_bg_args("refactor the auth middleware --model codex/gpt-5.5");
        assert_eq!(goal, "refactor the auth middleware");
        assert_eq!(model, Some("codex/gpt-5.5".to_string()));
    }

    #[test]
    fn parse_bg_args_without_model_flag() {
        let (goal, model) = parse_bg_args("write tests for the parser");
        assert_eq!(goal, "write tests for the parser");
        assert_eq!(model, None);
    }
}
