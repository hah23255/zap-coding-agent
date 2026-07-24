## Before you record

1. **Terminal 1** — clean shell, `cd` into a real project (NOT the zap repo itself — use something like a cloned Flask/Express repo so the demo feels like "a user's project", not zap's own source). `demos/code_indexing/flask` and `demos/skill_injection/flask` already exist pre-indexed if you want a shortcut.
2. **Terminal 2** — standing by for `sqlite3 .zap/code.db` queries and `cat ~/.zap/audit.jsonl`.
3. **Browser tab** — [README.md on GitHub](https://github.com/zap-coding-agent/zap-coding-agent#readme) scrolled to the "LLM Token Efficiency Matrix" table (~line 38-44) for the prompt-bloat comparison screenshot.

---

## [0:00] Cold open / hook (15s)

[Screen: terminal, blank prompt]

"Every AI coding agent you've used ... sends way more to the LLM than you think.
... Before it reads one line of your code.
... I got tired of that, so I built my own coding agent in Rust — it's called zap.
... Let me show you what makes it different."

---

## [0:15] Install (20s)

[Screen: terminal, run the one-liner]

```bash
curl -fsSL https://raw.githubusercontent.com/zap-coding-agent/zap-coding-agent/main/install.sh | bash
```

"One line. No Node, no Python, no Docker.
... zap is a single Rust binary — under 30 megabytes, sitting around 20 megabytes of memory when it's idle.
... Compare that to agents built on Node or Electron that need an entire JavaScript runtime just to start."

[Screen: `zap` — TUI banner appears]

"That's it. That's the whole install. Let's get into what's actually interesting."

---

## [0:35] Feature 1 — Prompt bloat (60s)

[Screen: type `hi` into zap, hit enter, let the response come back fast]

"First thing — prompt bloat.
... Open any popular coding agent, say 'hi', and inspect what it actually sends the LLM.
... You'll find hundreds, sometimes thousands of tokens of system prompt — every single turn, no matter what you typed.
... Some of these agents also register 70-plus tools on every request, whether you'll ever use them or not."

[Screen: cut to README token comparison table, or `content/evidence/system-prompt-comparison.md`]

"We measured it. Gemini CLI sends the same four thousand ninety-six tokens whether you're writing Java or React —
... the word 'java' doesn't even appear in its prompt file.
... OpenCode sends the same static prompt too — one string, every task.
... zap does the opposite: say 'hi' and it costs about thirty tokens. Not two thousand."

[Screen: back to zap TUI, show tool list or `/skill list`]

"And instead of dumping seventy-plus tools into context, zap starts with under thirty core tools —
... everything else is skill-based: markdown files that only get injected when your message actually needs them."

[Screen: type a Rust question, show the `↳ skills: rust` line appear]

"Ask a Rust question, the rust skill fires.
... Ask a git question, the git skill fires.
... Say 'thanks', nothing fires — you're back to a near-empty prompt."

---

## [1:35] Feature 2 — Code indexing (55s)

[Screen: run `zap --index-only` or show `/init` indexing a repo]

"Second — code indexing.
... Most agents 'read' your codebase by grepping for a string and hoping it's the right match.
... zap builds a real AST symbol index at startup — tree-sitter plus SQLite — so the model actually knows what exists before it writes anything."

[Screen: `sqlite3 .zap/code.db "SELECT path, line, kind FROM symbols WHERE name LIKE '%UserRepo%';"`]

"That's a plain SQLite database at `.zap/code.db` — you can query it yourself, no zap session needed.
... Ask zap to add a user repository, and it looks the symbol up in milliseconds instead of guessing —
... it edits the file that already exists instead of creating a duplicate one next to it."

[Screen: in zap, ask "where is X defined" and show `find_definition` hitting the index — INDEX hit log line]

"Every one of these lookups is logged — hit or miss — so you can see exactly when the index answered the question versus when it had to fall back to grep.
... Fewer blind reads, fewer wasted tool calls, way less guessing."

---

## [2:30] Feature 3 — Context visibility (70s)

[Screen: have a short real conversation — 2-3 turns — then type `/context`]

"Third — and this is the one I haven't seen in any other agent — context visibility.
... Every coding tool shows you a number. Twenty-two percent.
... That's it. You have no idea which question caused it, or what to do about it."

[Screen: `/context` overlay opens, full turn list with token costs]

"zap opens this instead.
... Every turn in your session, with its exact token cost and its share of the context window."

[Screen: navigate to the heaviest turn, open the detail panel]

"Open any turn and you see exactly what was sent — the real tool calls, the real JSON, the real file contents that came back.
... Nothing hidden."

[Screen: press `d` to drop a turn, confirm, show the token count drop live in the header]

"And if a turn already did its job — you don't need those file contents anymore — you delete just that turn.
... Not the whole session. Not a vague 'compact' button. That one turn, gone, and the token count drops instantly.
... This is full transparency into what's going to the LLM, on every single turn."

---

## [3:40] Feature 4 — Lazy-loaded MCP (35s)

[Screen: `/mcp list` showing servers as "pending"]

"Fourth — MCP support, but lazy-loaded.
... Most agents connect to every configured MCP server at startup and dump every tool schema into context, every turn — ten servers, five tools each, that's ten thousand-plus wasted tokens before you've asked anything.
... zap keeps every server pending. The model just sees a one-line stub per server."

[Screen: trigger a tool that needs an MCP server, watch it connect on demand]

"Only when the model actually needs one does it call `mcp_connect` — zap spawns it, does the handshake, and the real tools show up for that turn.
... And because it's the same `.mcp.json` format Claude Code and Cursor use, your config just works everywhere."

---

## [4:15] Feature 5 — Handoff document (35s)

[Screen: exit zap with `/exit`, then `cat .zap/context.md`]

"Fifth — session handoff.
... Most agents forget everything the moment you close the terminal. You re-explain the goal every time.
... When zap exits, it writes `.zap/context.md` automatically — the goal, the files touched, and a short 'what's next' summary."

[Screen: relaunch zap, show the "Last: ..." line in the startup banner]

"Next time you open zap, that's the first thing you see — before you've typed a single word, the agent already knows exactly where you left off and what's still unfinished."

---

## [4:50] Feature 6 — Audit log (25s)

[Screen: `/audit 20` inside zap, or `cat ~/.zap/audit.jsonl | tail -20`]

"Sixth — every tool call zap makes, every file it touches, every shell command it runs, gets written to a structured JSON log at `~/.zap/audit.jsonl`.
... Timestamp, tool name, outcome. Full audit trail, not just a chat transcript you have to scroll through."

---

## [5:15] Feature 7 — Skills from multiple directories (25s)

[Screen: open `~/.agent.toml`, show `skill_paths`]

"Last one — skills aren't locked to zap's own format.
... If your project already has skills written for Claude Code or Amazon Kiro, you point `skill_paths` at those folders in your config, and zap loads them right alongside its own —
... project skills, personal skills, and imported skills, in that priority order, no copy-pasting files around."

---

## [5:40] Close / CTA (20s)

[Screen: back to zap TUI banner]

"That's zap — a Rust coding agent that treats every token like it costs something.
... It's open-source, link's in the description. Try the install, give it a star if it's useful.
... Thanks for watching."

[End card]
