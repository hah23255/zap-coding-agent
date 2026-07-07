//! Project-level state persisted in `.zap/` (project.json, context.md, session_log.md,
//! understanding.md). Unlike `~/.zap/agent.db` (global), these files are project-scoped.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
mod project_understanding;

use std::path::PathBuf;
use project_understanding::{ensure_understanding_md as ensure_understanding_md_impl, refresh_understanding_md as refresh_understanding_md_impl};

/// Returns `.zap/` path in CWD, creating it if necessary.
pub fn zap_dir() -> PathBuf {
    let dir = PathBuf::from(".zap");
    std::fs::create_dir_all(&dir).ok();
    dir
}

// ── project.json ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct ProjectMeta {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub language: Vec<String>,
    #[serde(default)]
    pub indexed: bool,
    #[serde(default)]
    pub indexed_at: Option<String>,
    #[serde(default)]
    pub initialized_at: Option<String>,
    /// Number of top-level source modules at the time `/understand` last ran.
    /// Used to detect structural drift and prompt a refresh.
    #[serde(default)]
    pub domain_module_count: Option<usize>,
}

pub fn load_project_meta() -> Option<ProjectMeta> {
    let path = PathBuf::from(".zap").join("project.json");
    let contents = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&contents).ok()
}

pub fn save_project_meta(meta: &ProjectMeta) -> Result<()> {
    let path = zap_dir().join("project.json");
    std::fs::write(&path, serde_json::to_string_pretty(meta)?)?;
    Ok(())
}

/// Mark the project as indexed (called after a successful /index run).
pub fn mark_indexed() {
    let mut meta = load_project_meta().unwrap_or_default();
    meta.indexed = true;
    meta.indexed_at = Some(Utc::now().to_rfc3339());
    let _ = save_project_meta(&meta);
}

// Re-export domain_map functions so callers use crate::project::* unchanged.
pub use crate::domain_map::{
    build_domain_extraction_prompt,
    domain_map_is_stale,
    has_domain_map,
    load_domain_map,
    mark_domain_map_current,
    save_domain_section,
    source_module_count,
};

// ── context.md ────────────────────────────────────────────────────────────────

/// Returns the raw content of `.zap/context.md` if it exists and is non-empty.
pub fn load_session_context() -> Option<String> {
    let s = std::fs::read_to_string(PathBuf::from(".zap").join("context.md")).ok()?;
    if s.trim().is_empty() { None } else { Some(s) }
}

/// Extract just the "What was being worked on" line for the startup banner.
pub fn context_summary() -> Option<String> {
    let raw = load_session_context()?;
    // Find the line after "## What was being worked on"
    let mut found = false;
    for line in raw.lines() {
        if found {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with("<!--") {
                return Some(trimmed.to_string());
            }
        }
        if line.starts_with("## What was being worked on") {
            found = true;
        }
    }
    None
}

/// Extract the files list from context.md for the startup banner.
pub fn context_files() -> Vec<String> {
    let raw = match load_session_context() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut in_files = false;
    let mut files = Vec::new();
    for line in raw.lines() {
        if line.starts_with("## Files touched") {
            in_files = true;
            continue;
        }
        if in_files {
            if line.starts_with("## ") { break; }
            let trimmed = line.trim().trim_start_matches("- ");
            if !trimmed.is_empty() && trimmed != "(none)" && !trimmed.starts_with("<!--") {
                files.push(trimmed.to_string());
            }
        }
    }
    files
}

/// Extract the "What's next" section content from context.md, if any.
pub(crate) fn extract_whats_next(content: &str) -> Option<String> {
    let mut in_section = false;
    let mut lines: Vec<&str> = Vec::new();
    for line in content.lines() {
        if line.starts_with("## What's next") {
            in_section = true;
            continue;
        }
        if in_section {
            if line.starts_with("## ") { break; }
            lines.push(line);
        }
    }
    let text = lines.join("\n").trim().to_string();
    if text.is_empty() || text.starts_with("<!--") {
        None
    } else {
        Some(text)
    }
}

