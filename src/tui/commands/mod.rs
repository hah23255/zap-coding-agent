/// Native TUI slash-command handlers.
///
/// Returns `Some(text)` for commands handled inline (text shown in the chat area).
/// Returns `None` for commands that need interactive terminal I/O — caller will
/// suspend the TUI, run the command, and show a "press Enter" prompt.
mod text;

use crate::config::{Config, PermissionMode};
use crate::session::Session;

/// Full list of slash commands shown in the command picker.
pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/help",              "show help"),
    ("/config",            "provider, model, URL"),
    ("/cost",              "token usage & estimated cost"),
    ("/history",           "message count"),
    ("/context",           "view and manage context — navigate turns, drop, compact, clear"),
    ("/clear",             "clear conversation history"),
    ("/compact",           "summarize and compress history"),
    ("/new",               "start a fresh session (clear history)"),
    ("/sessions",          "browse and resume old sessions"),
    ("/cd",                "change working directory"),
    ("/model",             "switch model for this session"),
    ("/models",            "list available models"),
    ("/provider",          "switch provider interactively"),
    ("/permissions",       "change permission mode"),
    ("/tasks",             "browse & execute task sessions"),
    ("/think",             "toggle extended thinking (on/off/N tokens)"),
    ("/attach",            "attach an image file to the next message"),
    ("/paste",             "attach an image from the clipboard to the next message"),
    ("/goal",              "run autonomously until condition met"),
    ("/index [quality]",   "reindex AST symbols · /index quality = health report"),
    ("/undo",              "undo last file edit"),
    ("/init",              "set up project (ZAP.md, index, project.json)"),
    ("/run",               "run a workflow"),
    ("/memory",            "manage memory entries"),
    ("/skill",             "list, use, show skills"),
    ("/audit",             "show last N audit log lines"),
    ("/hooks",             "list configured hooks"),
    ("/mcp",               "view/edit MCP server configs"),
    ("/remote [port]",     "start remote control — get a URL to code from anywhere"),
    ("/remote stop",       "stop the remote control server and tunnel"),
    ("/branch",            "create a conversation branch"),
    ("/branches",          "list conversation branches"),
    ("/switch",            "switch conversation branch"),
    ("/merge",             "merge a conversation branch"),
    ("/exit",              "quit zap"),
];

/// Return commands matching `input` (case-insensitive prefix).
///
/// - `/skill <tab>` → `/skill list|use|unuse|show|scope` sub-commands
/// - `/skill use <tab>` etc. → matching skill names from `skill_names`
/// - `/<skill-name>` → skill direct-run completions alongside built-ins
/// - otherwise → SLASH_COMMANDS filtered by prefix
pub fn filter_commands(input: &str, skill_names: &[String]) -> Vec<(String, String)> {
    let lower = input.to_lowercase();

    // /skill use|show|unuse <name> → dynamic skill-name completions
    for prefix in &["/skill use ", "/skill show ", "/skill unuse "] {
        if let Some(typed) = lower.strip_prefix(prefix) {
            return skill_names
                .iter()
                .filter(|n| n.to_lowercase().starts_with(typed))
                .map(|n| (format!("{}{}", prefix, n), format!("skill: {}", n)))
                .collect();
        }
    }

    // /skill<space> → sub-command completions
    if let Some(typed) = lower.strip_prefix("/skill ") {
        return [
            ("list",  "list all skills (built-in, global, project)"),
            ("use",   "pin skill — injected every turn"),
            ("unuse", "unpin a skill"),
            ("show",  "preview a skill's content"),
            ("scope", "manage domain scope"),
        ]
        .iter()
        .filter(|(sc, _)| sc.starts_with(typed))
        .map(|(sc, desc)| (format!("/skill {}", sc), desc.to_string()))
        .collect();
    }

    // Default: built-in commands filtered by prefix
    let mut results: Vec<(String, String)> = SLASH_COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.starts_with(lower.as_str()))
        .map(|(cmd, desc)| (cmd.to_string(), desc.to_string()))
        .collect();

    // Add skill names as direct /<skill-name> completions so the user can type
    // /read-jira directly without going through /skill use first.
    for name in skill_names {
        let slash = format!("/{}", name);
        if slash.to_lowercase().starts_with(lower.as_str()) && !is_builtin_command(name) {
            results.push((slash, format!("run skill: {}", name)));
        }
    }

    results
}

/// If `input` is `/<skill-name>` or `/<skill-name> <args>` where `skill-name`
/// is a known skill (not a built-in command), returns `skill_name`.
/// Returns `None` for all built-in commands and unknown names.
pub fn resolve_skill_command(input: &str, skill_names: &[String]) -> Option<String> {
    if !input.starts_with('/') { return None; }
    let stripped = &input[1..];
    let skill_part = stripped.split_whitespace().next().filter(|s| !s.is_empty())?;
    if is_builtin_command(skill_part) { return None; }
    skill_names.iter().find(|n| n.as_str() == skill_part).cloned()
}

