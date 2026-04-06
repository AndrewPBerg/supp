use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use ignore::WalkBuilder;
use rayon::prelude::*;
use regex::Regex;
use serde::Serialize;

use crate::compress;

// ── Data structures ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TodoTag {
    Todo,
    Fixme,
    Hack,
    Xxx,
}

impl TodoTag {
    fn from_str_upper(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "TODO" => Some(Self::Todo),
            "FIXME" => Some(Self::Fixme),
            "HACK" => Some(Self::Hack),
            "XXX" => Some(Self::Xxx),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Todo => "TODO",
            Self::Fixme => "FIXME",
            Self::Hack => "HACK",
            Self::Xxx => "XXX",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BlameInfo {
    pub author: String,
    pub date: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TodoItem {
    pub tag: TodoTag,
    pub file: String,
    pub line: usize,
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blame: Option<BlameInfo>,
}

#[derive(Serialize)]
pub struct TodoResult {
    pub items: Vec<TodoItem>,
    pub plain: String,
    pub files_scanned: usize,
    pub tag_counts: HashMap<String, usize>,
}

// ── Public API ─────────────────────────────────────────────────────

pub fn parse_tags(raw: &[String]) -> Result<Vec<TodoTag>> {
    raw.iter()
        .map(|s| {
            TodoTag::from_str_upper(s)
                .ok_or_else(|| anyhow!("unknown tag '{}' (expected TODO, FIXME, HACK, or XXX)", s))
        })
        .collect()
}

pub fn scan(
    root: &str,
    regex_filter: Option<&str>,
    tags: Option<&[TodoTag]>,
    context_lines: usize,
    blame: bool,
) -> Result<TodoResult> {
    let re = regex_filter.map(Regex::new).transpose()?;
    // Tag must be the first word after comment markers (no preceding prose).
    // Case-sensitive: real annotations are uppercase (TODO:, FIXME:).
    let tag_re = Regex::new(r"^\W*(TODO|FIXME|HACK|XXX)\b[:\s]?\s*(.*)")?;

    // Collect file paths
    let mut file_paths: Vec<(PathBuf, String)> = Vec::new();
    let walker = WalkBuilder::new(root)
        .sort_by_file_name(|a, b| a.cmp(b))
        .build();

    for entry in walker.flatten() {
        if !entry.path().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .to_string();
        if let Some(ref re) = re
            && !re.is_match(&rel)
        {
            continue;
        }
        file_paths.push((entry.into_path(), rel));
    }

    let files_scanned = file_paths.len();

    // Scan files in parallel
    let mut items: Vec<TodoItem> = file_paths
        .par_iter()
        .flat_map(|(abs_path, rel_path)| scan_file(abs_path, rel_path, &tag_re, context_lines))
        .collect();

    // Filter by requested tags
    if let Some(tags) = tags {
        items.retain(|item| tags.contains(&item.tag));
    }

    // Git blame enrichment
    if blame {
        let repo_dir = find_repo_root(root);
        if let Some(ref repo_dir) = repo_dir {
            enrich_blame(repo_dir, &mut items);
        }
    }

    // Sort: by tag order, then by file, then by line
    items.sort_by(|a, b| {
        a.tag
            .cmp(&b.tag)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });

    // Build counts
    let mut tag_counts: HashMap<String, usize> = HashMap::new();
    for item in &items {
        *tag_counts.entry(item.tag.label().to_string()).or_insert(0) += 1;
    }

    // Build plain text
    let plain = build_plain(&items);

    Ok(TodoResult {
        items,
        plain,
        files_scanned,
        tag_counts,
    })
}

// ── File scanning ──────────────────────────────────────────────────

fn scan_file(
    abs_path: &Path,
    rel_path: &str,
    tag_re: &Regex,
    context_lines: usize,
) -> Vec<TodoItem> {
    let content = match std::fs::read_to_string(abs_path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let lines: Vec<&str> = content.lines().collect();

    let lang = compress::detect_lang(rel_path);
    if let Some(lang) = lang {
        scan_with_treesitter(&content, &lines, rel_path, lang, tag_re, context_lines)
    } else {
        scan_with_regex(&lines, rel_path, tag_re, context_lines)
    }
}

fn scan_with_treesitter(
    content: &str,
    lines: &[&str],
    rel_path: &str,
    lang: compress::Lang,
    tag_re: &Regex,
    context_lines: usize,
) -> Vec<TodoItem> {
    let tree = match compress::parse_source(content, lang) {
        Some(t) => t,
        None => return scan_with_regex(lines, rel_path, tag_re, context_lines),
    };

    let mut comment_ranges: Vec<(usize, usize)> = Vec::new();
    let mut cursor = tree.walk();
    compress::collect_comments(&mut cursor, lang, &mut comment_ranges);

    let mut items = Vec::new();

    for (start_byte, end_byte) in &comment_ranges {
        let comment_text = &content[*start_byte..*end_byte];

        // A comment node can span multiple lines; check each line for tags
        let start_line = content[..*start_byte].matches('\n').count();

        for (offset, line_text) in comment_text.lines().enumerate() {
            if let Some(caps) = tag_re.captures(line_text) {
                let tag_str = caps.get(1).unwrap().as_str();
                let text = caps.get(2).map_or("", |m| m.as_str()).trim().to_string();
                let line_num = start_line + offset + 1; // 1-based

                let context = extract_context(lines, line_num, context_lines);

                if let Some(tag) = TodoTag::from_str_upper(tag_str) {
                    items.push(TodoItem {
                        tag,
                        file: rel_path.to_string(),
                        line: line_num,
                        text,
                        context,
                        blame: None,
                    });
                }
            }
        }
    }

    items
}

fn scan_with_regex(
    lines: &[&str],
    rel_path: &str,
    tag_re: &Regex,
    context_lines: usize,
) -> Vec<TodoItem> {
    let mut items = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        if let Some(caps) = tag_re.captures(line) {
            let tag_str = caps.get(1).unwrap().as_str();
            let text = caps.get(2).map_or("", |m| m.as_str()).trim().to_string();
            let line_num = idx + 1;

            let context = extract_context(lines, line_num, context_lines);

            if let Some(tag) = TodoTag::from_str_upper(tag_str) {
                items.push(TodoItem {
                    tag,
                    file: rel_path.to_string(),
                    line: line_num,
                    text,
                    context,
                    blame: None,
                });
            }
        }
    }

    items
}

fn extract_context(lines: &[&str], line_num: usize, context_lines: usize) -> Vec<String> {
    if context_lines == 0 {
        return vec![];
    }
    let idx = line_num.saturating_sub(1); // 0-based
    let start = idx.saturating_sub(context_lines);
    let end = (idx + context_lines + 1).min(lines.len());
    lines[start..end].iter().map(|l| l.to_string()).collect()
}

// ── Git blame ──────────────────────────────────────────────────────

fn find_repo_root(path: &str) -> Option<PathBuf> {
    let abs = std::fs::canonicalize(path).ok()?;
    let dir = if abs.is_file() {
        abs.parent()?.to_path_buf()
    } else {
        abs
    };
    let mut current = dir;
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn enrich_blame(repo_dir: &Path, items: &mut [TodoItem]) {
    // Group items by file to batch blame calls
    let mut by_file: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        by_file.entry(item.file.clone()).or_default().push(i);
    }

    for (file, indices) in &by_file {
        if let Some(blame_map) = git_blame_file(repo_dir, file) {
            for &idx in indices {
                if let Some(info) = blame_map.get(&items[idx].line) {
                    items[idx].blame = Some(info.clone());
                }
            }
        }
    }
}

fn git_blame_file(repo_dir: &Path, file: &str) -> Option<HashMap<usize, BlameInfo>> {
    let output = std::process::Command::new("git")
        .args(["blame", "--porcelain", "--", file])
        .current_dir(repo_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_porcelain_blame(&text)
}

fn parse_porcelain_blame(text: &str) -> Option<HashMap<usize, BlameInfo>> {
    let mut map: HashMap<usize, BlameInfo> = HashMap::new();
    let mut current_line: Option<usize> = None;
    let mut author = String::new();
    let mut epoch: Option<i64> = None;

    for line in text.lines() {
        // Header line: <40-char sha> <orig_line> <final_line> [<num_lines>]
        if line.len() >= 40 && line.as_bytes().get(40) == Some(&b' ') {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                current_line = parts[2].parse().ok();
            }
        } else if let Some(rest) = line.strip_prefix("author ") {
            author = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("author-time ") {
            epoch = rest.parse().ok();
        } else if line.starts_with('\t') {
            // Content line — emit the entry
            if let (Some(ln), Some(ep)) = (current_line, epoch) {
                let date = chrono::DateTime::from_timestamp(ep, 0)
                    .map(|dt| dt.format("%Y-%m-%d").to_string())
                    .unwrap_or_default();
                map.insert(
                    ln,
                    BlameInfo {
                        author: author.clone(),
                        date,
                    },
                );
            }
            current_line = None;
            epoch = None;
        }
    }

    Some(map)
}

// ── Plain text output ──────────────────────────────────────────────

fn build_plain(items: &[TodoItem]) -> String {
    let mut out = String::new();
    let mut current_tag: Option<TodoTag> = None;

    for item in items {
        if current_tag != Some(item.tag) {
            if current_tag.is_some() {
                out.push('\n');
            }
            out.push_str(item.tag.label());
            out.push('\n');
            current_tag = Some(item.tag);
        }

        out.push_str(&format!("  {}:{} — {}", item.file, item.line, item.text));
        if let Some(ref blame) = item.blame {
            out.push_str(&format!("  ({}, {})", blame.author, blame.date));
        }
        out.push('\n');

        if !item.context.is_empty() {
            for ctx_line in &item.context {
                out.push_str(&format!("    {}\n", ctx_line));
            }
        }
    }

    out
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tags_valid() {
        let tags = parse_tags(&["TODO".into(), "fixme".into()]).unwrap();
        assert_eq!(tags, vec![TodoTag::Todo, TodoTag::Fixme]);
    }

    #[test]
    fn parse_tags_invalid() {
        assert!(parse_tags(&["NOPE".into()]).is_err());
    }

    #[test]
    fn tag_ordering() {
        assert!(TodoTag::Todo < TodoTag::Fixme);
        assert!(TodoTag::Fixme < TodoTag::Hack);
        assert!(TodoTag::Hack < TodoTag::Xxx);
    }

    #[test]
    fn regex_matches_todo_comment() {
        let tag_re = Regex::new(r"^\W*(TODO|FIXME|HACK|XXX)\b[:\s]?\s*(.*)").unwrap();

        let caps = tag_re.captures("// TODO: implement this").unwrap();
        assert_eq!(caps.get(1).unwrap().as_str(), "TODO");
        assert_eq!(caps.get(2).unwrap().as_str().trim(), "implement this");

        let caps = tag_re.captures("# FIXME handle edge case").unwrap();
        assert_eq!(caps.get(1).unwrap().as_str(), "FIXME");

        let caps = tag_re.captures("/* HACK: workaround */").unwrap();
        assert_eq!(caps.get(1).unwrap().as_str(), "HACK");

        let caps = tag_re.captures("// XXX this is bad").unwrap();
        assert_eq!(caps.get(1).unwrap().as_str(), "XXX");
    }

    #[test]
    fn regex_does_not_match_substring() {
        let tag_re = Regex::new(r"^\W*(TODO|FIXME|HACK|XXX)\b[:\s]?\s*(.*)").unwrap();
        assert!(tag_re.captures("TODOLIST").is_none());
    }

    #[test]
    fn regex_does_not_match_mid_sentence() {
        let tag_re = Regex::new(r"^\W*(TODO|FIXME|HACK|XXX)\b[:\s]?\s*(.*)").unwrap();
        // Tags mentioned in prose should not match as annotations
        assert!(
            tag_re
                .captures("/// Find TODO, FIXME, and HACK comments")
                .is_none()
        );
        assert!(tag_re.captures("// ── Todo flags ───").is_none());
    }

    #[test]
    fn extract_context_basic() {
        let lines = vec!["a", "b", "c", "d", "e"];
        let ctx = extract_context(&lines, 3, 1); // line 3 (0-based idx 2), 1 context line
        assert_eq!(ctx, vec!["b", "c", "d"]);
    }

    #[test]
    fn extract_context_at_start() {
        let lines = vec!["a", "b", "c"];
        let ctx = extract_context(&lines, 1, 2);
        assert_eq!(ctx, vec!["a", "b", "c"]);
    }

    #[test]
    fn extract_context_zero() {
        let lines = vec!["a", "b", "c"];
        let ctx = extract_context(&lines, 2, 0);
        assert!(ctx.is_empty());
    }

    #[test]
    fn scan_regex_fallback() {
        let lines = vec![
            "#!/bin/bash",
            "# TODO: fix this script",
            "echo hello",
            "# FIXME handle errors",
        ];
        let tag_re = Regex::new(r"^\W*(TODO|FIXME|HACK|XXX)\b[:\s]?\s*(.*)").unwrap();
        let items = scan_with_regex(&lines, "test.sh", &tag_re, 0);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].tag, TodoTag::Todo);
        assert_eq!(items[0].line, 2);
        assert_eq!(items[1].tag, TodoTag::Fixme);
        assert_eq!(items[1].line, 4);
    }

    #[test]
    fn porcelain_blame_parsing() {
        let porcelain = "\
abc123456789012345678901234567890123abcd 1 1 1
author Alice
author-mail <alice@example.com>
author-time 1700000000
author-tz +0000
committer Alice
committer-mail <alice@example.com>
committer-time 1700000000
committer-tz +0000
summary initial commit
filename test.rs
\t// TODO: fix this
";
        let map = parse_porcelain_blame(porcelain).unwrap();
        assert_eq!(map.len(), 1);
        let info = map.get(&1).unwrap();
        assert_eq!(info.author, "Alice");
        assert_eq!(info.date, "2023-11-14");
    }
}
