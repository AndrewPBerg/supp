use crate::compress::{self, Lang};
use crate::symbol::Symbol;

// ── Full definition extraction ──────────────────────────────────────

pub(crate) fn extract_full_definition(content: &str, sym: &Symbol) -> String {
    let lang = match compress::detect_lang(&sym.file) {
        Some(l) => l,
        None => return extract_definition_by_lines(content, sym),
    };

    let tree = match compress::parse_source(content, lang) {
        Some(t) => t,
        None => return extract_definition_by_lines(content, sym),
    };

    let root = tree.root_node();
    if let Some(node) = find_definition_node(root, content, sym, lang) {
        return compress::node_text(content, node).to_string();
    }

    extract_definition_by_lines(content, sym)
}

pub(super) fn find_definition_node<'a>(
    node: tree_sitter::Node<'a>,
    content: &str,
    sym: &Symbol,
    _lang: Lang,
) -> Option<tree_sitter::Node<'a>> {
    let line = sym.line - 1; // tree-sitter uses 0-based

    if node.start_position().row == line {
        // Standard named definitions (fn, class, struct, etc.)
        if let Some(name_node) = node.child_by_field_name("name")
            && compress::node_text(content, name_node) == sym.name
        {
            return Some(node);
        }

        // C/C++: function_definition → declarator → function_declarator → identifier
        if node.kind() == "function_definition"
            && let Some(declarator) = node.child_by_field_name("declarator")
            && find_c_name_in_declarator(declarator, content) == Some(&sym.name)
        {
            return Some(node);
        }

        // C/C++: struct_specifier, enum_specifier, class_specifier with name field
        if matches!(
            node.kind(),
            "struct_specifier" | "enum_specifier" | "class_specifier"
        ) && let Some(name_node) = node.child_by_field_name("name")
            && compress::node_text(content, name_node) == sym.name
        {
            return Some(node);
        }

        // JS/TS: const MyComponent = (...) => { ... } (lexical_declaration wrapping arrow fn)
        if matches!(node.kind(), "lexical_declaration" | "variable_declaration") {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "variable_declarator"
                        && let Some(name_node) = child.child_by_field_name("name")
                        && compress::node_text(content, name_node) == sym.name
                    {
                        return Some(node);
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }

        // Python module-level assignments: expression_statement → assignment → left
        if node.kind() == "expression_statement" {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                let child = cursor.node();
                if child.kind() == "assignment"
                    && let Some(left) = child.child_by_field_name("left")
                    && left.kind() == "identifier"
                    && compress::node_text(content, left) == sym.name
                {
                    return Some(node);
                }
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if let Some(found) = find_definition_node(cursor.node(), content, sym, _lang) {
                return Some(found);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    None
}

/// Recursively find the function name inside a C/C++ declarator chain.
fn find_c_name_in_declarator<'a>(node: tree_sitter::Node<'a>, content: &'a str) -> Option<&'a str> {
    if node.kind() == "identifier" {
        return Some(compress::node_text(content, node));
    }
    // qualified_identifier: Foo::bar → the "name" field has the actual name
    if node.kind() == "qualified_identifier"
        && let Some(name) = node.child_by_field_name("name")
        && name.kind() == "identifier"
    {
        return Some(compress::node_text(content, name));
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if let Some(name) = find_c_name_in_declarator(cursor.node(), content) {
                return Some(name);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

fn extract_definition_by_lines(content: &str, sym: &Symbol) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let start_line = sym.line;
    if start_line == 0 || start_line > lines.len() {
        return String::new();
    }

    let start = start_line - 1;
    let lang = compress::detect_lang(&sym.file);
    let uses_indentation = matches!(lang, Some(Lang::Python));

    if uses_indentation {
        // Python: use indentation to find end of block
        let base_indent = lines[start].len() - lines[start].trim_start().len();
        let mut end = start;
        for (i, line) in lines[start + 1..].iter().enumerate() {
            if line.trim().is_empty() {
                end = start + 1 + i;
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            if indent <= base_indent {
                break;
            }
            end = start + 1 + i;
        }
        lines[start..=end.min(lines.len() - 1)].join("\n")
    } else {
        // Brace-based languages
        let mut brace_depth: i32 = 0;
        let mut found_open = false;
        let mut end = start;

        for (i, line) in lines[start..].iter().enumerate() {
            for ch in line.chars() {
                match ch {
                    '{' => {
                        brace_depth += 1;
                        found_open = true;
                    }
                    '}' => brace_depth -= 1,
                    _ => {}
                }
            }
            end = start + i;
            if found_open && brace_depth <= 0 {
                break;
            }
            if !found_open && line.trim_end().ends_with(';') {
                break;
            }
        }

        lines[start..=end.min(lines.len() - 1)].join("\n")
    }
}

/// Find the line span (start, end) of a symbol's definition for exclusion.
pub(super) fn find_definition_span(
    root: &std::path::Path,
    sym: &Symbol,
) -> Option<(usize, usize)> {
    let abs = root.join(&sym.file);
    let content = std::fs::read_to_string(&abs).ok()?;
    let lang = compress::detect_lang(&sym.file)?;
    let tree = compress::parse_source(&content, lang)?;
    let node = find_definition_node(tree.root_node(), &content, sym, lang)?;
    Some((node.start_position().row + 1, node.end_position().row + 1))
}
