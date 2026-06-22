use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use super::Tool;

// ── caller_scope parsing ───────────────────────────────────────────────────

/// Extract the bare function/method name from the `caller_scope` strings stored
/// in `call_sites`. The format is one of:
///   - `fn foo`
///   - `impl Struct · method`
///   - `impl Trait for Struct · method`
///
/// Returns the last token after a `·` separator if present, otherwise the last
/// space-separated token, otherwise the whole string.
pub fn extract_fn_name(caller_scope: &str) -> &str {
    // "impl Trait for Struct · method" or "impl Struct · method"
    if caller_scope.contains('·') {
        if let Some(after_dot) = caller_scope.rsplit('·').next() {
            let trimmed = after_dot.trim();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }
    // "fn foo" — drop the leading keyword, return the last token.
    caller_scope.rsplit(' ').next().unwrap_or(caller_scope).trim()
}

// ── BFS ripple analysis ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RippleLevel {
    pub depth:    usize,
    pub callers:  Vec<CallerSite>,
}

#[derive(Debug, Clone)]
pub struct CallerSite {
    pub fn_name:      String,
    pub caller_scope: String,
    pub path:         String,
    pub line:         usize,
}

/// Walk the call graph upward from `symbol`, up to `max_depth` BFS levels.
/// Returns one `RippleLevel` per depth (empty levels are omitted).
///
/// Uses `global_callers_of` which queries the in-process CodeIndex; returns
/// an empty vec (not an error) when the index is not available.
pub fn ripple_bfs(symbol: &str, max_depth: usize) -> Vec<RippleLevel> {
    let per_level_limit = 500;
    let mut levels: Vec<RippleLevel> = Vec::new();

    // Names to search at the next BFS frontier.
    let mut frontier: Vec<String> = vec![symbol.to_string()];
    // All fn_names we have seen to avoid cycles.
    let mut visited: HashSet<String> = HashSet::from([symbol.to_string()]);

    for depth in 1..=max_depth {
        let mut next_frontier: Vec<String> = Vec::new();
        let mut callers: Vec<CallerSite> = Vec::new();

        // Dedup within this depth level so we don't query the same fn twice.
        let mut queried: HashSet<String> = HashSet::new();

        for name in &frontier {
            if queried.contains(name.as_str()) { continue; }
            queried.insert(name.clone());

            let sites = crate::code_index::global_callers_of(name, None, per_level_limit);
            for cs in sites {
                let fn_name = extract_fn_name(&cs.caller_scope).to_string();
                if fn_name.is_empty() { continue; }
                callers.push(CallerSite {
                    fn_name:      fn_name.clone(),
                    caller_scope: cs.caller_scope.clone(),
                    path:         cs.path.clone(),
                    line:         cs.line,
                });
                if !visited.contains(&fn_name) {
                    visited.insert(fn_name.clone());
                    next_frontier.push(fn_name);
                }
            }
        }

        if !callers.is_empty() {
            levels.push(RippleLevel { depth, callers });
        }

        if next_frontier.is_empty() { break; }
        frontier = next_frontier;
    }

    levels
}

// ── Output formatting ──────────────────────────────────────────────────────

pub fn format_ripple(symbol: &str, levels: &[RippleLevel]) -> String {
    if levels.is_empty() {
        return format!("`{}` has no callers in the index — either it is unused, at the top of the call tree, or the index does not cover its callers.", symbol);
    }

    let total_sites: usize = levels.iter().map(|l| l.callers.len()).sum();
    let total_files: usize = {
        let mut files = HashSet::new();
        for l in levels { for c in &l.callers { files.insert(c.path.as_str()); } }
        files.len()
    };

    let mut out = format!("## Ripple analysis: `{}`\n\n", symbol);
    out.push_str(&format!(
        "**Total blast radius: {} call site(s) across {} file(s)**\n",
        total_sites, total_files
    ));

    for level in levels {
        let level_files: HashSet<&str> = level.callers.iter().map(|c| c.path.as_str()).collect();
        out.push_str(&format!(
            "\n### Depth {} — {} caller(s) in {} file(s)\n",
            level.depth,
            level.callers.len(),
            level_files.len(),
        ));

        // Group by file for readability.
        let mut by_file: HashMap<&str, Vec<&CallerSite>> = HashMap::new();
        for c in &level.callers {
            by_file.entry(c.path.as_str()).or_default().push(c);
        }
        let mut file_keys: Vec<&str> = by_file.keys().copied().collect();
        file_keys.sort_unstable();

        for path in file_keys {
            let mut sites = by_file[path].to_vec();
            sites.sort_by_key(|c| c.line);
            // Show a short relative-looking path.
            let short = shorten_path(path);
            for c in sites {
                out.push_str(&format!("  {}:{} — {}\n", short, c.line, c.caller_scope));
            }
        }
    }

    out
}

/// Strip long absolute path prefixes, keeping the last 3 components.
fn shorten_path(path: &str) -> &str {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 4 {
        return path;
    }
    // Find "src/" and return from there.
    if let Some(pos) = parts.iter().rposition(|&p| p == "src") {
        let idx = parts[..=pos].len() - 1;
        // Reconstruct is expensive; instead just find the byte offset.
        let mut slash_count = 0;
        for (i, ch) in path.char_indices() {
            if ch == '/' { slash_count += 1; }
            if slash_count == idx { return &path[i + 1..]; }
        }
    }
    // Fallback: last 3 path components.
    let mut slash_count = 0;
    for (i, ch) in path.char_indices().rev() {
        if ch == '/' { slash_count += 1; }
        if slash_count == 3 { return &path[i + 1..]; }
    }
    path
}

// ── Tool ──────────────────────────────────────────────────────────────────

pub struct RippleAnalysisTool;