/// Write `.zap/context.md` at session end.
/// `whats_next`: LLM-generated summary of next steps; if `None`, preserves existing content.
pub fn save_session_context(
    session_id: i64,
    goal: &str,
    files_changed: &[String],
    whats_next: Option<&str>,
) -> Result<()> {
    let now = Utc::now().format("%Y-%m-%d %H:%M").to_string();
    let files_section = if files_changed.is_empty() {
        "  (none)".to_string()
    } else {
        // Deduplicate preserving order
        let mut seen = std::collections::HashSet::new();
        files_changed.iter()
            .filter(|f| seen.insert(*f))
            .map(|f| format!("  - {}", f))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Prefer the provided summary; fall back to preserved existing content.
    let existing = std::fs::read_to_string(zap_dir().join("context.md")).unwrap_or_default();
    let next_section = whats_next
        .map(|s| s.to_string())
        .or_else(|| extract_whats_next(&existing))
        .unwrap_or_else(|| "<!-- fill this in between sessions -->".to_string());

    let content = format!(
        "# Session Context\n\
         \n\
         <!-- auto-written by zap at session end — edit freely -->\n\
         \n\
         ## Last updated\n\
         {now} — Session #{session_id}\n\
         \n\
         ## What was being worked on\n\
         {goal}\n\
         \n\
         ## Files touched\n\
         {files_section}\n\
         \n\
         ## What's next\n\
         {next_section}\n"
    );
    std::fs::write(zap_dir().join("context.md"), content)?;
    Ok(())
}

// ── session_log.md ────────────────────────────────────────────────────────────

/// Prepend one entry to `.zap/session_log.md` (newest first, capped at ~20k chars).
pub fn append_session_log(session_id: i64, goal: &str, files_changed: &[String], whats_next: Option<&str>) -> Result<()> {
    let path = zap_dir().join("session_log.md");
    let now = Utc::now().format("%Y-%m-%d").to_string();
    let files = if files_changed.is_empty() {
        "(no files modified)".to_string()
    } else {
        let mut seen = std::collections::HashSet::new();
        files_changed.iter()
            .filter(|f| seen.insert(*f))
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    };
    let next_line = whats_next
        .filter(|s| !s.trim().is_empty() && !s.contains("<!--"))
        .map(|s| {
            let joined = s.lines()
                .map(|l| l.trim().trim_start_matches("- ").trim_start_matches("• "))
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" | ");
            format!("Next: {}\n", joined)
        })
        .unwrap_or_default();
    let entry = format!("## Session #{session_id} — {now}\nGoal: {goal}\nFiles: {files}\n{next_line}\n");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let combined = format!("{}{}", entry, existing);
    let capped: String = combined.chars().take(20_000).collect();
    std::fs::write(&path, capped)?;
    Ok(())
}

/// Load recent entries from `.zap/session_log.md`, capped at `max_chars`.
pub fn load_session_log(max_chars: usize) -> Option<String> {
    let s = std::fs::read_to_string(PathBuf::from(".zap").join("session_log.md")).ok()?;
    if s.trim().is_empty() {
        return None;
    }
    if s.len() <= max_chars {
        Some(s)
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        Some(format!("{}\n\n[… truncated]", truncated))
    }
}

/// Look up the files string for a session from session_log.md.
pub fn session_log_files(session_id: i64) -> Option<String> {
    let s = std::fs::read_to_string(PathBuf::from(".zap").join("session_log.md")).ok()?;
    let marker = format!("## Session #{}", session_id);
    let mut in_target = false;
    for line in s.lines() {
        if line.starts_with(&marker) {
            in_target = true;
            continue;
        }
        if in_target {
            if line.starts_with("## Session #") {
                break; // next session entry — not ours
            }
            if let Some(files) = line.strip_prefix("Files: ") {
                return Some(files.to_string());
            }
        }
    }
    None
}

/// Extract up to `limit` "Next:" lines from session_log.md (newest-first). Returns "• bullet\n…" or None.
pub fn load_recent_whats_next(limit: usize) -> Option<String> {
    let s = std::fs::read_to_string(PathBuf::from(".zap").join("session_log.md")).ok()?;
    let bullets: Vec<String> = s.lines()
        .filter(|l| l.starts_with("Next: "))
        .take(limit)
        .map(|l| format!("• {}", l.trim_start_matches("Next: ").trim()))
        .collect();
    if bullets.is_empty() { None } else { Some(bullets.join("\n")) }
}
// ── understanding.md ──────────────────────────────────────────────────────────

/// Load `.zap/understanding.md`, capped at `max_chars` for system-prompt injection.
pub fn load_understanding(max_chars: usize) -> Option<String> {
    let s = std::fs::read_to_string(PathBuf::from(".zap").join("understanding.md")).ok()?;
    if s.trim().is_empty() {
        return None;
    }
    if s.len() <= max_chars {
        Some(s)
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        Some(format!("{}\n\n[… truncated — see .zap/understanding.md for full content]", truncated))
    }
}

