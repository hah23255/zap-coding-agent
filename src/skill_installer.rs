/// Skill installer — fetch and manage user-installed skills.
///
/// Handles `zap skill install / uninstall / list`.
/// Skills are single `.md` files placed in:
///   ~/.zap/skills/<slug>.md   (global, default)
///   .zap/skills/<slug>.md     (project-local, with --local)
use anyhow::{bail, Context, Result};
use colored::Colorize;
use std::path::PathBuf;

fn global_skills_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".zap").join("skills"))
}

fn local_skills_dir() -> PathBuf {
    PathBuf::from(".zap").join("skills")
}

fn skills_dir(local: bool) -> Result<PathBuf> {
    if local {
        Ok(local_skills_dir())
    } else {
        global_skills_dir()
    }
}

/// Resolve a source string to (raw_url, slug).
///
/// Accepted forms:
///   user/repo              → HEAD/SKILL.md, slug = repo name
///   user/repo/path/s.md   → HEAD/path/s.md, slug = "s"
///   https://...            → used as-is, slug = filename without .md
fn resolve_url(source: &str) -> Result<(String, String)> {
    if source.starts_with("https://") || source.starts_with("http://") {
        let slug = source
            .split('/')
            .next_back()
            .unwrap_or("skill")
            .trim_end_matches(".md")
            .to_string();
        if slug.is_empty() {
            bail!("Cannot derive a skill name from that URL");
        }
        return Ok((source.to_string(), slug));
    }

    // GitHub shorthand
    let parts: Vec<&str> = source.splitn(3, '/').collect();
    match parts.as_slice() {
        [user, repo] => {
            let url = format!(
                "https://raw.githubusercontent.com/{}/{}/HEAD/SKILL.md",
                user, repo
            );
            Ok((url, repo.to_string()))
        }
        [user, repo, path] => {
            let url = format!(
                "https://raw.githubusercontent.com/{}/{}/HEAD/{}",
                user, repo, path
            );
            let slug = path
                .split('/')
                .next_back()
                .unwrap_or("skill")
                .trim_end_matches(".md")
                .to_string();
            if slug.is_empty() {
                bail!("Cannot derive a skill name from that path");
            }
            Ok((url, slug))
        }
        _ => bail!(
            "Invalid source '{}'. Use a URL or GitHub shorthand (user/repo or user/repo/path/skill.md).",
            source
        ),
    }
}

pub fn install(source: &str, local: bool) -> Result<()> {
    let (url, slug) = resolve_url(source)?;

    println!("Fetching {}…", url.dimmed());

    let resp = reqwest::blocking::get(&url)
        .with_context(|| format!("Network error fetching {}", url))?;

    if !resp.status().is_success() {
        bail!("HTTP {} — {}", resp.status(), url);
    }

    let content = resp.text().context("Failed to read response body")?;

    if content.trim().is_empty() {
        bail!("Downloaded file is empty — check the source URL");
    }

    let dir = skills_dir(local)?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Cannot create {}", dir.display()))?;

    let dest = dir.join(format!("{}.md", slug));
    std::fs::write(&dest, &content)
        .with_context(|| format!("Cannot write {}", dest.display()))?;

    let scope = if local { "local" } else { "global" };
    println!(
        "{} installed {} {} {}",
        "✓".green().bold(),
        slug.cyan().bold(),
        "→".dimmed(),
        format!("{} ({})", dest.display(), scope).dimmed(),
    );
    println!(
        "  Invoke it inside zap with {}",
        format!("/{}", slug).cyan()
    );
    Ok(())
}

pub fn uninstall(name: &str, local: bool) -> Result<()> {
    let dir = skills_dir(local)?;
    let path = dir.join(format!("{}.md", name));

    if !path.exists() {
        // Helpful hint if it exists in the other scope
        let other_dir = if local {
            global_skills_dir()?
        } else {
            local_skills_dir()
        };
        let other_path = other_dir.join(format!("{}.md", name));
        if other_path.exists() {
            let other_scope = if local { "global" } else { "local" };
            bail!(
                "Skill '{}' not found in {} scope — it exists as a {} skill.\n  Re-run {}.",
                name,
                if local { "local" } else { "global" },
                other_scope,
                if local {
                    "without --local to remove the global copy"
                } else {
                    "with --local to remove the local copy"
                }
            );
        }
        bail!("Skill '{}' is not installed", name);
    }

    std::fs::remove_file(&path)
        .with_context(|| format!("Cannot remove {}", path.display()))?;

    println!("{} removed skill '{}'", "✓".green().bold(), name.cyan().bold());
    Ok(())
}

pub fn list() -> Result<()> {
    let global_dir = global_skills_dir()?;
    let local_dir = local_skills_dir();
    let mut found = false;

    // Local first (higher runtime priority)
    if local_dir.exists() {
        let mut entries: Vec<_> = std::fs::read_dir(&local_dir)
            .with_context(|| format!("Cannot read {}", local_dir.display()))?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x == "md")
                    .unwrap_or(false)
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let slug = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?");
            println!(
                "{}  {:<24}  {}",
                "[local] ".cyan().bold(),
                slug,
                path.display().to_string().dimmed()
            );
            found = true;
        }
    }

    // Global
    if global_dir.exists() {
        let mut entries: Vec<_> = std::fs::read_dir(&global_dir)
            .with_context(|| format!("Cannot read {}", global_dir.display()))?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x == "md")
                    .unwrap_or(false)
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let slug = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?");
            println!(
                "{}  {:<24}  {}",
                "[global]".dimmed(),
                slug,
                path.display().to_string().dimmed()
            );
            found = true;
        }
    }

    if !found {
        println!(
            "{}",
            "No skills installed yet.\n  Try: zap skill install user/repo".dimmed()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_url_bare_repo() {
        let (url, slug) = resolve_url("alice/my-skills").unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/alice/my-skills/HEAD/SKILL.md"
        );
        assert_eq!(slug, "my-skills");
    }

    #[test]
    fn resolve_url_repo_with_path() {
        let (url, slug) = resolve_url("alice/skills-pack/review.md").unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/alice/skills-pack/HEAD/review.md"
        );
        assert_eq!(slug, "review");
    }

    #[test]
    fn resolve_url_https() {
        let src = "https://example.com/path/to/debug-rust.md";
        let (url, slug) = resolve_url(src).unwrap();
        assert_eq!(url, src);
        assert_eq!(slug, "debug-rust");
    }

    #[test]
    fn resolve_url_invalid() {
        assert!(resolve_url("not-a-valid-source!!!").is_err());
    }
}
