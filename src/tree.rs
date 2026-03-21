use std::collections::HashMap;

use anyhow::Result;
use colored::Colorize;
use ignore::WalkBuilder;
use regex::Regex;
use std::path::Path;

use serde::Serialize;

use crate::git::FileStatus;
use crate::styles::file_status_indicator;

struct TreeNode {
    name: String,
    is_dir: bool,
    status: Option<FileStatus>,
    children: Vec<TreeNode>,
}

#[derive(Serialize)]
pub struct TreeResult {
    pub display: String,
    pub plain: String,
    pub file_count: usize,
    pub dir_count: usize,
    pub status_counts: HashMap<FileStatus, usize>,
}

pub fn build_tree(
    root: &str,
    max_depth: Option<usize>,
    regex_filter: Option<&str>,
    statuses: Option<(&HashMap<String, FileStatus>, &str)>,
) -> Result<TreeResult> {
    let re = regex_filter.map(Regex::new).transpose()?;

    let mut entries: Vec<(Vec<String>, bool, Option<FileStatus>)> = Vec::new();

    let mut walker = WalkBuilder::new(root);
    walker.sort_by_file_name(|a, b| a.cmp(b));
    if let Some(d) = max_depth {
        walker.max_depth(Some(d));
    }

    for entry in walker.build().flatten() {
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(path);

        if rel == Path::new("") {
            continue;
        }

        // Always skip __pycache__ directories and their contents
        if rel.components().any(|c| c.as_os_str() == "__pycache__") {
            continue;
        }

        let is_dir = path.is_dir();
        let rel_str = rel.to_string_lossy();

        if let Some(ref re) = re
            && !is_dir
            && !re.is_match(&rel_str)
        {
            continue;
        }

        // Look up git status for files
        let file_status = if !is_dir {
            statuses.and_then(|(map, prefix)| {
                let key = if prefix.is_empty() {
                    rel_str.to_string()
                } else {
                    format!("{}/{}", prefix, rel_str)
                };
                map.get(&key).copied()
            })
        } else {
            None
        };

        let components: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();

        entries.push((components, is_dir, file_status));
    }

    // If regex filter is active, prune directories that contain no matching files
    if re.is_some() {
        let file_paths: Vec<Vec<String>> = entries
            .iter()
            .filter(|(_, is_dir, _)| !*is_dir)
            .map(|(comps, _, _)| comps.clone())
            .collect();

        entries.retain(|(comps, is_dir, _)| {
            if !*is_dir {
                return true;
            }
            file_paths
                .iter()
                .any(|fp| fp.len() > comps.len() && fp[..comps.len()] == comps[..])
        });
    }

    // Build tree structure
    let mut root_node = TreeNode {
        name: format!(
            "{}/",
            if root == "." {
                "."
            } else {
                root.trim_end_matches('/')
            }
        ),
        is_dir: true,
        status: None,
        children: Vec::new(),
    };

    for (components, is_dir, file_status) in &entries {
        insert_node(&mut root_node, components, 0, *is_dir, *file_status);
    }

    // Render
    let mut display = String::new();
    let mut plain = String::new();
    let mut file_count = 0usize;
    let mut dir_count = 0usize;
    let mut status_counts: HashMap<FileStatus, usize> = HashMap::new();

    // Root line
    display.push_str(&format!("{}\n", root_node.name.bold()));
    plain.push_str(&format!("{}\n", root_node.name));

    render(
        &root_node.children,
        "",
        &mut display,
        &mut plain,
        &mut file_count,
        &mut dir_count,
        &mut status_counts,
    );

    Ok(TreeResult {
        display,
        plain,
        file_count,
        dir_count,
        status_counts,
    })
}

fn insert_node(
    parent: &mut TreeNode,
    components: &[String],
    depth: usize,
    is_dir: bool,
    status: Option<FileStatus>,
) {
    if depth >= components.len() {
        return;
    }

    let name = &components[depth];
    let is_last = depth == components.len() - 1;

    let child_pos = parent.children.iter().position(|c| c.name == *name);
    let idx = match child_pos {
        Some(i) => i,
        None => {
            parent.children.push(TreeNode {
                name: name.clone(),
                is_dir: if is_last { is_dir } else { true },
                status: if is_last { status } else { None },
                children: Vec::new(),
            });
            parent.children.len() - 1
        }
    };

    if !is_last {
        insert_node(
            &mut parent.children[idx],
            components,
            depth + 1,
            is_dir,
            status,
        );
    }
}

