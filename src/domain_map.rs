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
    "Please map the business domains of this codebase. Use these tools in order:\n\
     \n\
     1. `code_map '.'` — one call to see the full project layout with symbols\n\
     2. (optional) `code_map 'src'` if step 1 wasn't detailed enough\n\
     3. `read_file` on the manifest file (Cargo.toml / package.json / go.mod / pom.xml) — \
        one call only, for tech stack\n\
     \n\
     DO NOT read individual source files. The code_map output is sufficient.\n\
     \n\
     Write the result to `.zap/understanding.md` using `write_file`. Merge it with existing \
     content — do NOT overwrite the file, only add or replace the Domain Map section. \
     The section must be wrapped in these exact sentinel comments:\n\
     \n\
     <!-- zap:domain-map:begin -->\n\
     ## Domain Map\n\
     ...\n\
     <!-- zap:domain-map:end -->\n\
     \n\
     The Domain Map must contain:\n\
     \n\
     ### Business Domains\n\
     A table: | Domain | Owns | Key entry points |\n\
     List each distinct business concern (auth, billing, storage, etc.) with the \
     modules/files that implement it and 1-2 key function names.\n\
     \n\
     ### Cross-Cutting Concerns\n\
     Bullet list of infrastructure/plumbing that every domain touches \
     (error handling, logging, config, DB, serialization).\n\
     \n\
     ### Dependency Direction\n\
     One-line description of the layering rule (e.g. 'tools → session → agent_core; \
     nothing imports upward').\n\
     \n\
     Keep each table row to ≤ 120 chars. No narrative prose — facts and file paths only.\n\
     \n\
     Start your reply: 'Domain map via: [tools used]'."
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
