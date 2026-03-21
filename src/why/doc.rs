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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::{Symbol, SymbolKind};

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

    // ── extract_doc_comment ─────────────────────────────────────

    #[test]
    fn rust_doc_comment() {
        let content = "/// Does something.\n/// Returns a value.\npub fn foo() {}\n";
        let s = sym("foo", "test.rs", 3, SymbolKind::Function);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_some());
        let doc = result.unwrap();
        assert!(doc.contains("Does something."));
        assert!(doc.contains("Returns a value."));
    }

    #[test]
    fn rust_no_doc_comment() {
        let content = "pub fn foo() {}\n";
        let s = sym("foo", "test.rs", 1, SymbolKind::Function);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_none());
    }

    #[test]
    fn rust_attribute_above_doc() {
        let content = "/// Important function.\n#[inline]\npub fn foo() {}\n";
        let s = sym("foo", "test.rs", 3, SymbolKind::Function);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Important function."));
    }

    #[test]
    fn python_docstring_triple_double() {
        let content =
            "def greet(name):\n    \"\"\"Greet someone by name.\"\"\"\n    return f'Hi {name}'\n";
        let s = sym("greet", "test.py", 1, SymbolKind::Function);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Greet someone by name."));
    }

    #[test]
    fn python_docstring_triple_single() {
        let content = "def greet(name):\n    '''Greet someone.'''\n    return f'Hi {name}'\n";
        let s = sym("greet", "test.py", 1, SymbolKind::Function);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Greet someone."));
    }

    #[test]
    fn python_multiline_docstring() {
        let content = "def greet(name):\n    \"\"\"Greet someone.\n\n    Args:\n        name: The name.\n    \"\"\"\n    return f'Hi {name}'\n";
        let s = sym("greet", "test.py", 1, SymbolKind::Function);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_some());
        let doc = result.unwrap();
        assert!(doc.contains("Greet someone."));
        assert!(doc.contains("Args:"));
    }

    #[test]
    fn python_class_docstring() {
        let content = "class Foo:\n    \"\"\"A Foo class.\"\"\"\n    pass\n";
        let s = sym("Foo", "test.py", 1, SymbolKind::Class);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_some());
        assert!(result.unwrap().contains("A Foo class."));
    }

    #[test]
    fn python_no_docstring() {
        let content = "def foo():\n    pass\n";
        let s = sym("foo", "test.py", 1, SymbolKind::Function);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_none());
    }

    #[test]
    fn python_non_string_first_statement() {
        let content = "def foo():\n    x = 1\n    return x\n";
        let s = sym("foo", "test.py", 1, SymbolKind::Function);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_none());
    }

    #[test]
    fn js_block_comment() {
        let content = "/**\n * Does something.\n * @param x value\n */\nfunction foo(x) {}\n";
        let s = sym("foo", "test.js", 5, SymbolKind::Function);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Does something."));
    }

    #[test]
    fn js_line_comment() {
        let content = "// Helper function\nfunction foo() {}\n";
        let s = sym("foo", "test.js", 2, SymbolKind::Function);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Helper function"));
    }

    #[test]
    fn go_comment() {
        let content = "// Add adds two numbers.\nfunc Add(a, b int) int {\n\treturn a + b\n}\n";
        let s = sym("Add", "test.go", 2, SymbolKind::Function);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Add adds two numbers."));
    }

    #[test]
    fn java_javadoc() {
        let content = "/**\n * Runs the service.\n */\npublic void run() {}\n";
        let s = sym("run", "test.java", 4, SymbolKind::Method);
        let result = extract_doc_comment(content, &s);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Runs the service."));
    }

    // ── clean_docstring ─────────────────────────────────────────

    #[test]
    fn clean_single_line() {
        let result = clean_docstring("\"\"\"Hello world.\"\"\"");
        assert_eq!(result, "Hello world.");
    }

    #[test]
    fn clean_single_quotes() {
        let result = clean_docstring("'''Hello world.'''");
        assert_eq!(result, "Hello world.");
    }

    #[test]
    fn clean_multiline_dedent() {
        let result =
            clean_docstring("\"\"\"First line.\n    Second line.\n    Third line.\n    \"\"\"");
        assert!(result.contains("First line."));
        assert!(result.contains("Second line."));
    }

    #[test]
    fn clean_no_quotes() {
        let result = clean_docstring("Just a string");
        assert_eq!(result, "Just a string");
    }

    #[test]
    fn clean_empty_inner() {
        let result = clean_docstring("\"\"\"\"\"\"");
        assert!(result.is_empty());
    }

    // ── extract_comment_above ───────────────────────────────────

    #[test]
    fn comment_above_first_line() {
        let content = "fn foo() {}\n";
        // Line 1, no comment above
        let result = extract_comment_above(content, 1);
        assert!(result.is_none());
    }

    #[test]
    fn comment_above_line_zero() {
        let content = "fn foo() {}\n";
        let result = extract_comment_above(content, 0);
        assert!(result.is_none());
    }

    #[test]
    fn comment_above_beyond_file() {
        let content = "fn foo() {}\n";
        let result = extract_comment_above(content, 100);
        assert!(result.is_none());
    }

    #[test]
    fn decorator_skipped_to_find_comment() {
        let content = "// Important\n@decorator\ndef foo():\n    pass\n";
        let result = extract_comment_above(content, 3);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Important"));
    }

    #[test]
    fn rust_inner_doc_comment() {
        let content = "//! Module-level doc.\nfn foo() {}\n";
        let result = extract_comment_above(content, 2);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Module-level doc."));
    }
}
