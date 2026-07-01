use anyhow::Result;

/// Refresh the deterministic stats block of `.zap/understanding.md` at session start,
/// preserving any LLM-written analysis. No LLM call needed.
pub(crate) fn refresh_understanding_md(
    zap_dir: &std::path::Path,
    cwd_name: Option<String>,
    files: usize,
    symbols: usize,
    lang_counts: &[(String, usize)],
) -> Result<()> {
    let path = zap_dir.join("understanding.md");

    let name = cwd_name.as_deref().unwrap_or("(unknown)");
    let version = read_project_version().map(|v| format!(" v{v}")).unwrap_or_default();
    let langs_block = if lang_counts.is_empty() {
        String::new()
    } else {
        let parts: Vec<String> = lang_counts.iter()
            .map(|(l, n)| format!("  - {l}: {n} symbols"))
            .collect();
        format!("### Languages\n{}\n\n", parts.join("\n"))
    };

    let modules_block = list_source_modules();
    let skills_block = count_builtin_skills()
        .map(|n| format!("### Built-in skills\n  {n} skills in `src/default_skills/`\n\n"))
        .unwrap_or_default();

    let stats_block = format!(
        "<!-- zap:auto-stats:begin -->\n\
         ## Project\n\
         {name}{version} · {files} files · {symbols} symbols\n\n\
         {langs_block}\
         {modules_block}\
         {skills_block}\
         <!-- zap:auto-stats:end -->\n"
    );

    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let analysis_section = extract_analysis_section(&existing);

    let content = format!(
        "# Understanding\n\
         <!-- auto-updated by zap at session start — edit the Analysis section freely -->\n\n\
         {stats_block}\n\
         {analysis_section}"
    );

    std::fs::write(&path, content)?;
    Ok(())
}

/// Create a default `.zap/understanding.md` if absent or placeholder-only;
/// fills in deterministic stats from the code index when available.
pub(crate) fn ensure_understanding_md(
    zap_dir: &std::path::Path,
    cwd_name: Option<String>,
    files: usize,
    symbols: usize,
    lang_counts: &[(String, usize)],
) -> Result<()> {
    let path = zap_dir.join("understanding.md");
    let is_placeholder = match std::fs::read_to_string(&path) {
        Ok(s) => s.contains("This project has not yet been analysed"),
        Err(_) => true,
    };
    if path.exists() && !is_placeholder {
        return Ok(());
    }

    let content = if let Some(name) = cwd_name {
        let langs = if lang_counts.is_empty() {
            String::new()
        } else {
            let parts: Vec<String> = lang_counts.iter()
                .map(|(l, n)| format!("  - {l}: {n} symbols"))
                .collect();
            format!("\n## Languages\n{}\n", parts.join("\n"))
        };
        format!("\
# Understanding

Auto-generated from code index. Run `/init` for a detailed LLM-powered analysis.

## Project
{files} files · {symbols} symbols indexed · root: {name}
{langs}
## Structure
<!-- File-by-file analysis not yet run. Use /init to generate one. -->
")
    } else {
        "\
# Understanding

<!-- This file was auto-created by zap. Run `/init` or manually edit it to add project-specific knowledge. -->

## Overview

*This project has not yet been analysed. Run `/init` to generate a full understanding based on the code index.*
"
        .to_string()
    };

    std::fs::write(&path, content)?;
    Ok(())
}

/// Read project version from Cargo.toml or package.json.
fn read_project_version() -> Option<String> {
    if let Ok(s) = std::fs::read_to_string("Cargo.toml") {
        for line in s.lines() {
            let line = line.trim();
            if line.starts_with("version") {
                if let Some(v) = line.split('"').nth(1) {
                    return Some(v.to_string());
                }
            }
        }
    }
    if let Ok(s) = std::fs::read_to_string("package.json") {
        for line in s.lines() {
            let line = line.trim();
            if line.contains("\"version\"") {
                if let Some(v) = line.split('"').nth(3) {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Count .md files in src/default_skills/ (built-in skill count for zap projects).
fn count_builtin_skills() -> Option<usize> {
    let dir = std::path::Path::new("src").join("default_skills");
    if !dir.exists() {
        return None;
    }
    let count = std::fs::read_dir(&dir).ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
        .count();
    if count == 0 { None } else { Some(count) }
}

/// List top-level source modules and sub-directories (src/*.rs basenames + src/*/), capped at 50.
fn list_source_modules() -> String {
    let src = std::path::Path::new("src");
    if !src.exists() {
        return String::new();
    }
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in std::fs::read_dir(src).ok().into_iter().flatten().filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_dir() {
            if let Some(name) = p.file_name().map(|s| s.to_string_lossy().to_string()) {
                dirs.push(name);
            }
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            if let Some(name) = p.file_stem().map(|s| s.to_string_lossy().to_string()).filter(|n| n != "lib") {
                files.push(name);
            }
        }
    }

    dirs.sort();
    files.sort();
    dirs.extend(files);
    dirs.truncate(50);
    if dirs.is_empty() {
        return String::new();
    }
    format!("### Source modules\n  {}\n\n", dirs.join(", "))
}

/// Extract any LLM-written analysis from existing understanding.md,
/// preserving content that lives outside the auto-stats sentinels.
fn extract_analysis_section(existing: &str) -> String {
    if let Some(end) = existing.find("<!-- zap:auto-stats:end -->") {
        let after = existing[end + "<!-- zap:auto-stats:end -->".len()..].trim_start_matches('\n');
        if !after.trim().is_empty() {
            return format!("{}\n", after);
        }
        return String::new();
    }
    if existing.contains("## Architecture") || existing.contains("## Analysis") || existing.contains("## Overview") {
        if let Some(pos) = existing.find("\n## ").and_then(|p| existing[p + 1..].find("\n## ").map(|q| p + 1 + q)) {
            return format!("{}\n", existing[pos..].trim_start_matches('\n'));
        }
    }
    "## Analysis\n<!-- Run `/init` for a detailed LLM-powered analysis of architecture, patterns, and key modules. -->\n".to_string()
}

#[cfg(test)]
mod tests {
    use super::list_source_modules;

    #[test]
    fn list_source_modules_includes_directories() {
        let modules = list_source_modules();
        assert!(modules.contains("session"), "expected directory module in: {modules}");
    }
}
