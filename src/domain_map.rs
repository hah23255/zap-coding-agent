//! Business-domain extraction: `/understand` command support.
//!
//! Writes a structured domain map into `.zap/understanding.md` between
//! sentinel comments so it can be updated without destroying other content.

use anyhow::Result;

pub(crate) const DOMAIN_BEGIN: &str = "<!-- zap:domain-map:begin -->";
pub(crate) const DOMAIN_END:   &str = "<!-- zap:domain-map:end -->";

/// Count how many top-level Rust/Go/Python/TS source modules exist in `src/` (or CWD).
pub fn source_module_count() -> usize {
    let candidates = [
        ("src", "rs"),
        ("src", "go"),
        ("src", "py"),
        ("src", "ts"),
        (".",   "go"),
        (".",   "py"),
    ];
    for (dir, ext) in &candidates {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let n = entries
                .flatten()
                .filter(|e| e.path().extension().map(|x| x == *ext).unwrap_or(false))
                .count();
            if n > 0 { return n; }
        }
    }
    0
}

/// True when the recorded module count has drifted >10% from the current count,
/// which suggests a `/understand` refresh would be valuable.
pub fn domain_map_is_stale() -> bool {
    let Some(meta) = crate::project::load_project_meta() else { return false };
    let Some(recorded) = meta.domain_module_count else { return true };
    let current = source_module_count();
    if recorded == 0 { return current > 0; }
    let diff = (current as isize - recorded as isize).unsigned_abs();
    diff * 10 > recorded
}

/// Check whether `.zap/understanding.md` already contains a domain map section.
pub fn has_domain_map() -> bool {
    std::fs::read_to_string(crate::project::zap_dir().join("understanding.md"))
        .map(|s| s.contains(DOMAIN_BEGIN))
        .unwrap_or(false)
}

/// Load the domain map's content (business domains, dependency direction,
/// cross-cutting concerns) from between its sentinel comments, if present.
/// This is real, already-computed architecture knowledge — distinct from the
/// `/init`-only "## Analysis" section — and previously wasn't surfaced in any
/// system prompt at all (only a "you should run /understand" nudge fired when
/// it was *missing*; the content itself was never injected when it existed).
pub fn load_domain_map() -> Option<String> {
    load_domain_map_from(&crate::project::zap_dir().join("understanding.md"))
}

pub(crate) fn load_domain_map_from(path: &std::path::Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let start = raw.find(DOMAIN_BEGIN)? + DOMAIN_BEGIN.len();
    let end = raw[start..].find(DOMAIN_END)? + start;
    let content = raw[start..end].trim();
    if content.is_empty() { None } else { Some(content.to_string()) }
}

/// Merge the LLM-generated domain section into `.zap/understanding.md`.
/// If the file already has a domain section (sentinel pair), it is replaced.
/// Otherwise the section is appended.
pub fn save_domain_section(domain_content: &str) -> Result<()> {
    save_domain_section_to(
        &crate::project::zap_dir().join("understanding.md"),
        domain_content,
    )
}

pub(crate) fn save_domain_section_to(
    path: &std::path::Path,
    domain_content: &str,
) -> Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let block = format!("{}\n{}\n{}", DOMAIN_BEGIN, domain_content.trim(), DOMAIN_END);
    let new_content = if let (Some(start), Some(end_pos)) =
        (existing.find(DOMAIN_BEGIN), existing.find(DOMAIN_END))
    {
        let end = end_pos + DOMAIN_END.len();
        let before = existing[..start].trim_end_matches('\n').to_string();
        let after  = existing[end..].trim_start_matches('\n').to_string();
        if after.is_empty() {
            format!("{}\n\n{}\n", before, block)
        } else {
            format!("{}\n\n{}\n\n{}\n", before, block, after)
        }
    } else {
        let base = existing.trim_end_matches('\n').to_string();
        format!("{}\n\n{}\n", base, block)
    };
    std::fs::write(path, new_content)?;
    Ok(())
}

/// Record the current module count so staleness can be detected next session.
pub fn mark_domain_map_current() {
    let mut meta = crate::project::load_project_meta().unwrap_or_default();
    meta.domain_module_count = Some(source_module_count());
    let _ = crate::project::save_project_meta(&meta);
}

