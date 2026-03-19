use std::collections::HashMap;

use anyhow::Result;
use colored::Colorize;
use ignore::WalkBuilder;
use regex::Regex;
use std::path::Path;

use crate::git::FileStatus;
use crate::styles::file_status_indicator;

struct TreeNode {
    name: String,
    is_dir: bool,
    status: Option<FileStatus>,
    children: Vec<TreeNode>,
}

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

        let components: Vec<String> = rel.components()
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
            file_paths.iter().any(|fp| {
                fp.len() > comps.len() && fp[..comps.len()] == comps[..]
            })
        });
    }

    // Build tree structure
    let mut root_node = TreeNode {
        name: format!("{}/", if root == "." { "." } else { root.trim_end_matches('/') }),
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

    render(&root_node.children, "", &mut display, &mut plain, &mut file_count, &mut dir_count, &mut status_counts);

    Ok(TreeResult { display, plain, file_count, dir_count, status_counts })
}

fn insert_node(parent: &mut TreeNode, components: &[String], depth: usize, is_dir: bool, status: Option<FileStatus>) {
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
        insert_node(&mut parent.children[idx], components, depth + 1, is_dir, status);
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
            render(&child.children, &child_prefix, display, plain, file_count, dir_count, status_counts);
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
