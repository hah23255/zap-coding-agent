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
    "Map the business domains of this codebase from CODE STRUCTURE, not from documentation.\n\
     \n\
     STRICT RULES:\n\
     - NEVER read README.md, CLAUDE.md, AGENTS.md, or any other markdown/doc files.\n\
     - NEVER read package manifests for domain info (one manifest read for tech stack only).\n\
     - All domain insights must come from actual source code structure.\n\
     \n\
     Steps:\n\
     1. `code_map '.'` — see the full project layout and top-level structure\n\
     2. `code_map` on the main source directories (src/, app/, lib/, Services/, Controllers/, \
        pages/, api/, etc.) — repeat for each major directory to get symbol-level detail\n\
     3. One `read_file` on a package manifest (Cargo.toml / package.json / go.mod / \
        *.csproj / pom.xml) for tech stack identification only\n\
     \n\
     Your job is to surface things the documentation does NOT say:\n\
     - Which modules are the most connected (called from many places)?\n\
     - What are the real entry points vs peripheral code?\n\
     - Are there clusters of files that belong together as a domain?\n\
     - What cross-cutting patterns appear repeatedly across the codebase?\n\
     \n\
     Write the result to `.zap/understanding.md` using `write_file`. Merge — do NOT overwrite \
     the file. Only add or replace the Domain Map section using these exact sentinels:\n\
     \n\
     <!-- zap:domain-map:begin -->\n\
     ## Domain Map\n\
     ...\n\
     <!-- zap:domain-map:end -->\n\
     \n\
     The Domain Map must contain:\n\
     \n\
     ### Tech Stack\n\
     One line: language(s), framework(s), DB, key libraries.\n\
     \n\
     ### Business Domains\n\
     Table: | Domain | Source paths | Key symbols |\n\
     Each row = one distinct business concern (auth, billing, notifications, etc.) \
     with the files/dirs that implement it and 2-3 real function/class names found in \
     the code.\n\
     \n\
     ### Cross-Cutting Concerns\n\
     Bullet list of plumbing every domain touches (logging, error handling, config, DB \
     access, serialization). Include the actual module/file that owns each concern.\n\
     \n\
     ### Dependency Direction\n\
     One sentence: the layering rule observed in the code \
     (e.g. 'Controllers → Services → Repositories → DB; no upward imports').\n\
     \n\
     ### Hotspots\n\
     Up to 5 files/modules that appear most connected or central based on code_map output. \
     These are the highest-leverage files for understanding the system.\n\
     \n\
     Rules: ≤ 120 chars per table row. Facts and file paths only — no narrative prose. \
     Every claim must be grounded in what code_map showed, not documentation.\n\
     \n\
     Start your reply: 'Domain map via: [list the exact tool calls made]'."
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
