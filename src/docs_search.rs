use std::path::Path;

use anyhow::Result;
use ignore::WalkBuilder;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DocsMatch {
    pub file: String,
    pub line: usize,
    pub kind: String,
    pub text: String,
    pub score: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocsResult {
    pub query: String,
    pub matches: Vec<DocsMatch>,
    pub files_scanned: usize,
}

pub fn search(root: &str, query: &[String], limit: usize) -> Result<DocsResult> {
    let tokens: Vec<String> = query
        .iter()
        .flat_map(|q| q.split_whitespace())
        .map(|q| q.to_lowercase())
        .filter(|q| !q.is_empty())
        .collect();

    let mut matches = Vec::new();
    let mut files_scanned = 0;

    for entry in WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .sort_by_file_name(|a, b| a.cmp(b))
        .build()
        .flatten()
    {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        if !is_relevant_file(&rel) {
            continue;
        }
        files_scanned += 1;

        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        scan_file(&rel, &content, &tokens, &mut matches);
    }

    matches.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });
    matches.truncate(limit);

    Ok(DocsResult {
        query: query.join(" "),
        matches,
        files_scanned,
    })
}

fn scan_file(rel: &str, content: &str, tokens: &[String], out: &mut Vec<DocsMatch>) {
    let is_doc_file = is_doc_file(rel);
    let mut in_block = false;
    let mut in_py_docstring = false;

    for (idx, raw) in content.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }

        let candidate = if is_doc_file {
            Some(("doc", trim_doc_line(trimmed)))
        } else if trimmed.starts_with("///") || trimmed.starts_with("//!") {
            Some(("doc-comment", trim_marker(trimmed, &["///", "//!"])))
        } else if trimmed.starts_with("/**") || trimmed.starts_with("/*!") {
            in_block = !trimmed.contains("*/");
            Some(("doc-comment", trim_block_comment(trimmed)))
        } else if in_block {
            if trimmed.contains("*/") {
                in_block = false;
            }
            Some(("doc-comment", trim_block_comment(trimmed)))
        } else if trimmed.starts_with("//") {
            Some(("comment", trim_marker(trimmed, &["//"])))
        } else if trimmed.starts_with('#') {
            Some(("comment", trim_marker(trimmed, &["#"])))
        } else if trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''") {
            in_py_docstring = !ends_docstring_same_line(trimmed);
            Some(("docstring", trim_py_docstring(trimmed)))
        } else if in_py_docstring {
            if trimmed.ends_with("\"\"\"") || trimmed.ends_with("'''") {
                in_py_docstring = false;
            }
            Some(("docstring", trim_py_docstring(trimmed)))
        } else {
            None
        };

        let Some((kind, text)) = candidate else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() || text == "*" || text == "/" {
            continue;
        }

        let score = score_match(rel, text, tokens);
        if tokens.is_empty() || score > 0 {
            out.push(DocsMatch {
                file: rel.to_string(),
                line: line_no,
                kind: kind.to_string(),
                text: text.to_string(),
                score,
            });
        }
    }
}

fn score_match(file: &str, text: &str, tokens: &[String]) -> usize {
    if tokens.is_empty() {
        return if is_instruction_file(file) { 2 } else { 1 };
    }
    let haystack = format!("{} {}", file.to_lowercase(), text.to_lowercase());
    tokens
        .iter()
        .filter(|t| haystack.contains(t.as_str()))
        .count()
}

fn is_relevant_file(path: &str) -> bool {
    is_doc_file(path)
        || matches!(
            Path::new(path).extension().and_then(|e| e.to_str()),
            Some(
                "rs" | "go"
                    | "py"
                    | "ts"
                    | "tsx"
                    | "js"
                    | "jsx"
                    | "java"
                    | "c"
                    | "h"
                    | "cpp"
                    | "hpp"
            )
        )
}

fn is_doc_file(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("md" | "mdx" | "rst" | "txt")
    ) || is_instruction_file(name)
}

fn is_instruction_file(name: &str) -> bool {
    matches!(
        name,
        "README.md" | "AGENTS.md" | "CLAUDE.md" | "CONTRIBUTING.md"
    )
}

fn trim_doc_line(s: &str) -> &str {
    s.trim_start_matches('#').trim_start_matches('-').trim()
}

fn trim_marker<'a>(s: &'a str, markers: &[&str]) -> &'a str {
    let mut out = s;
    for marker in markers {
        out = out.strip_prefix(marker).unwrap_or(out);
    }
    out.trim()
}

fn trim_block_comment(s: &str) -> &str {
    s.trim_start_matches("/**")
        .trim_start_matches("/*!")
        .trim_start_matches("/*")
        .trim_start_matches('*')
        .trim_end_matches("*/")
        .trim()
}

fn trim_py_docstring(s: &str) -> &str {
    s.trim_matches('"').trim_matches('\'').trim()
}

fn ends_docstring_same_line(s: &str) -> bool {
    (s.starts_with("\"\"\"") && s[3..].contains("\"\"\""))
        || (s.starts_with("'''") && s[3..].contains("'''"))
}
