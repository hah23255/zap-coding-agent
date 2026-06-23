# pi vs zap — Comparison

**pi** ([pi.dev](https://pi.dev), [earendil-works/pi](https://github.com/earendil-works/pi)) is an open-source MIT-licensed terminal coding agent written in TypeScript/Node.js by Mario Zechner (creator of libGDX), distributed via npm/pnpm/bun. Its philosophy is a minimal harness (agent loop, 4 core tools, sessions) with everything else — permissions, sub-agents, MCP, plan mode — added as community TypeScript npm extensions.

Comparison done 2026-06-23 against zap v0.15.67.

---

## Side-by-side

| Area | pi | zap |
|---|---|---|
| **Language / runtime** | TypeScript / Node.js | Rust — single binary, no runtime |
| **Install** | npm/pnpm/bun or curl | curl script → native binary |
| **Default tools** | 4: read, write, edit, bash | ~18: file ops, shell, search_code, code_map, find_definition, find_references, who_calls, ripple_analysis, get_diagnostics, lsp_definition, lsp_type_at, memory, web_fetch, glob_read |
| **Code intelligence** | None built-in (grep/LLM only) | SQLite AST index (tree-sitter, 7 languages), call-graph BFS (`ripple_analysis`), `/index quality` health report |
| **LSP as agent tools** | No | `get_diagnostics`, `lsp_definition`, `lsp_type_at` callable by the model |
| **Session model** | **Tree-branching JSONL** — `/fork`, `/clone`, `/tree` navigation; all branches in one file | Linear sessions in SQLite; project-scoped by `cwd` |
| **Memory / cross-session** | Not documented | Durable key-value (`memory_set`/`memory_delete`) injected into every system prompt; `.zap/context.md` handoff; `/understand` domain map |
| **Extensibility** | TypeScript npm packages — add tools, commands, keybindings, TUI components | Markdown skill files; `zap skill install` from GitHub/URL; hooks (`hooks.json`); MCP servers |
| **Permissions / security** | None built-in — delegates to containers | Auto/Ask/Deny modes; file-write jail; secret pre-flight scan (25+ patterns); credential denylist; supply-chain CI |
| **Providers** | 15+ (Anthropic, OpenAI, Google, Bedrock, Azure, Groq, Cerebras, xAI, Ollama, OpenRouter…) | 21+ built-in slugs + any OpenAI-compat via TOML; Codex (ChatGPT subscription) |
| **Local model support** | Ollama | LM Studio + Ollama; vision heuristics per model; SLM-specific `core` tool profile (6-tool schema) |
| **Task planning** | Via extensions | Built-in: Vibe/Task mode → clarifying questions → `.zap/tasks/<slug>/tasks.md`; SLM executor pairing |
| **Background jobs** | Mentioned as architecture goal | On backlog (item #1 in crush-features backlog) |
| **Containerization** | First-class (Gondolin, Docker, OpenShell) | `sandbox = "container"` wraps shell in Docker/Podman (`--network none`) |
| **RPC / programmatic** | Structured IPC RPC mode | `--sdk` JSON-lines protocol; `--remote` token-gated WebSocket |
| **Platform** | macOS, Linux, Windows, **Android/Termux** | macOS, Linux, Windows |
| **Startup** | Node.js cold-start (~0.3–0.5s) | Sub-100ms native binary |

---

## What pi has that zap lacks

### Session tree branching
`/fork`, `/clone`, `/tree` — rewind to any point in conversation history and diverge into a new branch. All branches live in one JSONL file. Zap has `/branch`/`/switch`/`/merge` for conversation branching but it's not as deeply integrated as a first-class tree model.

### TypeScript extension ecosystem
Community npm packages can add new tools, slash commands, keybindings, and full TUI components. Extensions are published and discovered like any npm package. Zap's skill system is markdown-only (great for prompt injection, not for adding new Rust tools).

### Android / Termux support
Documented and tested on Android via Termux. Zap is not tested on Android.

### RPC protocol mode
A running pi agent can be controlled over a structured IPC channel without embedding the library — useful for IDE integration without shipping a full plugin SDK.

### No-permission-model as a choice
pi intentionally ships with no built-in permission system and documents container-based sandboxing instead. Teams that already run everything in containers prefer this over an in-process permission gate.

---

## What zap has that pi lacks

### AST symbol index
SQLite-backed tree-sitter index across Rust, Python, JS/TS, Java, C#, Go, C — offline, instant. `code_map`, `find_definition`, `find_references`, `who_calls`, `file_imports`, `where_imported`, `find_subtypes`, `find_supertypes`, `ripple_analysis` BFS. pi's baseline is grep + LLM.

### LSP exposed as agent tools
`get_diagnostics`, `lsp_definition`, `lsp_type_at` are callable tools the model can invoke. pi has no LSP integration.

### Durable memory
`memory_set`/`memory_delete` persist key-value facts across sessions in SQLite and are injected into every system prompt. pi has no equivalent; facts die with the session.

### `/understand` — domain map
One-shot LLM call that writes a business-domain map of the codebase to `.zap/understanding.md` with auto-staleness detection. Referenced by the skill system for smarter context injection.

### Built-in security hardening
- Secret pre-flight scan (25+ patterns for API keys, JWTs, cloud creds) before any cloud LLM call
- File-write jail (project root enforcement)
- Symlink-safe path guard
- Credential denylist
- Project trust gate (first-run confirmation for new directories)
- Supply-chain CI (`cargo deny`)

pi delegates all of this to the user's container setup. Fine for expert teams; risky for casual users.

### SLM-specific features
- `AGENT_TOOL_PROFILE=core`: 6-tool schema for small local models that can't handle 18 tool definitions
- Streaming idle watchdog: detects and nudges stalled SLM outputs
- First-token progress notices: keeps UX responsive when SLMs are slow
- Frontier-planner + SLM-executor pairing: Claude/GPT writes the plan, a local 7B model executes step-by-step

### Verify-aware watchdog
Failing-shell-command streak counter → rethink nudge → tool withdrawal + handoff summary. Prevents loops.

### Single binary / no runtime
`cargo install` or `curl` → one native binary. No Node.js, no npm, no version manager. Relevant for CI, Docker images, or users on constrained machines.

---

## Summary

| Dimension | pi wins | zap wins |
|---|---|---|
| Extensibility | TypeScript npm ecosystem | — |
| Session UX | Tree branching, fork/clone | — |
| Code intelligence | — | AST index, call graph, quality report |
| LSP | — | Exposed as agent tools |
| Memory | — | Durable cross-session, injected automatically |
| Security | — | Built-in, multi-layer, no containers required |
| SLM support | — | First-class: profile, watchdog, planner/executor split |
| Providers | AWS Bedrock, Azure | Codex, GoModel gateway, per-model context/output overrides |
| Mobile | Android/Termux | — |
| Runtime footprint | — | Native binary, sub-100ms startup |

**Bottom line:** pi's main differentiators are its TypeScript extension model (community tools as npm packages) and tree-branching session history. Zap's differentiators are depth of code intelligence (AST index + LSP tools), durable memory, multi-layer security that doesn't require containers, and SLM-specific execution infrastructure. They target different philosophies: pi = minimal harness + community extensions; zap = batteries-included, security-first, deep code awareness.
