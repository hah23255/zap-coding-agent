/// Handlers for every InputAction variant — extracted from mod.rs to keep it
/// under the 600-line limit.  Returns `true` when the TUI loop should quit.
use std::io::Stdout;

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::app::{App, AppState, DiffPanel, MsgRole, UiBlock, UiMessage};
use super::channel::PermissionDecision;
use super::input::{self, InputAction};
use super::{file_browser, git_info, goal, lifecycle, render, startup, turn_handler};
use crate::config::Config;
use crate::session::Session;

/// Run a full code-index scan with a live animated status, instead of freezing
/// the TUI for the duration. Indexing is a full directory walk + tree-sitter
/// parse — slow enough on a large repo that running it inline (e.g. via
/// `tokio::task::block_in_place`) blocks the calling task, so the whole event
/// loop (redraws included) froze with no feedback until it finished. Spawning
/// it on a blocking task and ticking a redraw loop around it (same pattern
/// `run_normal_turn` uses for LLM calls) keeps the spinner animating instead.
/// Used by both `/init`'s wizard and the plain `/index` slash command.
pub(super) async fn run_indexing_with_spinner(
    app: &mut App,
    session: &Session,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<String> {
    app.state = AppState::ToolRunning { name: "index".to_string(), label: "codebase".to_string() };
    app.auto_scroll = true;
    terminal.draw(|frame| render::draw(frame, app))?;

    let code_index = session.code_index.clone();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let progress_cwd = cwd.clone();
    let mut handle = tokio::task::spawn_blocking(move || {
        crate::session::commands::run_init_indexing(&code_index, &cwd)
    });
    let mut last_progress = std::time::Instant::now();
    let result = loop {
        tokio::select! {
            r = &mut handle => break r.unwrap_or_else(|e| format!("Index error: {}", e)),
            _ = tokio::time::sleep(std::time::Duration::from_millis(16)) => {
                app.tick_spinner();
                if last_progress.elapsed() >= std::time::Duration::from_secs(10) {
                    last_progress = std::time::Instant::now();
                    if let Some((files, syms, calls)) = crate::code_index::peek_scan_progress(&progress_cwd) {
                        if let AppState::ToolRunning { label, .. } = &mut app.state {
                            *label = format!("codebase — {} files · {} syms · {} calls", files, syms, calls);
                        }
                    }
                }
                terminal.draw(|frame| render::draw(frame, app))?;
            }
        }
    };
    Ok(result)
}

pub(super) async fn handle_action(
    action: InputAction,
    app: &mut App,
    session: &mut Session,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    config: &Config,
) -> Result<bool> {
    match action {
        InputAction::Quit => return Ok(true),

        InputAction::Submit(text) => {
            let is_shift = session.turn_count >= 3
                && crate::session::is_topic_shift(&text, &session.messages);
            if is_shift {
                app.topic_shift_confirm = Some(text);
            } else {
                app.prompt_history.push(text.clone());
                app.history_idx = None;
                app.messages.push(UiMessage {
                    role: MsgRole::User,
                    blocks: vec![UiBlock::Text(text.clone())],
                });
                app.pending_input = Some(text);
            }
        }

        InputAction::TopicShiftSend => {
            if let Some(text) = app.topic_shift_confirm.take() {
                app.prompt_history.push(text.clone());
                app.history_idx = None;
                app.messages.push(UiMessage {
                    role: MsgRole::User,
                    blocks: vec![UiBlock::Text(text.clone())],
                });
                app.pending_input = Some(text);
            }
        }

        InputAction::TopicShiftBranch => {
            if let Some(text) = app.topic_shift_confirm.take() {
                app.input = text;
                app.cursor = app.input.chars().count();
            }
            app.pending_input = Some("/branch".to_string());
        }

        InputAction::TopicShiftCancel => {
            if let Some(text) = app.topic_shift_confirm.take() {
                app.input = text;
                app.cursor = app.input.chars().count();
            }
        }

        InputAction::Slash(cmd) => {
            app.messages.push(UiMessage {
                role: MsgRole::User,
                blocks: vec![UiBlock::Text(cmd.clone())],
            });
            app.pending_input = Some(cmd);
        }

        InputAction::Cancel => {
            // handle_action is only reached from the idle event loop, never from
            // run_normal_turn (which intercepts Ctrl+C before dispatching actions).
            // If we're somehow stuck in a non-Idle state here (e.g. a background
            // log WARN landed an LlmChunk that flipped state to Thinking with no
            // real turn running), reset so the user isn't trapped.
            if !matches!(app.state, AppState::Idle) {
                app.state = AppState::Idle;
                app.streaming_blocks.clear();
            }
        }

        InputAction::PasteText(text) => {
            if matches!(app.state, AppState::Idle) {
                input::insert_text_at_cursor(app, &text);
            }
        }

        InputAction::ScrollUp(n) => { app.scroll_up(n); }

        InputAction::ScrollDown(n) => {
            let sz = terminal.size()?;
            let sidebar_w: u16 = if sz.width > render::SIDEBAR_W + 20 { render::SIDEBAR_W + 1 } else { 0 };
            let chat_w = sz.width.saturating_sub(sidebar_w);
            let viewport_h = sz.height as usize;
            let total = app.total_lines(chat_w);
            app.scroll_down(n, viewport_h.saturating_sub(3), total);
        }

        InputAction::OpenDirPicker => {
            lifecycle::suspend_tui(terminal)?;
            let chosen = lifecycle::open_dir_picker();
            lifecycle::resume_tui(terminal)?;
            if let Some(dir) = chosen {
                match std::env::set_current_dir(&dir) {
                    Ok(()) => {
                        let new_cwd = std::env::current_dir()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|_| dir.clone());
                        if new_cwd != app.cwd {
                            let old = app.cwd.clone();
                            app.cwd = new_cwd.clone();
                            app.recent_dirs.insert(0, old);
                            app.recent_dirs.dedup();
                            app.recent_dirs.truncate(4);
                        }
                        app.branch = git_info::git_branch();
                        let (dirty, ahead, behind) = git_info::git_status();
                        app.git_dirty = dirty;
                        app.git_ahead = ahead;
                        app.git_behind = behind;
                        app.messages.push(UiMessage {
                            role: MsgRole::Assistant,
                            blocks: vec![UiBlock::Text(format!(
                                "Changed directory to: {}\n\nYou can now ask me about this codebase, or use /index to build a code symbol index.",
                                new_cwd
                            ))],
                        });
                        app.auto_scroll = true;
                    }
                    Err(e) => {
                        app.messages.push(UiMessage {
                            role: MsgRole::Assistant,
                            blocks: vec![UiBlock::Text(format!(
                                "Failed to change directory to: {}\nError: {}",
                                dir, e
                            ))],
                        });
                        app.auto_scroll = true;
                    }
                }
            }
        }

        InputAction::ToggleFileBrowser => {
            if app.file_browser.is_some() {
                app.file_browser = None;
            } else {
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                match file_browser::FileBrowser::new(cwd) {
                    Ok(browser) => {
                        app.file_browser = Some(browser);
                        if let Some(ref mut b) = app.file_browser { let _ = b.load_preview(); }
                    }
                    Err(e) => { app.error = Some(format!("Failed to open file browser: {}", e)); }
                }
            }
        }

        InputAction::LoadSession { id: sid, goal } => {
            startup::load_session_into_app(app, session, sid, goal);
        }

        InputAction::StartNewSession => {
            if let Ok(json) = serde_json::to_string(&session.messages) {
                let _ = session.store.save_messages(session.session_id, &json);
            }
            let cwd = crate::persistence::current_project_cwd();
            match session.store.save_session("(new session)", &session.model, &cwd) {
                Ok(new_id) => {
                    app.messages.clear();
                    app.streaming_blocks.clear();
                    app.input.clear();
                    app.cursor = 0;
                    app.scroll = 0;
                    app.auto_scroll = true;
                    app.turn = 0;
                    app.context_pct = 0;
                    app.tokens_input = 0;
                    app.tokens_output = 0;
                    app.tokens_cache_read = 0;
                    app.total_cost_usd = 0.0;
                    app.error = None;
                    app.picker_sel = 0;
                    app.expanded_tools.clear();
                    app.quit_confirm = false;
                    app.goal_state = None;
                    app.active_skill = None;
                    app.skill_history.clear();
                    session.session_id = new_id;
                    session.messages.clear();
                    session.turn_count = 0;
                    session.session_usage = crate::llm_client::Usage {
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                    };
                    session.files_changed.clear();
                    session.staged_images.clear();
                    session.skill_trace.clear();
                    session.compact_failures = 0;
                    app.messages.push(UiMessage {
                        role: MsgRole::Assistant,
                        blocks: vec![UiBlock::Text(format!("Started new session #{new_id}."))],
                    });
                }
                Err(e) => { app.error = Some(format!("Failed to create new session: {e}")); }
            }
        }

        InputAction::CloseSessionPicker => {}

        InputAction::DropLastTurn => {
            use crate::llm_client::ContentBlock;
            let drop_from = session.messages.iter().rposition(|m| {
                m.role == "user"
                    && m.content.first().is_some_and(|b| matches!(b, ContentBlock::Text { .. }))
            });
            if let Some(idx) = drop_from {
                session.messages.truncate(idx);
                session.turn_count = session.turn_count.saturating_sub(1);
                if matches!(app.messages.last().map(|m| &m.role), Some(MsgRole::Assistant)) {
                    app.messages.pop();
                }
                if matches!(app.messages.last().map(|m| &m.role), Some(MsgRole::User)) {
                    app.messages.pop();
                }
                if let Some(prev) = app.prompt_history.pop() {
                    app.input = prev;
                    app.cursor = app.input.chars().count();
                }
                app.context_pct = session.context_fill_pct();
                crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::Notice(
                    "↩ Last turn dropped — prompt restored to input.".to_string()
                ));
            }
        }

        InputAction::ContextViewerDrop => {
            if let Some(ref viewer) = app.context_viewer {
                if let Some(entry) = viewer.turns.get(viewer.selected) {
                    let start = entry.msg_index;
                    let end = start + entry.msg_count;
                    if end <= session.messages.len() {
                        session.messages.drain(start..end);
                    }
                }
            }
            app.context_viewer = Some(turn_handler::build_context_viewer(session));
            if let Some(ref mut v) = app.context_viewer {
                let max = v.turns.len().saturating_sub(1);
                v.selected = v.selected.min(max);
            }
            app.context_pct = session.context_fill_pct();
        }

        InputAction::ContextViewerCompact => {
            app.context_viewer = None;
            session.cmd_compact().await;
            app.context_pct = session.context_fill_pct();
        }

        InputAction::ContextViewerClearConfirm(confirmed) => {
            if confirmed {
                app.context_viewer = None;
                session.cmd_clear();
                app.context_pct = 0;
            } else if let Some(ref mut v) = app.context_viewer {
                v.confirm_clear = false;
            }
        }

        InputAction::SelectProvider(idx) => {
            lifecycle::handle_select_provider(app, config, idx);
        }

        InputAction::ClearInput => {}

        InputAction::SelectMode(is_task) => {
            if is_task {
                lifecycle::suspend_tui(terminal)?;
                let task_intro = lifecycle::run_task_planning_tui(session).await;
                lifecycle::resume_tui(terminal)?;
                if let Some(intro) = task_intro {
                    app.messages.push(UiMessage {
                        role: MsgRole::User,
                        blocks: vec![UiBlock::Text("Starting task session…".to_string())],
                    });
                    app.pending_input = Some(intro);
                }
            }
        }

        InputAction::ToggleLastToolExpand => {
            match goal::next_tool_id_to_expand(app) {
                Some(id) => { app.expanded_tools.insert(id); }
                None     => { app.expanded_tools.clear(); }
            }
        }

        InputAction::ConfirmDomainScope(names) => {
            session.domain_scope = names.into_iter().collect();
        }

        InputAction::ConfirmInit { language, do_index, do_understand } => {
            let languages: Vec<String> = language
                .split([',', ' '])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_lowercase)
                .collect();

            let index_section = if do_index {
                Some(run_indexing_with_spinner(app, session, terminal).await?)
            } else {
                None
            };

            app.state = AppState::Thinking;
            terminal.draw(|frame| render::draw(frame, app))?;
            let (output, llm_prompt) = session.cmd_init_direct(languages, index_section, do_understand);
            app.state = AppState::Idle;
            app.messages.push(UiMessage {
                role: MsgRole::Assistant,
                blocks: vec![UiBlock::Text(output)],
            });
            app.auto_scroll = true;
            if let Some(prompt) = llm_prompt {
                app.messages.push(UiMessage {
                    role: MsgRole::Assistant,
                    blocks: vec![UiBlock::Text(
                        "Analysing codebase to fill ZAP.md — this may take a minute…".to_string()
                    )],
                });
                app.pending_input = Some(prompt);
            }
        }

        InputAction::CancelInit => {}

        InputAction::PasteImage => {
            lifecycle::handle_paste_image(app, session, false);
        }

        InputAction::OpenDiffViewer => {
            app.diff_viewer = crate::tui::render::open_diff_viewer();
            if app.diff_viewer.is_none() {
                app.error = Some("No diff found (clean working tree and no previous commit)".to_string());
            }
        }

        InputAction::CloseDiffViewer => { app.diff_viewer = None; }
        InputAction::CloseCommandPopup => { app.command_popup = None; }

        InputAction::PermitAllow => {
            if let Some(ref mut popup) = app.permission_popup {
                if let Some(tx) = popup.response_tx.take() { let _ = tx.send(PermissionDecision::Allow); }
            }
            app.permission_popup = None;
        }
        InputAction::PermitDeny => {
            if let Some(ref mut popup) = app.permission_popup {
                if let Some(tx) = popup.response_tx.take() { let _ = tx.send(PermissionDecision::Deny); }
            }
            app.permission_popup = None;
        }
        InputAction::PermitAlways => {
            if let Some(ref mut popup) = app.permission_popup {
                if let Some(tx) = popup.response_tx.take() { let _ = tx.send(PermissionDecision::Always); }
            }
            app.permission_popup = None;
        }

        InputAction::CommandPopupScrollUp(n) => {
            if let Some(ref mut p) = app.command_popup { p.scroll = p.scroll.saturating_sub(n); }
        }
        InputAction::CommandPopupScrollDown(n) => {
            if let Some(ref mut p) = app.command_popup { p.scroll = p.scroll.saturating_add(n); }
        }

        InputAction::DiffNavUp => {
            if let Some(ref mut dv) = app.diff_viewer {
                if dv.panel == DiffPanel::Files && !dv.files.is_empty() {
                    dv.selected = dv.selected.saturating_sub(1);
                }
            }
        }
        InputAction::DiffNavDown => {
            if let Some(ref mut dv) = app.diff_viewer {
                if dv.panel == DiffPanel::Files {
                    dv.selected = dv.selected.saturating_add(1).min(dv.files.len().saturating_sub(1));
                }
            }
        }
        InputAction::DiffSwitchPanel => {
            if let Some(ref mut dv) = app.diff_viewer {
                dv.panel = match dv.panel {
                    DiffPanel::Files => DiffPanel::Diff,
                    DiffPanel::Diff  => DiffPanel::Files,
                };
            }
        }
        InputAction::DiffScrollUp(n) => {
            if let Some(ref mut dv) = app.diff_viewer {
                if dv.panel == DiffPanel::Diff { dv.diff_scroll = dv.diff_scroll.saturating_sub(n); }
            }
        }
        InputAction::DiffScrollDown(n) => {
            if let Some(ref mut dv) = app.diff_viewer {
                if dv.panel == DiffPanel::Diff { dv.diff_scroll = dv.diff_scroll.saturating_add(n); }
            }
        }

        InputAction::BtwOpen | InputAction::BtwClose | InputAction::BtwSubmit(_) => {}
        InputAction::None => {}
        InputAction::SecretAllow | InputAction::SecretDeny => {}

        InputAction::ApiKeyChar(c) => {
            if let Some(ref mut pending) = app.api_key_input { pending.input.push(c); }
        }
        InputAction::ApiKeyBackspace => {
            if let Some(ref mut pending) = app.api_key_input { pending.input.pop(); }
        }
        InputAction::ApiKeyCancel => { app.api_key_input = None; }
        InputAction::ApiKeyModelUp => {
            if let Some(ref mut p) = app.api_key_input { p.model_sel = p.model_sel.saturating_sub(1); }
        }
        InputAction::ApiKeyModelDown => {
            if let Some(ref mut p) = app.api_key_input {
                let max = p.models.len().saturating_sub(1);
                p.model_sel = (p.model_sel + 1).min(max);
            }
        }

        InputAction::ApiKeySubmit => {
            if let Some(ref mut pending) = app.api_key_input {
                if pending.picking_model {
                    let chosen = pending.models.get(pending.model_sel)
                        .filter(|m| m.as_str() != "Other…")
                        .cloned()
                        .unwrap_or_else(|| pending.models.first().cloned().unwrap_or_default());
                    let pending = app.api_key_input.take().unwrap();
                    let current_config = session.config.clone();
                    lifecycle::apply_provider_switch(
                        session, app, &current_config,
                        pending.slug, pending.name, chosen,
                        pending.kind_str, pending.provider,
                        pending.base_url, pending.auth_header,
                        pending.resolved_key,
                    );
                } else {
                    let typed = pending.input.trim().to_string();
                    let api_key = if typed.is_empty() {
                        if pending.has_existing_key {
                            session.config.all_providers.get(&pending.slug)
                                .and_then(|e| e.api_key.clone())
                                .filter(|k| !k.is_empty())
                        } else {
                            None
                        }
                    } else {
                        Some(typed)
                    };
                    pending.resolved_key = api_key;
                    pending.picking_model = true;
                    pending.model_sel = 0;
                }
            }
        }

        InputAction::CloseGeminiAuthPrompt => {
            app.gemini_auth_prompt = false;
            app.gemini_reauth = false;
        }
        InputAction::GeminiAuthApiKey => {
            lifecycle::handle_gemini_auth_apikey(app);
        }
        InputAction::LaunchGeminiAuth => {
            lifecycle::handle_gemini_auth_launch(terminal, session, app, config)?;
        }

        InputAction::OpenFilePicker => {
            let files = collect_files_for_picker();
            app.file_picker = Some(super::app::FilePickerState {
                query: String::new(),
                query_cursor: 0,
                all_files: files,
                selected: 0,
            });
        }

        InputAction::FilePickerSelect(path) => {
            // Insert `@path` at the current cursor position in the main input.
            input::insert_text_at_cursor(app, &format!("@{}", path));
        }

        InputAction::FilePickerClose => {}
    }

    Ok(false)
}

