use crate::compress::{self, Lang};
use crate::symbol::Symbol;

// ── Doc comment extraction (language-aware) ─────────────────────────

pub(crate) fn extract_doc_comment(content: &str, sym: &Symbol) -> Option<String> {
    let lang = compress::detect_lang(&sym.file);

    // Python: docstrings live inside the function/class body
    if lang == Some(Lang::Python)
        && let Some(docstring) = extract_python_docstring(content, sym)
    {
        return Some(docstring);
    }

    // Rust/C/JS/etc: comments live above the definition
    extract_comment_above(content, sym.line)
}

fn extract_python_docstring(content: &str, sym: &Symbol) -> Option<String> {
    let tree = compress::parse_source(content, Lang::Python)?;
    let root = tree.root_node();
    let def_node = super::definition::find_definition_node(root, content, sym, Lang::Python)?;

    let body = def_node.child_by_field_name("body")?;
    let mut cursor = body.walk();
    if !cursor.goto_first_child() {
        return None;
    }

    let first_stmt = cursor.node();
    if first_stmt.kind() != "expression_statement" {
        return None;
    }

    let mut inner = first_stmt.walk();
    if !inner.goto_first_child() {
        return None;
    }

    let expr = inner.node();
    if expr.kind() != "string" {
        return None;
    }

    let raw = compress::node_text(content, expr);
    Some(clean_docstring(raw))
}

fn clean_docstring(raw: &str) -> String {
    let s = raw.trim();
    let inner = if (s.starts_with("\"\"\"") && s.ends_with("\"\"\"")
        || s.starts_with("'''") && s.ends_with("'''"))
        && s.len() >= 6
    {
        &s[3..s.len() - 3]
    } else {
        s
    };

    // Dedent: find the minimum leading whitespace across non-empty lines
    let lines: Vec<&str> = inner.lines().collect();
    if lines.len() <= 1 {
        return inner.trim().to_string();
    }

    let min_indent = lines[1..]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

    let mut result: Vec<&str> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            result.push(line.trim());
        } else if line.len() >= min_indent {
            result.push(&line[min_indent..]);
        } else {
            result.push(line.trim());
        }
    }

    // Trim trailing empty lines
    while result.last().is_some_and(|l| l.trim().is_empty()) {
        result.pop();
    }
    // Trim leading empty lines
    while result.first().is_some_and(|l| l.trim().is_empty()) {
        result.remove(0);
    }

    result.join("\n")
}

fn extract_comment_above(content: &str, def_line: usize) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if def_line == 0 || def_line > lines.len() {
        return None;
    }

    let def_idx = def_line - 1;
    let mut comment_lines: Vec<&str> = Vec::new();

    let mut i = def_idx.wrapping_sub(1);
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with("///") || trimmed.starts_with("//!") {
            // Rust doc comments
            comment_lines.push(trimmed);
        } else if trimmed.starts_with("//") {
            // Regular line comments (Go, C, C++, Java, JS/TS)
            comment_lines.push(trimmed);
        } else if trimmed.starts_with("/**") || trimmed.ends_with("*/") || trimmed.starts_with('*')
        {
            // Block doc comments (Java, JS/TS, C)
            comment_lines.push(trimmed);
        } else if trimmed.starts_with('#') || trimmed.starts_with('@') {
            // Rust attributes / Python decorators — skip but keep looking
        } else {
            break;
        }
        i = i.wrapping_sub(1);
    }

    if comment_lines.is_empty() {
        return None;
    }

    comment_lines.reverse();
    Some(comment_lines.join("\n"))
}
