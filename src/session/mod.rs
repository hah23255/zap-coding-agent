pub mod commands;
pub mod routing;
pub mod scheduler;
pub mod task_classifier;
mod casual;
mod history;
mod memory_refresh;
mod preview;
mod summarizer;
mod tools;
mod turn;
mod watchdog;

#[cfg(test)]
mod agent_loop_tests;
#[cfg(test)]
mod test_factory;

pub use history::model_context_limit;
pub use casual::is_topic_shift;
pub use preview::preview_found_nothing;

use anyhow::Result;
use colored::Colorize;
use inquire::Confirm;
use std::sync::{Arc, Mutex};

use crate::{
    config::{default_context_window_for_provider, Config},
    context_manager,
    llm_client::{create_client, ContentBlock, LlmProvider, Message, Usage},
    permission_manager::PermissionManager,
    persistence,
    tools::{SpawnAgentTool, ToolRegistry},
    ui::ThinkingSpinner,
};

pub const MAX_TURNS: usize = 50;

pub fn configured_context_limit(config: &Config) -> usize {
    std::env::var("ZAP_MAX_CONTEXT_TOKENS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .or_else(|| config.budget.map(|b| b as usize))
        .or_else(|| {
            let entry = config.all_providers.get(&config.provider_slug);
            // Per-model context takes priority over provider-level context_window.
            let model_ctx = entry
                .and_then(|e| e.models.get(&config.model))
                .and_then(|m| m.context);
            model_ctx
                .or_else(|| entry.and_then(|e| e.context_window))
                .or_else(|| {
                    default_context_window_for_provider(
                        &config.provider_slug,
                        entry.and_then(|e| e.kind.as_deref()),
                    )
                })
        })
        .unwrap_or_else(|| {
            model_context_limit(&config.model)
        })
}

/// Load the previous session's messages, applying a context-size guard:
/// 1. `windowed_history` — cap to 8 user turns + prune oversized tool results
/// 2. Token budget — drop oldest user+assistant pairs until under 30% of the
///    model's context window (leaving 70% for new conversation + prompt + tools)
fn load_and_guard_previous_messages(
    store: &crate::persistence::Store,
    current_session_id: i64,
    cwd: &str,
    config: &Config,
) -> Vec<Message> {
    let Some(json) = store.load_previous_messages(current_session_id, cwd).ok().flatten() else {
        return Vec::new();
    };
    let prev: Vec<Message> = match serde_json::from_str(&json) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    // Step 1: apply the same sliding window used at runtime (last 8 user turns,
    // tool results outside the last 2 turns pruned to 150 chars).
    let mut windowed = history::windowed_history(&prev);

    // Step 2: token budget — cap at 30% of the model's context window.
    let budget = configured_context_limit(config) * 30 / 100;
    let mut tokens = Session::tokens_for_messages(&windowed);
    while tokens > budget && windowed.len() > 1 {
        // Find the oldest user-text message to drop it together with any
        // assistant response that immediately follows it.
        if let Some(idx) = windowed.iter().position(|m| {
            m.role == "user"
                && m.content
                    .first()
                    .is_some_and(|b| matches!(b, ContentBlock::Text { .. }))
        }) {
            let drop_end = if idx + 1 < windowed.len() && windowed[idx + 1].role == "assistant" {
                idx + 2 // user + assistant pair
            } else {
                idx + 1 // just the user message
            };
            if drop_end >= windowed.len() {
                break;
            }
            windowed.drain(0..drop_end);
        } else {
            break;
        }
        tokens = Session::tokens_for_messages(&windowed);
    }

    history::repair_tool_call_adjacency(&windowed)
}

// ── Edit ledger ────────────────────────────────────────────────────────────────

/// Per-file edit ledger entry, surviving sliding-window context eviction.
#[derive(Debug, Clone)]
pub struct EditedFile {
    pub first_turn: usize,
    pub last_turn:  usize,
    pub ops_count:  usize,
}

impl EditedFile {
    /// Summary line: "src/foo.rs (turns 3–7, 5 ops)" — typically ~30–40 chars.
    fn summary(&self, path: &str) -> String {
        if self.first_turn == self.last_turn {
            format!("{} (turn {}, {} op{})", path, self.first_turn, self.ops_count,
                if self.ops_count == 1 { "" } else { "s" })
        } else {
            format!("{} (turns {}–{}, {} op{})", path, self.first_turn, self.last_turn, self.ops_count,
                if self.ops_count == 1 { "" } else { "s" })
        }
    }
}

// ── Session ───────────────────────────────────────────────────────────────────

pub struct Session {
    pub client:        Box<dyn LlmProvider>,
    pub tools:         ToolRegistry,
    pub permissions:   PermissionManager,
    pub system:        String,
    pub tool_defs:     Vec<serde_json::Value>,
    pub messages:      Vec<Message>,
    pub model:         String,
    pub base_url:      Option<String>,
    pub session_usage: Usage,
    pub turn_count:    usize,
    pub tool_count:    usize,
    pub session_id:    i64,
    pub config:        Config,
    /// Images staged with /attach, sent with the next user turn then cleared.
    pub staged_images: Vec<(String, String)>,
    pub skills:        Vec<crate::skill_manager::Skill>,
    /// Names of Domain skills active this session. Empty = no restriction (all Domain candidates).
    pub domain_scope:  std::collections::HashSet<String>,
    /// Skills the user has explicitly pinned via `/skill use <name>` — injected every turn.
    pub pinned_skills: std::collections::HashSet<String>,
    pub current_branch: String,
    pub code_index:    Arc<Mutex<crate::code_index::CodeIndex>>,
    pub store:         persistence::Store,
    pub hooks:         crate::hooks::HookRunner,
    /// Extended thinking token budget. 0 = disabled. Anthropic only.
    pub thinking_budget: u32,
    /// Number of consecutive compact failures; gates auto-compact circuit breaker.
    pub compact_failures: u8,
    /// Files modified this session — survives sliding-window context eviction.
    pub edited_files:  std::collections::HashMap<String, EditedFile>,
    /// Legacy alias: deduplicated, sorted list derived from edited_files keys.
    pub files_changed: Vec<String>,
    /// Info lines shown at TUI startup (context banner, init nudge). Drained by run_tui().
    pub startup_notices: Vec<String>,
    /// Per-turn skill trace: (turn_number, input_preview, skill_names, reason_if_none).
    pub skill_trace: Vec<(usize, String, Vec<String>, Option<String>)>,
    /// LLM-generated summary of turns that have slid off the context window.
    /// Prepended as a synthetic message pair on every non-casual turn so the LLM
    /// retains decisions made before the window start.
    pub dropped_summary: String,
    /// Message index marking the start of the previous context window.
    /// Used to detect which turns newly slid off and need summarization.
    pub last_window_start: usize,
    /// Per-command failing verification state this turn (verify-aware progress
    /// watchdog). Keyed by normalized shell command → (fail count, round of
    /// last failed run). Cleared when that command passes and at each new
    /// user turn.
    pub verify_fails: std::collections::HashMap<String, (u32, usize)>,
    /// Current agentic round within this user turn (LLM call index).
    pub agentic_round: usize,
    /// A watchdog nudge has been issued this turn (suppresses repeats).
    pub watchdog_nudged: bool,
    /// Set when the watchdog escalates: tools are withdrawn for the rest of
    /// the turn so the model can only write its escalation summary.
    pub verify_escalated: bool,
    /// When true, skip model routing for the next turn only (user pressed [n] on
    /// the model-switch approval prompt). Cleared at the start of each turn.
    pub skip_routing_once: bool,
}

impl Session {
    pub async fn new(config: &Config) -> Result<Self> {
        crate::http::init(config);
        // additional_dirs (from --add-dir or ~/.agent.toml) also get write access.
        let all_write_roots: Vec<String> = config.allowed_paths.iter()
            .chain(config.additional_dirs.iter())
            .cloned()
            .collect();
        crate::tools::file::init_allowed_write_roots(&all_write_roots);
        crate::tools::clear_todos();
        let store = persistence::init()?;
        let cwd_str = persistence::current_project_cwd();
        // Subagents are internal helpers, not real top-level sessions — don't
        // persist a row for them (they were previously bloating ~/.zap/agent.db
        // with empty "(repl)" entries on every spawn_agent call). session_id=0
        // is a sentinel meaning "not persisted"; callers guard writes on it.
        let session_id = if config.is_subagent {
            0
        } else {
            store.save_session("(repl)", &config.model, &cwd_str)?
        };

        // SLM tier: use a compact ~400-token prompt and force the core tool profile.
        // is_slm_tier() checks explicit `tier = "slm"` / `tier = "qwen3_8b"` first,
        // then auto-detects from localhost URL + model name size suffix (≤13B).
        let slm_mode = crate::config::is_slm_tier(config);
        let qwen3_8b_mode = crate::config::is_qwen3_8b_tier(config);
        let mut config = config.clone();
        if slm_mode && config.tool_profile != "core" {
            config.tool_profile = "core".to_string();
        }
        let config = &config;
        let mut system = if slm_mode {
            context_manager::build_slm_system_prompt(config)?
        } else {
            context_manager::build_system_prompt(config)?
        };
        let mut tools = ToolRegistry::new(config.sandbox.clone());

        // MCP: load config into pending_mcp — servers connect on first use via mcp_connect tool.
        // Qwen3 8B tier skips MCP entirely to remove connector/schema overhead for that model.
        let mcp_cfg = crate::mcp::load_config();
        let mcp_had_config = mcp_cfg.had_config;
        let mcp_server_count = if qwen3_8b_mode { 0 } else { mcp_cfg.servers.len() };
        if !qwen3_8b_mode && mcp_server_count > 0 {
            tools.load_mcp_lazy(mcp_cfg);
        }

        if config.agent_depth > 0 {
            tools.register(std::sync::Arc::new(SpawnAgentTool::new(config.clone())));
        }

        let tool_defs  = tools.tool_definitions_filtered(&config.disabled_tools);
        let tool_count = tool_defs.len();

        let _bootstrapped = crate::skill_manager::bootstrap_bundled_skills();

        let skills    = crate::skill_manager::load_all_skills(&config.skill_paths);
        let always_on = crate::skill_manager::always_on_skills(&skills);

        let mut skill_warning: Option<String> = None;
        if !always_on.is_empty() {
            let (block, warning) = crate::skill_manager::assemble_always_on_block(
                &always_on,
                config.skill_token_budget,
            );
            if !block.is_empty() {
                system.push_str("\n\n");
                system.push_str(&block);
            }
            skill_warning = warning;
        }

        let project_meta = crate::project::load_project_meta();

        let detected = crate::skill_manager::detect_domain_scope(&skills);
        let domain_scope: std::collections::HashSet<String> = if let Some(ref meta) = project_meta {
            if !meta.language.is_empty() {
                meta.language.iter().cloned().collect()
            } else if !detected.is_empty() {
                detected.iter().cloned().collect()
            } else {
                crate::skill_manager::detect_from_extensions(&skills)
                    .into_iter().collect()
            }
        } else if !detected.is_empty() {
            detected.iter().cloned().collect()
        } else {
            crate::skill_manager::detect_from_extensions(&skills)
                .into_iter().collect()
        };

        let index_nudge: Option<String> = if config.is_subagent {
            None
        } else {
            match &project_meta {
                Some(meta) if !meta.indexed =>
                    Some("Run /index for fast symbol lookup. Run /init for full project context (LLM analysis).".into()),
                None =>
                    Some("Run /index for fast code symbol lookup.".into()),
                _ => None,
            }
        };

        if !skills.is_empty() && !config.is_subagent && !config.tui_mode {
            crate::skill_manager::print_repl_skill_summary(
                &skills, &always_on, &domain_scope, skill_warning.as_deref(),
            );
        }

        let hooks = crate::hooks::HookRunner::load();
        if !hooks.is_empty() && !config.is_subagent && !config.tui_mode {
            println!(
                "  {} {} hook(s) loaded",
                "◎".truecolor(255, 160, 80),
                hooks.total().to_string().cyan(),
            );
        }

        if mcp_server_count > 0 && !config.is_subagent && !config.tui_mode {
            let mut server_names: Vec<&str> = tools.pending_mcp_servers()
                .iter()
                .map(|(n, _)| *n)
                .collect();
            server_names.sort_unstable();
            println!(
                "  {} {} MCP server(s) available (lazy): {}",
                "⬡".truecolor(255, 140, 60),
                server_names.len().to_string().cyan(),
                server_names.join(", ").dimmed(),
            );
        } else if mcp_had_config && !config.is_subagent && !config.tui_mode {
            println!(
                "  {} {}",
                "○".truecolor(180, 120, 60),
                "MCP config found but no runnable stdio servers — all entries are disabled or use SSE/HTTP transport  (/mcp to edit)".truecolor(150, 120, 80),
            );
        }

        if !config.is_subagent && !config.tui_mode {
            if let Some(summary) = crate::http::network_summary(config) {
                println!(
                    "  {} {}",
                    "◎".truecolor(180, 180, 100),
                    summary.dimmed(),
                );
            }
        }

        let mut startup_notices: Vec<String> = Vec::new();
        if !config.additional_dirs.is_empty() && !config.is_subagent {
            let dirs = config.additional_dirs.iter()
                .map(|d| format!("  + {}", d))
                .collect::<Vec<_>>()
                .join("\n");
            startup_notices.push(format!("Additional directories (read/write):\n{}", dirs));
        }
        let mut messages: Vec<Message> = Vec::new();
        if !config.is_subagent {
            if let Some(summary) = crate::project::context_summary() {
                let files = crate::project::context_files();
                let files_part = if files.is_empty() {
                    String::new()
                } else {
                    format!("Files: {}", files.join(", "))
                };

                if config.tui_mode {
                    startup_notices.push(format!("↩ Last: {}", summary));
                    if !files_part.is_empty() {
                        startup_notices.push(format!("   {}", files_part));
                    }
                    if let Some(ctx) = crate::project::load_session_context() {
                        system.push_str("\n\n## Last Session Handoff\n");
                        system.push_str(&ctx);
                    }
                    if let Some(recent_next) = crate::project::load_recent_whats_next(3) {
                        system.push_str("\n\n## Recent What's Next (last 3 sessions)\n");
                        system.push_str(&recent_next);
                        system.push('\n');
                    }
                    // Restore full conversation history from the previous session.
                    messages = load_and_guard_previous_messages(&store, session_id, &cwd_str, config);
                } else {
                    println!("  {} Last: {}", "◌".dimmed(), summary.truecolor(180, 175, 210));
                    if !files_part.is_empty() {
                        println!("  {} {}", "◌".dimmed(), files_part.dimmed());
                    }
                    let is_tty = unsafe { libc::isatty(0 as libc::c_int) } != 0;
                    let resume = if is_tty {
                        Confirm::new("Resume from last session?")
                            .with_default(true)
                            .prompt()
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    if resume {
                        if let Some(ctx) = crate::project::load_session_context() {
                            system.push_str("\n\n## Last Session Handoff\n");
                            system.push_str(&ctx);
                        }
                        if let Some(recent_next) = crate::project::load_recent_whats_next(3) {
                            system.push_str("\n\n## Recent What's Next (last 3 sessions)\n");
                            system.push_str(&recent_next);
                            system.push('\n');
                        }
                        // Restore full conversation history from the previous session.
                        messages = load_and_guard_previous_messages(&store, session_id, &cwd_str, config);
                    }
                }
            }

            if let Some(nudge) = index_nudge {
                if config.tui_mode {
                    startup_notices.push(nudge);
                } else {
                    println!("  {} {}", "◌".dimmed(), nudge);
                }
            }
            if config.tui_mode {
                if let Some(ref warning) = skill_warning {
                    startup_notices.push(warning.clone());
                }
                let active_count = tools.active_tool_names().len();
                let disabled_count = config.disabled_tools.len();
                let tool_notice = if disabled_count > 0 {
                    format!(
                        "Tools: {} active ({} disabled) — /tools to list",
                        active_count.saturating_sub(disabled_count),
                        disabled_count
                    )
                } else {
                    format!("Tools: {} loaded — /tools to list all", active_count)
                };
                startup_notices.push(tool_notice);
            }
        }

        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let code_index = {
            // Only open the file-backed DB if it already exists — creating a new
            // SQLite database with WAL mode takes 200-500ms on macOS APFS and adds
            // noticeable latency to the first LLM turn. For new/unindexed projects,
            // start with an in-memory index; `cmd_index` upgrades to file-backed.
            let db_exists = cwd.join(".zap").join("code.db").exists();
            let idx = if db_exists {
                crate::code_index::CodeIndex::open(&cwd)
                    .unwrap_or_else(|_| crate::code_index::CodeIndex::open_in_memory()
                        .expect("SQLite in-memory always works"))
            } else {
                crate::code_index::CodeIndex::open_in_memory()
                    .expect("SQLite in-memory always works")
            };
            let arc = Arc::new(Mutex::new(idx));
            crate::code_index::set_global(arc.clone());

            // Initialize LSP manager with current working directory as root.
            // Language servers are spawned lazily on first tool use.
            let lsp_cwd = cwd.to_string_lossy().to_string();
            crate::lsp::set_global(crate::lsp::LspManager::new(lsp_cwd));

            arc
        };

        if !config.is_subagent {
            let (files, symbols, langs) = code_index.lock().ok().and_then(|guard| {
                let (f, s) = guard.total_stats().ok()?;
                let l = guard.stats_by_language().ok().unwrap_or_default();
                Some((f, s, l))
            }).unwrap_or_default();
            if files == 0 {
                let msg = "Code index is empty — run /index to enable find_definition and code_map tools.".to_string();
                if config.tui_mode {
                    startup_notices.push(msg);
                } else {
                    println!("  {} {}", "◌".dimmed(), msg);
                }
            }
            let cwd_name = cwd.file_name().map(|n| n.to_string_lossy().to_string());
            if let Err(e) = crate::project::refresh_understanding_md(cwd_name, files, symbols, &langs) {
                crate::log::write("WARN ", &format!("could not refresh understanding.md: {e}"));
            }
        }

        if !config.is_subagent {
            // Only auto-index projects that have been explicitly /init'd.
            // Prevents accidentally indexing C:\, /, or other system directories.
            let is_indexed = crate::project::load_project_meta()
                .map(|m| m.indexed)
                .unwrap_or(false);
            if is_indexed {
                crate::code_index::spawn_background_indexer(cwd.clone());
            }
        }

        Ok(Self {
            client: create_client(config),
            tools,
            permissions: PermissionManager::new(config.permission_mode.clone()),
            system,
            tool_defs,
            messages,
            model: config.model.clone(),
            base_url: config.base_url.clone(),
            session_usage: Usage::default(),
            turn_count: 0,
            tool_count,
            session_id,
            config: config.clone(),
            staged_images: Vec::new(),
            skills,
            domain_scope,
            pinned_skills: std::collections::HashSet::new(),
            current_branch: "main".to_string(),
            code_index,
            store,
            hooks,
            thinking_budget: 0,
            compact_failures: 0,
            files_changed: Vec::new(),
            edited_files: std::collections::HashMap::new(),
            startup_notices,
            skill_trace: Vec::new(),
            dropped_summary: String::new(),
            last_window_start: 0,
            verify_fails: std::collections::HashMap::new(),
            agentic_round: 0,
            watchdog_nudged: false,
            verify_escalated: false,
            skip_routing_once: false,
        })
    }

    pub fn make_spinner() -> ThinkingSpinner { ThinkingSpinner::new() }

    pub fn estimated_context_tokens(&self) -> usize {
        Self::tokens_for_messages(&self.messages)
    }

    fn tokens_for_messages(messages: &[Message]) -> usize {
        let chars: usize = messages.iter().map(|m| {
            m.content.iter().map(|b| match b {
                ContentBlock::Text { text }              => text.len(),
                ContentBlock::ToolUse { input, .. }      => input.to_string().len(),
                ContentBlock::ToolResult { content, .. } => content.len(),
                ContentBlock::Image { data, .. }         => data.len() / 4,
                ContentBlock::Thinking { thinking, .. }  => thinking.len() / 4,
                ContentBlock::Reasoning { content, .. }  => content.len() / 4,
            }).sum::<usize>()
        }).sum();
        chars / 4
    }

    pub fn context_fill_pct(&self) -> u8 {
        let effective = history::windowed_history(&self.messages);
        // Include dropped_summary — it's prepended on every non-casual turn once
        // older history slides out of the window, so it's part of what's actually
        // sent. Without it this number could read lower than the same basis used
        // by the auto-compact trigger, which is exactly the kind of mismatch that
        // made compact fire at a percentage the user never saw on screen.
        let tokens = Self::tokens_for_messages(&effective) + self.dropped_summary.len() / 4;
        let limit = configured_context_limit(&self.config);
        ((tokens * 100) / limit).min(100) as u8
    }

    /// Same as `context_fill_pct` but accepts a pre-computed token count (e.g. including
    /// projected skill tokens) so the compaction check can be skill-aware.
    pub fn context_fill_pct_with(&self, tokens: usize) -> u8 {
        let limit = configured_context_limit(&self.config);
        ((tokens * 100) / limit).min(100) as u8
    }

    // ── Slash dispatcher ──────────────────────────────────────────────────────

    /// Returns true if the session should end.
    pub async fn handle_slash(&mut self, line: &str, config: &Config) -> bool {
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        let cmd = parts[0];
        let arg = parts.get(1).copied().unwrap_or("").trim();

        match cmd {
            "/help"        => self.cmd_help(),
            "/config"      => self.cmd_config(),
            "/history"     => self.cmd_history(),
            "/clear"       => self.cmd_clear(),
            "/cost"        => self.cmd_cost(),
            "/models"      => self.cmd_models().await,
            "/sessions"    => self.cmd_sessions(arg),
            "/provider"    => self.cmd_provider(config),
            "/memory"      => self.cmd_memory(arg),
            "/audit"       => self.cmd_audit(arg),
            "/compact"     => { self.cmd_compact().await; }
            "/attach"      => self.cmd_attach(arg),
            "/paste"       => self.cmd_paste(),
            "/skill"       => self.cmd_skill(arg).await,
            "/run"         => {
                if arg.is_empty() {
                    let workflows = crate::workflow::discover_workflows();
                    if workflows.is_empty() {
                        println!("  No workflows found. Create .zap/workflows/<name>.yaml");
                    } else {
                        println!("  Available workflows:");
                        for (name, _) in &workflows { println!("    {} {}", "◌".dimmed(), name.cyan()); }
                        println!("  Run with: {}", "/run <name>".dimmed());
                    }
                } else if let Err(e) = self.cmd_run_workflow(arg).await {
                    println!("  {} workflow error: {}", "✗".red(), e);
                }
            }
            "/workflow"    => {
                if arg.starts_with("new ") || arg.starts_with("new\t") {
                    let name = arg[4..].trim();
                    if name.is_empty() {
                        println!("  usage: /workflow new <name>");
                    } else {
                        match crate::workflow::scaffold_workflow(name) {
                            Ok(p)  => println!("  {} created {}", "✓".green(), p.display().to_string().cyan()),
                            Err(e) => println!("  {} {}", "✗".red(), e),
                        }
                    }
                } else {
                    println!("  usage: /workflow new <name>   create a workflow scaffold");
                }
            }
            "/hooks"       => crate::hooks::print_hooks_list(&self.hooks),
            "/tools" | "/tool" => self.cmd_tools(),
            "/mcp"         => self.cmd_mcp(arg),
            "/remote"      => {
                // Token-gated (see src/remote.rs); refused in Auto mode where a
                // leaked URL could drive the shell unattended.
                if matches!(self.config.permission_mode, crate::config::PermissionMode::Auto) {
                    println!("  {} /remote is refused in Auto permission mode — switch to ask mode first (/permissions ask).", "⚠".yellow());
                } else {
                    let port: u16 = arg.parse().unwrap_or(0);
                    let token = crate::remote::generate_token();
                    crate::remote_channel::activate();
                    match crate::remote::start_server(port, token.clone()).await {
                        Ok(actual_port) => {
                            println!("  {} remote server on http://127.0.0.1:{}/?token={}", "⚡".bright_yellow(), actual_port, token);
                            match crate::remote::launch_tunnel(actual_port).await {
                                Ok(url) => {
                                    let base = url.trim_end_matches('/');
                                    let remote_url = format!("{}/?token={}", base, token);
                                    crate::remote::copy_to_clipboard(&remote_url);
                                    println!("  {} {}", "🌐".truecolor(100, 200, 255), remote_url.cyan().bold());
                                    println!("     Copied to clipboard. Keep this URL secret — the token is the only access control.");
                                }
                                Err(e) => println!("  {} tunnel failed: {} — use http://127.0.0.1:{}/?token={} on same network", "⚠".yellow(), e, actual_port, token),
                            }
                        }
                        Err(e) => println!("  {} {}", "✗".red(), e),
                    }
                }
            }
            "/tasks"       => self.cmd_tasks().await,
            "/think"       => self.cmd_think(arg),
            "/index"       => self.cmd_index(arg),
            "/branch"      => self.cmd_branch(arg).await,
            "/branches"    => self.cmd_branches(),
            "/switch"      => self.cmd_switch(arg).await,
            "/merge"       => self.cmd_merge(arg).await,
            "/undo"        => {
                let path = if arg.is_empty() { "list" } else { arg };
                if path == "list" {
                    let snaps = crate::snapshot::list_snapshots();
                    if snaps.is_empty() {
                        println!("  No undo history this session.");
                    } else {
                        println!("  Undo available:");
                        for s in snaps { println!("    {}", s.cyan()); }
                    }
                } else {
                    match crate::snapshot::restore_snapshot(path) {
                        Ok(content) => println!("  {} Reverted '{}' ({} bytes)", "✓".green(), path.cyan(), content.len()),
                        Err(e)      => println!("  {} {}", "✗".red(), e),
                    }
                }
            }
            "/init" => {
                if let Some(prompt) = self.cmd_init() {
                    if let Err(e) = self.handle_user_turn(&prompt).await {
                        println!("  {} agent error: {}", "✗".red(), e);
                    }
                }
            }
            "/understand" => {
                if let Some(prompt) = self.cmd_understand() {
                    if let Err(e) = self.handle_user_turn(&prompt).await {
                        println!("  {} agent error: {}", "✗".red(), e);
                    } else {
                        crate::project::mark_domain_map_current();
                        // Trim trailing blank lines the LLM may have written.
                        let upath = crate::project::zap_dir().join("understanding.md");
                        if let Ok(content) = std::fs::read_to_string(&upath) {
                            let trimmed = content.trim_end_matches('\n').to_string() + "\n";
                            let _ = std::fs::write(&upath, trimmed);
                        }
                    }
                }
            }
            "/permissions" => self.cmd_permissions(arg),
            "/model"       => {
                if arg.is_empty() { println!("  Usage: /model <model-id>"); }
                else              { self.cmd_model(arg, config); }
            }
            "/deploy"      => self.cmd_deploy(arg).await,
            "/exit" | "/quit" => return true,
            other => println!("  {} Unknown command {}. Try {}.",
                "✗".red(), other.yellow(), "/help".cyan()),
        }
        false
    }
}
