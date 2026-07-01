//! Per-turn model routing: classify input → swap client/model for one turn.
use colored::Colorize as _;

use super::Session;

/// Check `config.model_routes` for the classified task type.
/// If a different model is configured, swaps `session.model` and `session.client`
/// for the duration of the turn and returns `Some((original_model, original_client))`.
/// The caller must pass the returned value to `restore_routing` at every return point.
///
/// Returns `None` when no routing applies (no route, same model, or skip flag set).
pub fn route_for_turn(
    session: &mut Session,
    input: &str,
) -> Option<(String, Box<dyn crate::llm_client::LlmProvider>)> {
    if std::mem::replace(&mut session.skip_routing_once, false) {
        return None;
    }
    let task_type = crate::session::task_classifier::classify(input);
    if task_type == crate::session::task_classifier::TaskType::Default {
        return None;
    }
    let routed = session.config.model_routes.get(task_type.as_str()).cloned()?;
    if routed == session.model {
        return None;
    }
    let notice = format!(
        "  ◎ Routing {} task to {} (model_routes)",
        task_type.as_str(), routed
    );
    if crate::tui::channel::is_tui_mode() {
        crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::Notice(notice));
    } else {
        println!("{}", notice.truecolor(180, 175, 210));
    }
    let mut temp_config  = session.config.clone();
    temp_config.model    = routed.clone();
    let temp_client      = crate::llm_client::create_client(&temp_config);
    let orig_model       = std::mem::replace(&mut session.model,  routed);
    let orig_client      = std::mem::replace(&mut session.client, temp_client);
    Some((orig_model, orig_client))
}

/// Restore `session.model` and `session.client` if routing was active this turn.
/// Call at every return point in `handle_user_turn`.
pub fn restore_routing(
    session: &mut Session,
    save: &mut Option<(String, Box<dyn crate::llm_client::LlmProvider>)>,
) {
    if let Some((om, oc)) = save.take() {
        session.model  = om;
        session.client = oc;
    }
}
