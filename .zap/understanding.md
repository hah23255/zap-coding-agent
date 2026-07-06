# Understanding
<!-- auto-updated by zap at session start — edit the Analysis section freely -->

<!-- zap:auto-stats:begin -->
## Project
ideas v0.15.130 · 329 files · 5097 symbols

### Languages
  - python: 3193 symbols
  - rust: 1894 symbols
  - javascript: 10 symbols

### Source modules
  bin, code_index, config, default_skills, llm_client, lsp, project, session, tools, tui, agent_core, audit, cli, context_manager, context_utils, domain_map, hooks, http, log, main, mcp, permission_manager, persistence, plan_execution, project, remote, remote_channel, secret_scanner, shell_runner, skill_installer, skill_manager, snapshot, stream_highlighter, task_planner, trust, ui, workflow

### Built-in skills
  27 skills in `src/default_skills/`

<!-- zap:auto-stats:end -->

## Analysis
<!-- Run `/init` for a detailed LLM-powered analysis of architecture, patterns, and key modules. -->

<!-- zap:domain-map:begin -->
## Domain Map

### Business Domains

| Domain | Owns | Key entry points |
|---|---|---|
| Agent orchestration and user modes | Main CLI/TUI/REPL/SDK/subagent execution flow, top-level startup, and routing into the interactive agent experience. | `src/main.rs::main`, `src/lib.rs::run`, `src/cli.rs::run`, `src/agent_core.rs::{run, run_tui, run_repl, run_sdk, run_subagent}` |
| LLM provider integration | Provider abstraction, request/response conversion, streaming/non-streaming calls, retries, auth, credentials, and provider-specific clients. | `src/llm_client/mod.rs`, `src/llm_client/{anthropic,openai,codex,claude_code,auth,credentials}.rs` |
| Tool execution and permissions | Built-in coding-agent tools, shell and filesystem actions, edit/search/code-navigation tools, approval prompts, and permission policy enforcement. | `src/tools/*`, `src/permission_manager.rs`, `src/shell_runner.rs`, `src/audit.rs` |
| Code intelligence and repository understanding | Multi-language code indexing, symbol extraction, call/import/type graphs, definition/reference lookup, context packing, and domain-map support. | `src/code_index/*`, `src/domain_map.rs` |
| Context assembly and project/session state | System prompt construction, project metadata, session logs, saved understanding/context files, persistent state, memory, and context utilities. | `src/context_manager.rs`, `src/context_utils.rs`, `src/project.rs`, `src/persistence.rs` |
| Configuration and runtime environment | Config loading/saving, provider and mode settings, HTTP client defaults, proxy/network summaries, logging, and runtime hooks. | `src/config.rs`, `src/http.rs`, `src/log.rs`, `src/hooks.rs` |
| MCP and external tool integrations | MCP server config, transport startup, tool discovery/wrapping, validation of external commands, and normalized MCP tool execution. | `src/mcp.rs` |
| Skills and workflow guidance | Built-in skill library and plan-execution instructions that influence agent behavior and task workflows. | `src/skill_manager/*`, `src/default_skills/`, `src/plan_execution.rs` |
| Remote access and collaboration | Websocket/browser-facing remote channel, remote server lifecycle, token handling, and tunnel launch helpers. | `src/remote.rs`, `src/remote_channel.rs` |
| Safety and security controls | Secret scanning, permission gates, audit records, redaction/log hygiene, command validation, and boundaries around side-effecting operations. | `src/secret_scanner.rs`, `src/permission_manager.rs`, `src/audit.rs`, `src/log.rs`, `src/mcp.rs::validate_mcp_command` |
| Evaluation and diagnostics | Offline evaluation harness and result reporting for agent task behavior. | `src/bin/evals.rs` |

### Cross-Cutting Concerns

- Permissions and safety span shell execution, filesystem mutation, MCP tools, remote operations, and the main agent loop.
- Configuration is consumed by provider selection, agent modes, permission behavior, hooks, network/HTTP behavior, MCP servers, logging, and persistence.
- Logging, auditing, and redaction are shared infrastructure for LLM calls, tool calls, shell commands, network setup, and session history.
- Code indexing is both a user-facing capability through tools and an internal dependency for prompt/context construction and project understanding.
- Persistence underlies project context, session state, memory, branch/session tracking, and understanding refreshes.
- Skills and hooks act as policy/adaptation layers around core execution rather than isolated product flows.
- Security boundaries are most important at edges: shell, filesystem writes, subprocess/MCP startup, HTTP/provider calls, and remote access.

### Dependency Direction

- Entry points (`main`, `lib`, `cli`, `agent_core`) sit at the outside and orchestrate inward-facing domains such as config, context, LLM clients, tools, permissions, persistence, skills, and remote/MCP adapters.
- Provider-specific LLM clients should depend on shared LLM abstractions plus HTTP/auth/config helpers, not on UI, CLI, or tool internals.
- Tool implementations should use permission, audit, logging, shell, and code-index helpers, while the code-index core should stay usable independently of the interactive agent loop.
- Context/project/persistence modules provide the state and prompt substrate; orchestration consumes them rather than duplicating storage or markdown-update logic.
- MCP, remote, hooks, and HTTP/provider integrations are edge adapters that should normalize external capabilities/events before they reach agent orchestration.
- Safety modules are intentionally cross-cutting and should be invoked before side effects reach shell, filesystem, network, subprocess, MCP, or remote boundaries.
<!-- zap:domain-map:end -->



























