# Session Context

<!-- auto-written by zap at session end — edit freely -->

## Last updated
2026-07-04 06:50 — Session #551

## What was being worked on
can you implement a feature to set coding model for task , ther eis issue in git

## Files touched
  - src/session/task_classifier.rs
  - src/config/tests.rs
  - src/config/mod.rs

## What's next
- Add a `web-search` subcommand to the `zap` CLI and wire it to the existing search/news-fetch tool so `zap web-search --query "latest news"` works instead of returning `unrecognized subcommand 'web-search'`.
- Implement a resilient news-fetch path in the web search integration: keep the Google News RSS fallback for blocked DuckDuckGo requests, and expose topic filtering like `India`, `AI`, `world`, and `markets` through the CLI/API.
- Verify provider capability handling for DeepSeek in the repo’s model integration layer: add an explicit “vision/image-input supported or not” check per model/provider and surface a clear error or capability flag when DeepSeek vision is not confirmed.