/// Walk the current working directory and collect relative file paths for the
/// file picker. Skips common build-output / VCS directories and hidden dirs
/// below the root level. Capped at depth 8 and 2 000 files so opening the
/// picker on a large mono-repo is instant.
fn collect_files_for_picker() -> Vec<String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let skip: &[&str] = &[
        ".git", "target", "node_modules", "__pycache__", ".next",
        "dist", "build", ".cache", ".venv", "venv", ".tox",
    ];
    let mut files: Vec<String> = Vec::new();
    collect_recursive(&cwd, &cwd, 0, skip, &mut files);
    files.sort();
    files
}

fn collect_recursive(
    base: &std::path::Path,
    dir: &std::path::Path,
    depth: usize,
    skip: &[&str],
    out: &mut Vec<String>,
) {
    if depth > 8 || out.len() >= 2_000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut sorted: Vec<_> = entries.flatten().collect();
    sorted.sort_by_key(|e| e.file_name());
    for entry in sorted {
        let name = entry.file_name().into_string().unwrap_or_default();
        if skip.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        // Skip hidden directories at any depth; still surface hidden files
        // (.gitignore, .env, .babelrc, etc.).
        if name.starts_with('.') && ft.is_dir() {
            continue;
        }
        if ft.is_dir() {
            collect_recursive(base, &path, depth + 1, skip, out);
        } else if ft.is_file() {
            if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_string_lossy().into_owned());
            }
        }
    }
}
