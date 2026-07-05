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