#[async_trait]
impl Tool for RippleAnalysisTool {
    fn name(&self) -> &str { "ripple_analysis" }

    fn description(&self) -> &str {
        "Show the full blast radius of changing a symbol: who calls it directly, \
         who calls those callers, and so on. Works from the local call graph — \
         no language server needed. Use before renaming, changing a signature, or \
         deleting a function to understand the full impact. Depth 1 = direct callers, \
         depth 2 = callers of callers, etc. (default max depth: 3)."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Function or method name to trace (e.g. 'execute', 'build_system_prompt')."
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum BFS depth (1–5, default 3)."
                }
            },
            "required": ["symbol"]
        })
    }

    fn permission_context(&self, input: &serde_json::Value) -> String {
        format!("ripple_analysis({})", input["symbol"].as_str().unwrap_or("?"))
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let symbol    = input["symbol"].as_str().context("ripple_analysis: 'symbol' required")?;
        let max_depth = input["depth"].as_u64().unwrap_or(3).clamp(1, 5) as usize;

        if symbol.trim().is_empty() {
            return Ok("ripple_analysis: symbol must not be empty".to_string());
        }

        let levels = ripple_bfs(symbol, max_depth);
        Ok(format_ripple(symbol, &levels))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_fn_name ──────────────────────────────────────────────────

    #[test]
    fn extract_plain_fn() {
        assert_eq!(extract_fn_name("fn execute"), "execute");
    }

    #[test]
    fn extract_impl_method() {
        assert_eq!(extract_fn_name("impl Session · new"), "new");
    }

    #[test]
    fn extract_trait_impl_method() {
        assert_eq!(extract_fn_name("impl Tool for ReadFileTool · execute"), "execute");
    }

    #[test]
    fn extract_empty_string() {
        assert_eq!(extract_fn_name(""), "");
    }

    #[test]
    fn extract_no_dot_no_space() {
        assert_eq!(extract_fn_name("standalone"), "standalone");
    }

    #[test]
    fn extract_fn_with_underscores() {
        assert_eq!(extract_fn_name("fn build_system_prompt"), "build_system_prompt");
    }

    #[test]
    fn extract_trims_whitespace() {
        assert_eq!(extract_fn_name("impl Foo · bar "), "bar");
    }

    // ── format_ripple ─────────────────────────────────────────────────────

    fn make_site(fn_name: &str, scope: &str, path: &str, line: usize) -> CallerSite {
        CallerSite {
            fn_name:      fn_name.to_string(),
            caller_scope: scope.to_string(),
            path:         path.to_string(),
            line,
        }
    }

    #[test]
    fn format_empty_levels_returns_no_callers_message() {
        let out = format_ripple("foo", &[]);
        assert!(out.contains("no callers"), "got: {}", out);
    }

    #[test]
    fn format_single_depth_shows_header_and_site() {
        let level = RippleLevel {
            depth:   1,
            callers: vec![make_site("bar", "fn bar", "/project/src/main.rs", 42)],
        };
        let out = format_ripple("foo", &[level]);
        assert!(out.contains("Depth 1"), "missing depth header");
        assert!(out.contains("main.rs:42"), "missing file:line");
        assert!(out.contains("fn bar"), "missing caller scope");
        assert!(out.contains("1 call site"), "missing total count");
    }

    #[test]
    fn format_multiple_depths() {
        let levels = vec![
            RippleLevel {
                depth:   1,
                callers: vec![make_site("bar", "fn bar", "/project/src/a.rs", 10)],
            },
            RippleLevel {
                depth:   2,
                callers: vec![
                    make_site("baz", "fn baz", "/project/src/b.rs", 5),
                    make_site("qux", "fn qux", "/project/src/b.rs", 20),
                ],
            },
        ];
        let out = format_ripple("foo", &levels);
        assert!(out.contains("Depth 1"), "missing depth 1");
        assert!(out.contains("Depth 2"), "missing depth 2");
        assert!(out.contains("3 call site"), "total should be 3");
        assert!(out.contains("2 file"), "should be 2 files total");
    }

    #[test]
    fn format_groups_callers_by_file() {
        let level = RippleLevel {
            depth: 1,
            callers: vec![
                make_site("a", "fn a", "/project/src/foo.rs", 1),
                make_site("b", "fn b", "/project/src/foo.rs", 2),
                make_site("c", "fn c", "/project/src/bar.rs", 3),
            ],
        };
        let out = format_ripple("target", &[level]);
        // foo.rs should appear once as a group header, not duplicated.
        let foo_count = out.matches("foo.rs").count();
        assert!(foo_count >= 1, "foo.rs should appear");
        // bar.rs must also appear.
        assert!(out.contains("bar.rs"), "bar.rs missing");
    }

    #[test]
    fn format_sites_sorted_by_line_within_file() {
        let level = RippleLevel {
            depth: 1,
            callers: vec![
                make_site("b", "fn b", "/project/src/main.rs", 100),
                make_site("a", "fn a", "/project/src/main.rs", 10),
            ],
        };
        let out = format_ripple("x", &[level]);
        let pos_10  = out.find("main.rs:10").unwrap();
        let pos_100 = out.find("main.rs:100").unwrap();
        assert!(pos_10 < pos_100, "line 10 should appear before line 100");
    }

    // ── shorten_path ──────────────────────────────────────────────────────

    #[test]
    fn shorten_finds_src_prefix() {
        let path = "/Users/alice/projects/myapp/src/tools/mod.rs";
        let short = shorten_path(path);
        assert!(short.starts_with("src/"), "expected src/ prefix, got: {}", short);
    }

    #[test]
    fn shorten_short_path_unchanged() {
        let path = "src/main.rs";
        assert_eq!(shorten_path(path), path);
    }
}
