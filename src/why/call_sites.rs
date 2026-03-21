use std::path::Path;

use crate::compress;
use crate::symbol::Symbol;

use super::CallSite;

// ── Call site discovery ─────────────────────────────────────────────

pub(crate) fn find_call_sites(root: &Path, sym: &Symbol) -> Vec<CallSite> {
    let mut sites = Vec::new();
    let name = &sym.name;

    if name.len() <= 2 {
        return sites;
    }

    // For same-file filtering: find the definition's line span so we can skip it
    let def_span = super::definition::find_definition_span(root, sym);

    let walker = ignore::WalkBuilder::new(root)
        .sort_by_file_name(|a, b| a.cmp(b))
        .build();

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let is_same_file = rel == sym.file;

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lang = compress::detect_lang(&rel);

        for (line_idx, line) in content.lines().enumerate() {
            let line_num = line_idx + 1; // 1-based

            // Skip lines inside the definition itself (same file only)
            if is_same_file
                && let Some((def_start, def_end)) = def_span
                && line_num >= def_start
                && line_num <= def_end
            {
                continue;
            }

            if contains_identifier(line, name) {
                let caller = lang.and_then(|l| {
                    let tree = compress::parse_source(&content, l)?;
                    find_enclosing_function(&tree, &content, line_idx)
                });

                sites.push(CallSite {
                    file: rel.clone(),
                    line: line_num,
                    context: line.trim().to_string(),
                    caller,
                });
            }
        }
    }

    sites.truncate(30);
    sites
}

pub(crate) fn contains_identifier(line: &str, name: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = line[start..].find(name) {
        let abs_pos = start + pos;
        let before_ok = abs_pos == 0 || !is_ident_char(line.as_bytes()[abs_pos - 1]);
        let after_pos = abs_pos + name.len();
        let after_ok = after_pos >= line.len() || !is_ident_char(line.as_bytes()[after_pos]);

        if before_ok && after_ok {
            return true;
        }
        start = abs_pos + 1;
    }
    false
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub(crate) fn find_enclosing_function(
    tree: &tree_sitter::Tree,
    content: &str,
    line: usize,
) -> Option<String> {
    find_enclosing_fn_recursive(tree.root_node(), content, line)
}

fn find_enclosing_fn_recursive(
    node: tree_sitter::Node,
    content: &str,
    line: usize,
) -> Option<String> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }

    loop {
        let child = cursor.node();
        let start = child.start_position().row;
        let end = child.end_position().row;

        if line >= start && line <= end {
            let is_fn = matches!(
                child.kind(),
                "function_item"
                    | "function_definition"
                    | "function_declaration"
                    | "method_declaration"
                    | "method_definition"
                    | "constructor_declaration"
            );

            if is_fn && let Some(name_node) = child.child_by_field_name("name") {
                return Some(compress::node_text(content, name_node).to_string());
            }

            if let Some(found) = find_enclosing_fn_recursive(child, content, line) {
                return Some(found);
            }
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }

    None
}