/// Quick pre-filter: true if `input` could be a `/<skill-name>` command.
pub fn could_be_skill_command(input: &str) -> bool {
    let Some(rest) = input.strip_prefix('/') else { return false };
    let name = rest.split_whitespace().next().unwrap_or("");
    !name.is_empty() && !is_builtin_command(name)
}

/// True if `name` (without leading '/') matches a built-in slash command.
fn is_builtin_command(name: &str) -> bool {
    let slash = format!("/{}", name);
    SLASH_COMMANDS.iter().any(|(cmd, _)| {
        cmd.split_whitespace().next().unwrap_or(cmd) == slash.as_str()
    })
}

pub fn handle_inline(
    session: &mut Session,
    input: &str,
    config: &Config,
) -> Option<String> {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd  = parts[0];
    let arg  = parts.get(1).copied().unwrap_or("").trim();

    match cmd {
        "/help"    => Some(text::help_text()),
        "/config"  => Some(text::config_text(session)),
        "/cost"    => Some(text::cost_text(session)),
        "/history" => Some(format!(
            "{} messages in history  ·  {} turns this session",
            session.messages.len(), session.turn_count
        )),
        "/new" => {
            session.messages.clear();
            session.turn_count = 0;
            let cwd = crate::persistence::current_project_cwd();
            if let Ok(id) = session.store.save_session("(new session)", &session.config.model, &cwd) {
                session.session_id = id;
            }
            Some("New session started. History cleared.".to_string())
        }
        "/clear" => {
            session.messages.clear();
            Some("History cleared.".to_string())
        }
        "/model" if !arg.is_empty() => {
            session.model = arg.to_string();
            let mut nc = config.clone();
            nc.model = arg.to_string();
            session.client = crate::llm_client::create_client(&nc);
            Some(format!("Model switched to: {}", arg))
        }
        "/permissions" => {
            let new_mode = match arg.to_lowercase().as_str() {
                "ask"  => PermissionMode::Ask,
                "auto" => PermissionMode::Auto,
                "deny" => PermissionMode::Deny,
                _ => return Some("Usage: /permissions ask|auto|deny".to_string()),
            };
            session.permissions.mode = new_mode;
            Some(format!("Permission mode: {}", arg))
        }
        "/think" => {
            let budget = session.thinking_budget;
            match arg {
                "" | "status" => {
                    let status = if budget == 0 {
                        "off".to_string()
                    } else {
                        format!("{} token budget", budget)
                    };
                    Some(format!(
                        "Extended thinking: {}\nUsage: /think on|off|<tokens>  (Anthropic only; requires claude-3-7-sonnet+)",
                        status
                    ))
                }
                "off" | "0" => {
                    session.thinking_budget = 0;
                    Some("Extended thinking disabled.".to_string())
                }
                "on" => {
                    session.thinking_budget = 8000;
                    Some("Extended thinking enabled (8000 token budget). Requires claude-3-7-sonnet or newer.".to_string())
                }
                n => match n.parse::<u32>() {
                    Ok(0) => {
                        session.thinking_budget = 0;
                        Some("Extended thinking disabled.".to_string())
                    }
                    Ok(v) => {
                        session.thinking_budget = v;
                        Some(format!("Extended thinking enabled ({} token budget).", v))
                    }
                    Err(_) => Some("Usage: /think on|off|<budget_tokens>  e.g. /think 8000".to_string()),
                }
            }
        }
        "/cd" => {
            if arg.is_empty() {
                Some(format!(
                    "Usage: /cd <path>\nCurrent directory: {}",
                    std::env::current_dir().map(|p| p.display().to_string())
                        .unwrap_or_else(|_| "?".to_string())
                ))
            } else {
                match std::env::set_current_dir(arg) {
                    Ok(()) => Some(format!(
                        "Working directory: {}",
                        std::env::current_dir().map(|p| p.display().to_string())
                            .unwrap_or_else(|_| arg.to_string())
                    )),
                    Err(e) => Some(format!("cd: {}", e)),
                }
            }
        }
        "/attach" => {
            if !crate::llm_client::provider_supports_vision(&session.config) {
                return Some(format!(
                    "✗ {} does not support vision — image will not be sent.\n· Switch to Claude or GPT-4o to use images.",
                    session.config.base_url.as_deref().unwrap_or("this provider")
                ));
            }

            if arg.is_empty() {
                return Some("Usage: /attach <image-path>".to_string());
            }

            let mime = match std::path::Path::new(arg)
                .extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref()
            {
                Some("png") => "image/png",
                Some("jpg") | Some("jpeg") => "image/jpeg",
                Some("gif") => "image/gif",
                Some("webp") => "image/webp",
                _ => return Some("✗ Unsupported format. Use png / jpg / gif / webp.".to_string()),
            };

            match std::fs::read(arg) {
                Ok(bytes) => {
                    use base64::Engine;
                    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    let kb = bytes.len() / 1024;
                    session.staged_images.push((mime.to_string(), data));
                    Some(format!("✓ Attached {} ({} KB, {}) — it will be sent with your next message.", arg, kb, mime))
                }
                Err(e) => Some(format!("✗ Could not read '{}': {}", arg, e)),
            }
        }
        "/paste" => {
            if !crate::llm_client::provider_supports_vision(&session.config) {
                return Some(format!(
                    "✗ {} does not support vision — image will not be sent.\n· Switch to Claude or GPT-4o to use images.",
                    session.config.base_url.as_deref().unwrap_or("this provider")
                ));
            }

            #[cfg(windows)]
            let tmp = r"C:\Windows\Temp\zap_clipboard_paste.png";
            #[cfg(not(windows))]
            let tmp = "/tmp/zap_clipboard_paste.png";

            if crate::session::commands::paste_clipboard_image(tmp) && std::path::Path::new(tmp).exists() {
                match std::fs::read(tmp) {
                    Ok(bytes) => {
                        use base64::Engine;
                        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        let kb = bytes.len() / 1024;
                        session.staged_images.push(("image/png".to_string(), data));
                        Some(format!("✓ Image attached from clipboard ({} KB) — it will be sent with your next message.", kb))
                    }
                    Err(e) => Some(format!("✗ Failed to read clipboard image: {}", e)),
                }
            } else {
                Some("✗ No image in clipboard. Copy a screenshot first, then run /paste.\n· You can also use /attach <path> to stage a file directly.".to_string())
            }
        }
        "/skill" => Some(text::handle_skill_inline(session, arg)),
        "/remote" => {
            // Remote control is gated by a per-session token (see src/remote.rs):
            // the public URL carries `?token=…`, and the `/ws` upgrade is rejected
            // without it, so a leaked URL minus the token is inert. We also refuse
            // to start in Auto permission mode, where a leaked URL could drive the
            // shell unattended.
            if arg.trim() == "stop" {
                if crate::remote_channel::is_active() {
                    crate::remote_channel::deactivate();
                    Some("Remote control stopped.".to_string())
                } else {
                    Some("Remote control is not running.".to_string())
                }
            } else if matches!(session.config.permission_mode, crate::config::PermissionMode::Auto) {
                Some(
                    "⚠ /remote is refused in Auto permission mode — a leaked URL could \
                     drive the shell unattended. Switch to ask mode first \
                     (/permissions ask), then re-run /remote."
                        .to_string(),
                )
            } else {
                let port: u16 = arg.parse().unwrap_or(0);
                let token = crate::remote::generate_token();
                let token_for_task = token.clone();
                tokio::spawn(async move {
                    crate::remote_channel::activate();
                    match crate::remote::start_server(port, token_for_task.clone()).await {
                        Ok(actual_port) => {
                            crate::zap_warn!(
                                "⚡ remote server listening on http://127.0.0.1:{}/?token={}",
                                actual_port, token_for_task
                            );
                            match crate::remote::launch_tunnel(actual_port).await {
                                Ok(url) => {
                                    let base = url.trim_end_matches('/');
                                    crate::zap_warn!("🌐 remote URL: {}/?token={}", base, token_for_task);
                                    crate::zap_warn!("   Keep this URL secret — the token is the only access control. /remote stop to end.");
                                }
                                Err(e) => crate::zap_warn!(
                                    "tunnel failed: {} — use http://127.0.0.1:{}/?token={} on the same network",
                                    e, actual_port, token_for_task
                                ),
                            }
                        }
                        Err(e) => crate::zap_warn!("remote server failed: {}", e),
                    }
                });
                Some("⚡ Starting authenticated remote server… URL (with token) will appear in a moment.".to_string())
            }
        }
        "/index" if arg == "quality" || arg == "health" => Some(text::index_quality_text()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_top_level_commands_by_prefix() {
        let cmds = filter_commands("/he", &[]);
        assert!(cmds.iter().any(|(c, _)| c == "/help"));
        assert!(cmds.iter().all(|(c, _)| c.starts_with("/he")));
    }

    #[test]
    fn slash_alone_returns_all_commands() {
        let cmds = filter_commands("/", &[]);
        assert_eq!(cmds.len(), SLASH_COMMANDS.len());
    }

    #[test]
    fn image_commands_are_listed() {
        let cmds = filter_commands("/att", &[]);
        assert!(cmds.iter().any(|(c, _)| c == "/attach"));

        let cmds = filter_commands("/pas", &[]);
        assert!(cmds.iter().any(|(c, _)| c == "/paste"));
    }

    #[test]
    fn attach_stages_image_for_next_turn() {
        let mut path = std::env::temp_dir();
        path.push(format!("zap-test-{}.png", std::process::id()));
        std::fs::write(&path, b"not-real-png-but-extension-is-enough").unwrap();

        let config = crate::config::Config::default();
        let mut session = crate::session::Session::new_for_test(
            &config,
            Box::new(crate::llm_client::mock::MockClient::with_script(vec![])),
        ).unwrap();
        let response = handle_inline(&mut session, &format!("/attach {}", path.display()), &config)
            .expect("/attach should be handled inline");

        let _ = std::fs::remove_file(&path);

        assert!(response.contains("Attached"));
        assert_eq!(session.staged_images.len(), 1);
        assert_eq!(session.staged_images[0].0, "image/png");
        assert!(!session.staged_images[0].1.is_empty());
    }

    #[test]
    fn attach_reports_missing_file() {
        let config = crate::config::Config::default();
        let mut session = crate::session::Session::new_for_test(
            &config,
            Box::new(crate::llm_client::mock::MockClient::with_script(vec![])),
        ).unwrap();
        let response = handle_inline(&mut session, "/attach /tmp/zap-definitely-missing.png", &config)
            .expect("/attach should be handled inline");

        assert!(response.contains("Could not read"));
        assert!(session.staged_images.is_empty());
    }

    #[test]
    fn skill_space_shows_subcommands() {
        let cmds = filter_commands("/skill ", &[]);
        let names: Vec<&str> = cmds.iter().map(|(c, _)| c.as_str()).collect();
        assert!(names.contains(&"/skill list"));
        assert!(names.contains(&"/skill use"));
        assert!(names.contains(&"/skill show"));
        assert!(names.contains(&"/skill unuse"));
        assert!(names.contains(&"/skill scope"));
    }

    #[test]
    fn skill_use_space_shows_skill_names() {
        let skills = vec!["rust-expert".to_string(), "python-helper".to_string()];
        let cmds = filter_commands("/skill use ", &skills);
        let names: Vec<&str> = cmds.iter().map(|(c, _)| c.as_str()).collect();
        assert!(names.contains(&"/skill use rust-expert"));
        assert!(names.contains(&"/skill use python-helper"));
    }

    #[test]
    fn skill_use_filters_by_typed_prefix() {
        let skills = vec!["rust-expert".to_string(), "python-helper".to_string()];
        let cmds = filter_commands("/skill use ru", &skills);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].0, "/skill use rust-expert");
    }

    #[test]
    fn skill_show_and_unuse_also_complete_names() {
        let skills = vec!["myskill".to_string()];
        assert_eq!(filter_commands("/skill show ", &skills)[0].0, "/skill show myskill");
        assert_eq!(filter_commands("/skill unuse ", &skills)[0].0, "/skill unuse myskill");
    }

    #[test]
    fn filter_commands_no_skill_names_empty_completions() {
        let cmds = filter_commands("/skill use ", &[]);
        assert!(cmds.is_empty());
    }

    #[test]
    fn skill_subcommand_prefix_filtering() {
        let cmds = filter_commands("/skill l", &[]);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].0, "/skill list");
    }

    #[test]
    fn skill_names_appear_as_direct_slash_completions() {
        let skills = vec!["read-jira".to_string(), "review-pr".to_string()];
        let cmds = filter_commands("/read", &skills);
        assert!(cmds.iter().any(|(c, _)| c == "/read-jira"));
        assert!(cmds.iter().all(|(c, _)| c.starts_with("/read")));
    }

    #[test]
    fn skill_completions_dont_shadow_builtins() {
        let skills = vec!["help".to_string()];
        let cmds = filter_commands("/help", &skills);
        let help_entries: Vec<_> = cmds.iter().filter(|(c, _)| c == "/help").collect();
        assert_eq!(help_entries.len(), 1);
        assert_eq!(help_entries[0].1, "show help");
    }

    #[test]
    fn resolve_skill_command_exact_match() {
        let skills = vec!["read-jira".to_string()];
        assert_eq!(
            resolve_skill_command("/read-jira", &skills),
            Some("read-jira".to_string())
        );
        assert_eq!(
            resolve_skill_command("/read-jira PROJ-123", &skills),
            Some("read-jira".to_string())
        );
    }

    #[test]
    fn resolve_skill_command_no_match() {
        let skills = vec!["read-jira".to_string()];
        assert_eq!(resolve_skill_command("/unknown", &skills), None);
        assert_eq!(resolve_skill_command("no-slash", &skills), None);
    }

    #[test]
    fn resolve_skill_command_builtin_not_intercepted() {
        let skills = vec!["help".to_string(), "index".to_string()];
        assert_eq!(resolve_skill_command("/help", &skills), None);
        assert_eq!(resolve_skill_command("/index", &skills), None);
    }
}