/// Returns true when understanding.md has no real LLM-written analysis yet —
/// just the auto-generated stats block plus the placeholder comment.
/// Used to decide whether to auto-trigger an init analysis turn on startup.
pub fn understanding_needs_analysis() -> bool {
    let s = match std::fs::read_to_string(zap_dir().join("understanding.md")) {
        Ok(s) => s,
        Err(_) => return true, // file missing
    };
    if s.trim().is_empty() { return true; }
    // Has real analysis if any of these section headers appear after the stats block.
    let has_analysis = s.contains("## Architecture")
        || s.contains("## Overview")
        || (s.contains("## Analysis")
            && !s.contains("<!-- Run `/init`"));
    !has_analysis
}

pub fn save_understanding(content: &str) -> Result<()> {
    std::fs::write(zap_dir().join("understanding.md"), content)?;
    Ok(())
}

/// Refresh the deterministic stats block of `.zap/understanding.md` at session start,
/// preserving any LLM-written analysis. No LLM call needed.
pub fn refresh_understanding_md(
    cwd_name: Option<String>,
    files: usize,
    symbols: usize,
    lang_counts: &[(String, usize)],
) -> Result<()> {
    refresh_understanding_md_impl(&zap_dir(), cwd_name, files, symbols, lang_counts)
}

/// Create a default `.zap/understanding.md` if absent or placeholder-only;
/// fills in deterministic stats from the code index when available.
pub fn ensure_understanding_md(
    cwd_name: Option<String>,
    files: usize,
    symbols: usize,
    lang_counts: &[(String, usize)],
) -> Result<()> {
    ensure_understanding_md_impl(&zap_dir(), cwd_name, files, symbols, lang_counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_context(next: &str) -> String {
        format!("# Session Context\n## What was being worked on\nsome goal\n## Files touched\n  - foo.rs\n## What's next\n{next}\n")
    }

    #[test]
    fn extract_returns_content() {
        let s = make_context("- Finish auth module
- Write tests");
        let r = extract_whats_next(&s).unwrap();
        assert!(r.contains("Finish auth module"), "got: {r}");
        assert!(r.contains("Write tests"));
    }

    #[test]
    fn extract_none_for_placeholder_comment() {
        let s = make_context("<!-- fill this in between sessions -->");
        assert!(extract_whats_next(&s).is_none());
    }

    #[test]
    fn extract_none_when_section_absent() {
        let s = "# Session Context
## What was being worked on
goal
";
        assert!(extract_whats_next(s).is_none());
    }

    #[test]
    fn extract_none_for_blank_content() {
        let s = make_context("

  
");
        assert!(extract_whats_next(&s).is_none());
    }

    #[test]
    fn extract_stops_at_next_section_header() {
        let s = "## What's next
- step one
## Other section
- should not appear
";
        let r = extract_whats_next(s).unwrap();
        assert!(r.contains("step one"));
        assert!(!r.contains("should not appear"), "leaked: {r}");
    }

    #[test]
    fn extract_trims_whitespace() {
        let s = "## What's next

  - trimmed  

";
        let r = extract_whats_next(s).unwrap();
        assert!(!r.starts_with("
"), "leading newline: {r}");
        assert!(!r.ends_with("
"), "trailing newline: {r}");
    }

    #[test]
    fn extract_multiline_content() {
        let s = make_context("- step one
- step two
- step three");
        let r = extract_whats_next(&s).unwrap();
        assert_eq!(r.lines().count(), 3);
    }

    #[test]
    fn load_recent_whats_next_parses_bullets() {
        // Calls the real function via tempdir+chdir; session_log is newest-first.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".zap")).unwrap();
        std::fs::write(dir.path().join(".zap/session_log.md"),
            "## Session #2\nGoal: g2\nFiles: (none)\nNext: do C\n\n\
             ## Session #1\nGoal: g1\nFiles: (none)\nNext: do A | do B\n\n").unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let text = load_recent_whats_next(3);
        std::env::set_current_dir(&orig).unwrap();
        let text = text.expect("should find Next: lines");
        assert!(text.starts_with("• "), "should start with bullet: {text}");
        assert!(text.contains("• do C"), "got: {text}");
        assert!(text.contains("• do A | do B"), "got: {text}");
    }


}
