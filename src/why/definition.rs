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
    if sym.line == 0 {
        return None;
    }
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
pub(super) fn find_definition_span(root: &std::path::Path, sym: &Symbol) -> Option<(usize, usize)> {
    let abs = root.join(&sym.file);
    let content = std::fs::read_to_string(&abs).ok()?;
    let lang = compress::detect_lang(&sym.file)?;
    let tree = compress::parse_source(&content, lang)?;
    let node = find_definition_node(tree.root_node(), &content, sym, lang)?;
    Some((node.start_position().row + 1, node.end_position().row + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolKind;

    fn sym(name: &str, file: &str, line: usize, kind: SymbolKind) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind,
            file: file.to_string(),
            line,
            signature: String::new(),
            parent: None,
            keywords: Vec::new(),
        }
    }

    // ── extract_full_definition ─────────────────────────────────

    #[test]
    fn rust_function_definition() {
        let content = "pub fn greet(name: &str) -> String {\n    format!(\"Hi {}\", name)\n}\n";
        let s = sym("greet", "test.rs", 1, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("pub fn greet"));
        assert!(result.contains("format!"));
    }

    #[test]
    fn rust_struct_definition() {
        let content = "pub struct Config {\n    pub name: String,\n}\n";
        let s = sym("Config", "test.rs", 1, SymbolKind::Struct);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("pub struct Config"));
        assert!(result.contains("name: String"));
    }

    #[test]
    fn python_function_definition() {
        let content = "def greet(name):\n    return f'Hi {name}'\n";
        let s = sym("greet", "test.py", 1, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("def greet"));
    }

    #[test]
    fn python_class_definition() {
        let content = "class MyClass:\n    def __init__(self):\n        self.x = 1\n";
        let s = sym("MyClass", "test.py", 1, SymbolKind::Class);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("class MyClass"));
    }

    #[test]
    fn js_arrow_function_const() {
        let content = "const greet = (name) => {\n    return `Hi ${name}`;\n};\n";
        let s = sym("greet", "test.js", 1, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("greet"));
    }

    #[test]
    fn c_function_definition() {
        let content = "int add(int a, int b) {\n    return a + b;\n}\n";
        let s = sym("add", "test.c", 1, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("int add"));
    }

    #[test]
    fn c_struct_specifier() {
        let content = "struct Point {\n    int x;\n    int y;\n};\n";
        let s = sym("Point", "test.c", 1, SymbolKind::Struct);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("struct Point"));
    }

    #[test]
    fn cpp_qualified_function() {
        let content = "void Foo::bar() {\n    return;\n}\n";
        let s = sym("bar", "test.cpp", 1, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("Foo::bar"));
    }

    #[test]
    fn python_module_assignment() {
        let content = "MAX_SIZE = 100\n";
        let s = sym("MAX_SIZE", "test.py", 1, SymbolKind::Const);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("MAX_SIZE = 100"));
    }

    #[test]
    fn unsupported_language_fallback() {
        let content = "some content here\nmore content\n";
        let s = sym("something", "test.txt", 1, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        // Falls back to line-based extraction
        assert!(result.contains("some content here"));
    }

    #[test]
    fn invalid_line_number() {
        let content = "fn foo() {}\n";
        let s = sym("foo", "test.rs", 999, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        // Line out of range - falls back, returns empty from extract_definition_by_lines
        assert!(result.is_empty());
    }

    #[test]
    fn line_zero_returns_empty() {
        let content = "fn foo() {}\n";
        let s = sym("foo", "test.rs", 0, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        assert!(result.is_empty());
    }

    // ── extract_definition_by_lines ─────────────────────────────

    #[test]
    fn brace_based_multiline() {
        let content = "fn foo() {\n    let x = 1;\n    x + 1\n}\n";
        let s = sym("foo", "test.unknown_ext", 1, SymbolKind::Function);
        // Unknown ext → falls through to line-based
        let result = extract_full_definition(content, &s);
        assert!(result.contains("fn foo()"));
        assert!(result.contains("}"));
    }

    #[test]
    fn semicolon_terminated_line() {
        let content = "int add(int a, int b);\n";
        let s = sym("add", "test.unknown_ext", 1, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("int add"));
    }

    #[test]
    fn python_indentation_based() {
        let content = "def foo():\n    x = 1\n    return x\n\ndef bar():\n    pass\n";
        let s = sym("foo", "test.py_fallback", 1, SymbolKind::Function);
        // .py_fallback won't be detected as Python, so uses brace-based fallback
        let result = extract_full_definition(content, &s);
        assert!(result.contains("def foo()"));
    }

    // ── find_definition_span ────────────────────────────────────

    #[test]
    fn definition_span_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = "fn hello() {\n    println!(\"hi\");\n}\n";
        std::fs::write(dir.path().join("test.rs"), content).unwrap();
        let s = sym("hello", "test.rs", 1, SymbolKind::Function);
        let span = find_definition_span(dir.path(), &s);
        assert!(span.is_some());
        let (start, end) = span.unwrap();
        assert_eq!(start, 1);
        assert_eq!(end, 3);
    }

    #[test]
    fn definition_span_file_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let s = sym("foo", "nonexistent.rs", 1, SymbolKind::Function);
        assert!(find_definition_span(dir.path(), &s).is_none());
    }

    #[test]
    fn definition_span_unsupported_lang() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.txt"), "some text").unwrap();
        let s = sym("foo", "test.txt", 1, SymbolKind::Function);
        assert!(find_definition_span(dir.path(), &s).is_none());
    }

    // ── find_c_name_in_declarator ───────────────────────────────

    #[test]
    fn c_function_declarator_name() {
        let content = "int (*callback)(int, int);\n";
        let s = sym("callback", "test.c", 1, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        // Should at least extract the line
        assert!(!result.is_empty());
    }

    #[test]
    fn ts_function_declaration() {
        let content = "function greet(name: string): string {\n    return `Hi ${name}`;\n}\n";
        let s = sym("greet", "test.ts", 1, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("function greet"));
    }

    #[test]
    fn java_class_definition() {
        let content = "public class MyService {\n    public void run() {\n        System.out.println(\"running\");\n    }\n}\n";
        let s = sym("MyService", "test.java", 1, SymbolKind::Class);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("class MyService"));
    }

    #[test]
    fn go_function_definition() {
        let content = "package main\n\nfunc Add(a, b int) int {\n\treturn a + b\n}\n";
        let s = sym("Add", "test.go", 3, SymbolKind::Function);
        let result = extract_full_definition(content, &s);
        assert!(result.contains("func Add"));
    }
}
