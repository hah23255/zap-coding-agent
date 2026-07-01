use super::app::UiBlock;

/// Scan `input` for `@path` tokens; for each resolvable file, append its
/// content to the returned string. The original tokens are preserved so the
/// user sees them in chat. Unresolvable tokens (missing files, directories)
/// are silently left as-is — the LLM still sees the label.
pub(super) fn expand_at_refs(input: &str) -> String {
    let mut expansions = String::new();
    let mut seen = std::collections::HashSet::new();

    for token in input.split_whitespace() {
        let Some(raw_path) = token.strip_prefix('@') else { continue };
        // Strip trailing punctuation the user may have typed after the path.
        let path = raw_path.trim_end_matches([',', '.', ':', ';', ')', ']', '"', '\'']);
        if path.is_empty() || seen.contains(path) {
            continue;
        }
        seen.insert(path.to_string());
        // Guard against accidentally sending huge files (e.g. logs, binaries).
        const MAX_FILE_BYTES: u64 = 256 * 1024; // 256 KB
        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if file_size > MAX_FILE_BYTES {
            expansions.push_str(&format!(
                "\n\n--- @{} --- (file too large to inline: {} KB)",
                path, file_size / 1024,
            ));
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else { continue };
        let lang = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        expansions.push_str(&format!(
            "\n\n--- @{} ---\n```{}\n{}\n```",
            path,
            lang,
            content.trim_end(),
        ));
    }

    if expansions.is_empty() {
        input.to_string()
    } else {
        format!("{}{}", input, expansions)
    }
}

/// Split a raw text string into alternating Text and Code UiBlocks.
pub fn parse_text_into_blocks(text: &str, blocks: &mut Vec<UiBlock>) {
    let mut current_text = String::new();
    let mut in_fence = false;
    let mut fence_lang = String::new();
    let mut fence_lines: Vec<String> = Vec::new();

    for line in text.lines() {
        if !in_fence {
            if line.trim_start().starts_with("```") {
                if !current_text.is_empty() {
                    blocks.push(UiBlock::Text(std::mem::take(&mut current_text)));
                }
                in_fence = true;
                fence_lang = line.trim().trim_start_matches('`').to_string();
                fence_lines.clear();
            } else {
                if !current_text.is_empty() {
                    current_text.push('\n');
                }
                current_text.push_str(line);
            }
        } else if line.trim() == "```" || line.trim() == "~~~" {
            blocks.push(UiBlock::Code {
                lang: fence_lang.clone(),
                lines: fence_lines.clone(),
            });
            in_fence = false;
            fence_lang.clear();
            fence_lines.clear();
        } else {
            fence_lines.push(line.to_string());
        }
    }

    if in_fence && !fence_lines.is_empty() {
        blocks.push(UiBlock::Code { lang: fence_lang, lines: fence_lines });
    } else if !current_text.is_empty() {
        blocks.push(UiBlock::Text(current_text));
    }
}
