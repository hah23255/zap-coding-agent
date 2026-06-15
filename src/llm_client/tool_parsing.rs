/// Recover Mistral-family native `[TOOL_CALLS]name[ARGS]{json}` markers leaked
/// into response `content`. Mistral models (Devstral, Mistral Large) sometimes
/// emit their native format instead of OpenAI `tool_calls` when generating long
/// responses with multiple actions.
///
/// **Frontier safety:** the function fast-returns when `text` lacks the literal
/// `[TOOL_CALLS]` substring. Claude / GPT / Gemini never emit it — for them
/// this is a single `str::contains` and out.
pub fn parse_mistral_tool_calls(text: &str) -> Vec<(String, serde_json::Value)> {
    const OPEN: &str = "[TOOL_CALLS]";
    const ARGS: &str = "[ARGS]";
    if !text.contains(OPEN) { return Vec::new(); }
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find(OPEN) {
        let after_open = cursor + rel + OPEN.len();
        let Some(args_rel) = text[after_open..].find(ARGS) else { cursor = after_open; continue };
        let name = text[after_open..after_open + args_rel].trim();
        // Reject empty names and names containing whitespace/newlines (chatter, not a real call).
        if name.is_empty() || name.chars().any(|c| c.is_whitespace()) { cursor = after_open + args_rel; continue; }
        let json_start = after_open + args_rel + ARGS.len();
        let Some(json_end_rel) = find_json_object_end(&text[json_start..]) else { cursor = json_start; continue };
        let json_str = &text[json_start..json_start + json_end_rel];
        if let Ok(input) = serde_json::from_str::<serde_json::Value>(json_str) {
            out.push((name.to_string(), input));
        }
        cursor = json_start + json_end_rel;
    }
    out
}

/// Walk forward finding the byte index just past the matching `}` of the first
/// balanced JSON object in `s`. Handles nested objects and string-escaped braces.
fn find_json_object_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut esc = false;
    let mut started = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if esc { esc = false; continue; }
            match b { b'\\' => esc = true, b'"' => in_str = false, _ => {} }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => { depth += 1; started = true; }
            b'}' => {
                depth -= 1;
                if started && depth == 0 { return Some(i + 1); }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::parse_mistral_tool_calls;

    #[test]
    fn no_markers_returns_empty() {
        assert_eq!(parse_mistral_tool_calls(""), Vec::new());
        assert_eq!(parse_mistral_tool_calls("just normal text from claude"), Vec::new());
        assert_eq!(parse_mistral_tool_calls("response with no tool markers at all"), Vec::new());
    }

    #[test]
    fn single_tool_call_recovered() {
        let text = r#"[TOOL_CALLS]shell[ARGS]{"command": "ls -la"}"#;
        let out = parse_mistral_tool_calls(text);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "shell");
        assert_eq!(out[0].1["command"], "ls -la");
    }

    #[test]
    fn multiple_calls_in_one_response() {
        let text = r#"Step 1[TOOL_CALLS]write_file[ARGS]{"path":"a.ts","content":"x"}Step 2[TOOL_CALLS]write_file[ARGS]{"path":"b.ts","content":"y"}"#;
        let out = parse_mistral_tool_calls(text);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].1["path"], "a.ts");
        assert_eq!(out[1].1["path"], "b.ts");
    }

    #[test]
    fn nested_json_in_args() {
        let text = r#"[TOOL_CALLS]update[ARGS]{"path":"x.ts","meta":{"nested":{"deep":true}}}"#;
        let out = parse_mistral_tool_calls(text);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1["meta"]["nested"]["deep"], true);
    }

    #[test]
    fn marker_without_args_is_chatter() {
        let text = "[TOOL_CALLS]Step 4 complete. Proceeding to Step 5.[TOOL_CALLS]write_file";
        let out = parse_mistral_tool_calls(text);
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn name_with_whitespace_rejected() {
        let text = r#"[TOOL_CALLS]not a name[ARGS]{"x":1}"#;
        let out = parse_mistral_tool_calls(text);
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn malformed_json_skipped_not_crash() {
        let text = r#"[TOOL_CALLS]shell[ARGS]{"command": broken json"#;
        let out = parse_mistral_tool_calls(text);
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn mixed_valid_and_invalid_only_valid_returned() {
        let text = concat!(
            r#"[TOOL_CALLS]bad[ARGS]{not json"#,
            r#" middle text "#,
            r#"[TOOL_CALLS]good[ARGS]{"ok":true}"#,
        );
        let out = parse_mistral_tool_calls(text);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "good");
    }

    #[test]
    fn frontier_response_is_no_op() {
        let text = "Sure, I'll help you with that. Let me read the file first.";
        let out = parse_mistral_tool_calls(text);
        assert!(out.is_empty());
    }

    #[test]
    fn escaped_braces_in_string_dont_confuse_depth() {
        let text = r#"[TOOL_CALLS]write_file[ARGS]{"path":"a.ts","content":"if (x) { return; }"}"#;
        let out = parse_mistral_tool_calls(text);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1["content"], "if (x) { return; }");
    }
}