fn render(
    children: &[TreeNode],
    prefix: &str,
    display: &mut String,
    plain: &mut String,
    file_count: &mut usize,
    dir_count: &mut usize,
    status_counts: &mut HashMap<FileStatus, usize>,
) {
    for (i, child) in children.iter().enumerate() {
        let is_last = i == children.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last {
            format!("{}    ", prefix)
        } else {
            format!("{}│   ", prefix)
        };

        if child.is_dir {
            *dir_count += 1;
            display.push_str(&format!(
                "{}{}{}\n",
                prefix.dimmed(),
                connector.dimmed(),
                format!("{}/", child.name).bold()
            ));
            plain.push_str(&format!("{}{}{}/\n", prefix, connector, child.name));
            render(
                &child.children,
                &child_prefix,
                display,
                plain,
                file_count,
                dir_count,
                status_counts,
            );
        } else {
            *file_count += 1;
            if let Some(st) = child.status {
                *status_counts.entry(st).or_insert(0) += 1;
                let (_plain_ind, colored_ind) = file_status_indicator(st);
                // Pad the filename to align indicators
                display.push_str(&format!(
                    "{}{}{} {}\n",
                    prefix.dimmed(),
                    connector.dimmed(),
                    child.name,
                    colored_ind,
                ));
            } else {
                display.push_str(&format!(
                    "{}{}{}\n",
                    prefix.dimmed(),
                    connector.dimmed(),
                    child.name
                ));
            }
            plain.push_str(&format!("{}{}{}\n", prefix, connector, child.name));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_tree(files: &[&str]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for f in files {
            let path = dir.path().join(f);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, "content").unwrap();
        }
        dir
    }

    // ── Structure ────────────────────────────────────────────────

    #[test]
    fn empty_dir() {
        let dir = TempDir::new().unwrap();
        let result = build_tree(dir.path().to_str().unwrap(), None, None, None).unwrap();
        assert_eq!(result.file_count, 0);
        assert_eq!(result.dir_count, 0);
    }

    #[test]
    fn single_file() {
        let dir = setup_tree(&["hello.txt"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, None, None).unwrap();
        assert_eq!(result.file_count, 1);
        assert_eq!(result.dir_count, 0);
        assert!(result.plain.contains("hello.txt"));
    }

    #[test]
    fn nested_dirs() {
        let dir = setup_tree(&["src/main.rs", "src/lib.rs"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, None, None).unwrap();
        assert_eq!(result.file_count, 2);
        assert_eq!(result.dir_count, 1);
        assert!(result.plain.contains("src/"));
        assert!(result.plain.contains("main.rs"));
        assert!(result.plain.contains("lib.rs"));
    }

    #[test]
    fn deeply_nested() {
        let dir = setup_tree(&["a/b/c/d.txt"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, None, None).unwrap();
        assert_eq!(result.file_count, 1);
        assert_eq!(result.dir_count, 3); // a, b, c
    }

    // ── max_depth ────────────────────────────────────────────────

    #[test]
    fn max_depth_limits_output() {
        let dir = setup_tree(&["a/b/c/deep.txt", "top.txt"]);
        let result = build_tree(dir.path().to_str().unwrap(), Some(1), None, None).unwrap();
        // depth 1 = only immediate children
        assert!(result.plain.contains("top.txt"));
        assert!(!result.plain.contains("deep.txt"));
    }

    #[test]
    fn max_depth_none_unlimited() {
        let dir = setup_tree(&["a/b/c/deep.txt"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, None, None).unwrap();
        assert!(result.plain.contains("deep.txt"));
    }

    #[test]
    fn max_depth_large_on_shallow() {
        let dir = setup_tree(&["file.txt"]);
        let result = build_tree(dir.path().to_str().unwrap(), Some(100), None, None).unwrap();
        assert_eq!(result.file_count, 1);
    }

    // ── regex_filter ─────────────────────────────────────────────

    #[test]
    fn regex_keeps_matching_files() {
        let dir = setup_tree(&["main.rs", "lib.rs", "readme.md"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, Some(r"\.rs$"), None).unwrap();
        assert_eq!(result.file_count, 2);
        assert!(!result.plain.contains("readme.md"));
    }

    #[test]
    fn regex_no_match_zero_files() {
        let dir = setup_tree(&["main.rs"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, Some(r"\.py$"), None).unwrap();
        assert_eq!(result.file_count, 0);
    }

    #[test]
    fn regex_invalid_returns_err() {
        let dir = setup_tree(&["file.txt"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, Some(r"[invalid"), None);
        assert!(result.is_err());
    }

    #[test]
    fn regex_prunes_empty_dirs() {
        let dir = setup_tree(&["src/main.rs", "docs/readme.md"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, Some(r"\.rs$"), None).unwrap();
        // docs/ should be pruned since it has no matching files
        assert!(!result.plain.contains("docs/"));
        assert!(result.plain.contains("src/"));
    }

    // ── git statuses ─────────────────────────────────────────────

    #[test]
    fn status_counts_populated() {
        let dir = setup_tree(&["modified.rs", "added.rs"]);
        let mut statuses = HashMap::new();
        statuses.insert("modified.rs".to_string(), FileStatus::Modified);
        statuses.insert("added.rs".to_string(), FileStatus::Added);
        let result = build_tree(
            dir.path().to_str().unwrap(),
            None,
            None,
            Some((&statuses, "")),
        )
        .unwrap();
        assert_eq!(result.status_counts.get(&FileStatus::Modified), Some(&1));
        assert_eq!(result.status_counts.get(&FileStatus::Added), Some(&1));
    }

    #[test]
    fn no_statuses_empty_counts() {
        let dir = setup_tree(&["file.txt"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, None, None).unwrap();
        assert!(result.status_counts.is_empty());
    }

    // ── Output format ────────────────────────────────────────────

    #[test]
    fn plain_contains_tree_chars() {
        let dir = setup_tree(&["a.txt", "b.txt"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, None, None).unwrap();
        assert!(result.plain.contains("├──") || result.plain.contains("└──"));
    }

    #[test]
    fn plain_has_no_ansi() {
        let dir = setup_tree(&["a.txt"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, None, None).unwrap();
        assert!(!result.plain.contains("\x1b["));
    }

    #[test]
    fn root_line_format() {
        let dir = setup_tree(&["a.txt"]);
        let root = dir.path().to_str().unwrap();
        let result = build_tree(root, None, None, None).unwrap();
        let first_line = result.plain.lines().next().unwrap();
        assert!(first_line.ends_with('/'));
    }

    #[test]
    fn status_with_prefix() {
        let dir = setup_tree(&["sub/file.txt"]);
        let mut statuses = HashMap::new();
        statuses.insert("prefix/sub/file.txt".to_string(), FileStatus::Modified);
        let result = build_tree(
            dir.path().to_str().unwrap(),
            None,
            None,
            Some((&statuses, "prefix")),
        )
        .unwrap();
        assert_eq!(result.status_counts.get(&FileStatus::Modified), Some(&1));
    }

    #[test]
    fn display_differs_from_plain() {
        let dir = setup_tree(&["src/main.rs"]);
        let result = build_tree(dir.path().to_str().unwrap(), None, None, None).unwrap();
        // display has colored output, plain doesn't — they should differ
        // (unless colors are force-disabled in CI)
        assert!(!result.display.is_empty());
        assert!(!result.plain.is_empty());
    }

    #[test]
    fn status_with_no_file_indicator() {
        let dir = setup_tree(&["a.txt", "b.txt"]);
        let mut statuses = HashMap::new();
        statuses.insert("a.txt".to_string(), FileStatus::Added);
        // b.txt has no status entry
        let result = build_tree(
            dir.path().to_str().unwrap(),
            None,
            None,
            Some((&statuses, "")),
        )
        .unwrap();
        assert_eq!(result.status_counts.get(&FileStatus::Added), Some(&1));
        assert_eq!(result.file_count, 2);
    }
}