/// Build the LLM prompt that drives domain extraction.
/// Pure function — testable without an LLM.
pub fn build_domain_extraction_prompt() -> String {
    "Map the business domains of this codebase from CODE STRUCTURE only.\n\
     \n\
     STRICT RULES — read before doing anything:\n\
     - NEVER read .zap/understanding.md (do not recycle old output)\n\
     - NEVER read README.md, CLAUDE.md, AGENTS.md, or any markdown/doc file\n\
     - ONE manifest read allowed (Cargo.toml / package.json / go.mod / *.csproj) for tech stack only\n\
     - Every claim in your output must be grounded in code_map results, not docs\n\
     \n\
     Exploration — REQUIRED minimum 3 code_map calls:\n\
     1. `code_map '.'` — top-level layout, identify the main source directories\n\
     2. `code_map '<main-src-dir>'` — e.g. src/, lib/, Services/, Controllers/, app/, pages/, api/\n\
     3. `code_map` on 1-2 more subdirectories that look like distinct domains\n\
     Repeat for more directories if the codebase has multiple top-level areas.\n\
     \n\
     From the code_map output, extract:\n\
     - Which directories/files own each business concern?\n\
     - Which symbols (functions/classes) are the real entry points?\n\
     - Which files appear in many call chains (hotspots)?\n\
     - What layering pattern does the code enforce?\n\
     \n\
     Output — write ONLY the domain section using edit_file on `.zap/understanding.md`.\n\
     Replace the block between these exact sentinels (add them if absent):\n\
     \n\
     <!-- zap:domain-map:begin -->\n\
     ## Domain Map\n\
     ...\n\
     <!-- zap:domain-map:end -->\n\
     \n\
     The section must contain these four sub-sections and nothing else:\n\
     \n\
     ### Tech Stack\n\
     One line: language · framework · DB · key libraries (from manifest only).\n\
     \n\
     ### Business Domains\n\
     Table: | Domain | Source path(s) | Key entry points |\n\
     One row per distinct business concern. Key entry points = real function/class names \
     seen in code_map output, not invented names.\n\
     \n\
     ### Cross-Cutting Concerns\n\
     Bullet list: each item = concern name + the file/module that owns it \
     (e.g. '- Logging: src/log.rs'). Only include concerns actually visible in code_map.\n\
     \n\
     ### Hotspots\n\
     Up to 5 files ranked by centrality (most symbols + most called). \
     Format: `path/to/file.rs — reason it's central`.\n\
     \n\
     Formatting rules: ≤ 120 chars per table row. No narrative prose — file paths and \
     symbol names only. Do not invent names not seen in code_map output.\n\
     \n\
     Start your reply with: 'Domain map via: [list all code_map calls made]'."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_understanding(content: &str) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn save_domain_section_appends_when_absent() {
        let existing = "# Understanding\n\n## Analysis\nsome existing text\n";
        let f = tmp_understanding(existing);
        save_domain_section_to(f.path(), "## Domain Map\n- auth: src/auth.rs").unwrap();
        let result = std::fs::read_to_string(f.path()).unwrap();
        assert!(result.contains(DOMAIN_BEGIN), "begin sentinel missing");
        assert!(result.contains("## Domain Map"), "section missing");
        assert!(result.contains("auth: src/auth.rs"), "content missing");
        assert!(result.contains(DOMAIN_END), "end sentinel missing");
        assert!(result.contains("some existing text"), "existing content lost");
    }

    #[test]
    fn save_domain_section_replaces_existing() {
        let existing = format!(
            "# Understanding\n\n{}\n## Domain Map\nold content\n{}\n",
            DOMAIN_BEGIN, DOMAIN_END
        );
        let f = tmp_understanding(&existing);
        save_domain_section_to(f.path(), "## Domain Map\nnew content").unwrap();
        let result = std::fs::read_to_string(f.path()).unwrap();
        assert!(!result.contains("old content"), "old content not replaced");
        assert!(result.contains("new content"), "new content missing");
        assert_eq!(result.matches("zap:domain-map:begin").count(), 1);
        assert_eq!(result.matches("zap:domain-map:end").count(), 1);
    }

    #[test]
    fn save_domain_section_preserves_content_after_block() {
        let existing = format!(
            "# Understanding\n\n{}\n## Domain Map\nold\n{}\n\n## Other Section\nkept\n",
            DOMAIN_BEGIN, DOMAIN_END
        );
        let f = tmp_understanding(&existing);
        save_domain_section_to(f.path(), "## Domain Map\nnew").unwrap();
        let result = std::fs::read_to_string(f.path()).unwrap();
        assert!(result.contains("## Other Section"), "trailing section lost");
        assert!(result.contains("kept"), "trailing content lost");
        assert!(result.contains("new"), "new domain content missing");
        assert!(!result.contains("old\n"), "old content not replaced");
    }

    #[test]
    fn load_domain_map_extracts_content_between_sentinels() {
        let existing = format!(
            "# Understanding\n\n{}\n## Domain Map\n\n### Business Domains\nauth: src/auth.rs\n{}\n",
            DOMAIN_BEGIN, DOMAIN_END
        );
        let f = tmp_understanding(&existing);
        let content = load_domain_map_from(f.path()).expect("domain map should be found");
        assert!(content.contains("Business Domains"));
        assert!(content.contains("auth: src/auth.rs"));
        assert!(!content.contains(DOMAIN_BEGIN), "sentinel should be stripped");
    }

    #[test]
    fn load_domain_map_absent_returns_none() {
        let f = tmp_understanding("# Understanding\n\nno domain map here\n");
        assert!(load_domain_map_from(f.path()).is_none());
    }

    #[test]
    fn build_domain_extraction_prompt_is_nonempty() {
        let p = build_domain_extraction_prompt();
        assert!(p.contains("code_map"), "should reference code_map tool");
        assert!(p.contains("Domain Map"), "should name the output section");
        assert!(p.contains("zap:domain-map:begin"), "should include sentinel");
        assert!(p.contains("Business Domains"), "should request domain table");
    }

    #[test]
    fn staleness_threshold_not_stale() {
        let recorded = 10usize;
        let current  = 10usize;
        let diff = (current as isize - recorded as isize).unsigned_abs();
        assert!(diff * 10 <= recorded);
    }

    #[test]
    fn staleness_threshold_stale() {
        let recorded = 10usize;
        let current  = 15usize;
        let diff = (current as isize - recorded as isize).unsigned_abs();
        assert!(diff * 10 > recorded);
    }
}
