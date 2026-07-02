---
category: practice
name: slash-commands
trigger: ["slash command", "/command", "add command", "new command", "tui command", "missing command", "command picker", "autocomplete command", "slash completion"]
tokens: ~420
---

## Adding or changing a zap slash command

Goal: make sure a new slash command is not only implemented, but also **discoverable** in the picker and documented consistently.

### Checklist

1. **Find the handler path first**
   - TUI picker/registry: `src/tui/commands/mod.rs`
   - TUI command execution: `src/tui/turn_handler.rs`
   - TUI-specific helpers: `src/tui/*_handler.rs`
   - Session/help text: `src/session/commands/info.rs`

2. **Implement the command behavior**
   - Add or update the actual command handler.
   - Keep the change surgical; match existing command style.

3. **Register it in slash completions**
   - Update `SLASH_COMMANDS` in `src/tui/commands/mod.rs`.
   - If there is a paired command, add both (example: `/schedule` and `/unschedule`).
   - If the command has argument forms, choose the picker label carefully (example: `/remote [port]`).

4. **Wire command dispatch**
   - Confirm `src/tui/turn_handler.rs` routes the command to the handler.
   - If it is session-only or TUI-only, keep that boundary explicit.

5. **Update help/docs**
   - Add or update the relevant entry in `src/session/commands/info.rs`.
   - If the command is user-facing enough to matter in release notes, update `FEATURES.md` too.

6. **Protect against regressions**
   - Add or update a picker/completion test in `src/tui/commands/mod.rs` when practical.
   - For missing-command bugs, prefer a test that asserts the command appears in `filter_commands("/", ...)` or prefix filtering.

7. **Verify immediately after editing**
   - Run `get_diagnostics` on every edited Rust file.
   - Run targeted tests if you added one.
   - If the command affects broader flow, run `cargo test`.

### Common failure mode

A command can be fully implemented and still feel "broken" because it was never added to `SLASH_COMMANDS`. Always check implementation **and** discoverability.

### Minimum done criteria

A slash command change is only done when all are true:
- handler exists
- dispatch path exists
- picker entry exists
- help text exists
- verification passed
