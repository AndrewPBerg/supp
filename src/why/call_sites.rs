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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolKind;
    use std::fs;
    use tempfile::TempDir;

    fn sym(name: &str, file: &str, line: usize) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            file: file.to_string(),
            line,
            signature: String::new(),
            parent: None,
            keywords: Vec::new(),
        }
    }

    // ── contains_identifier ─────────────────────────────────────

    #[test]
    fn contains_exact_match() {
        assert!(contains_identifier("foo(bar)", "foo"));
        assert!(contains_identifier("foo(bar)", "bar"));
    }

    #[test]
    fn rejects_substring() {
        assert!(!contains_identifier("foobar", "foo"));
        assert!(!contains_identifier("barfoo", "foo"));
    }

    #[test]
    fn match_at_start() {
        assert!(contains_identifier("foo = 1", "foo"));
    }

    #[test]
    fn match_at_end() {
        assert!(contains_identifier("x = foo", "foo"));
    }

    #[test]
    fn match_with_dot() {
        assert!(contains_identifier("x.foo()", "foo"));
    }

    #[test]
    fn no_match_underscore_prefix() {
        assert!(!contains_identifier("_foo = 1", "foo"));
    }

    #[test]
    fn no_match_underscore_suffix() {
        assert!(!contains_identifier("foo_ = 1", "foo"));
    }

    #[test]
    fn empty_line() {
        assert!(!contains_identifier("", "foo"));
    }

    #[test]
    fn multiple_occurrences() {
        assert!(contains_identifier("foo + foo", "foo"));
    }

    #[test]
    fn identifier_with_numbers() {
        assert!(contains_identifier("call foo2()", "foo2"));
        assert!(!contains_identifier("call foo2()", "foo"));
    }

    // ── find_enclosing_function ─────────────────────────────────

    #[test]
    fn rust_enclosing_function() {
        let content = "fn outer() {\n    let x = inner();\n}\nfn inner() {}\n";
        let tree = compress::parse_source(content, compress::Lang::Rust).unwrap();
        let result = find_enclosing_function(&tree, content, 1); // line 1 (0-based) is inside outer
        assert_eq!(result, Some("outer".to_string()));
    }

    #[test]
    fn no_enclosing_function() {
        let content = "let x = 1;\nfn foo() {}\n";
        let tree = compress::parse_source(content, compress::Lang::Rust).unwrap();
        let result = find_enclosing_function(&tree, content, 0); // top-level
        assert_eq!(result, None);
    }

    #[test]
    fn python_enclosing_function() {
        let content = "def outer():\n    x = helper()\n    return x\n";
        let tree = compress::parse_source(content, compress::Lang::Python).unwrap();
        let result = find_enclosing_function(&tree, content, 1);
        assert_eq!(result, Some("outer".to_string()));
    }

    #[test]
    fn js_enclosing_function() {
        let content = "function doWork() {\n    let result = compute();\n    return result;\n}\n";
        let tree = compress::parse_source(content, compress::Lang::JavaScript).unwrap();
        let result = find_enclosing_function(&tree, content, 1);
        assert_eq!(result, Some("doWork".to_string()));
    }

    #[test]
    fn java_method_enclosing() {
        let content = "class Foo {\n    void doStuff() {\n        helper();\n    }\n}\n";
        let tree = compress::parse_source(content, compress::Lang::Java).unwrap();
        let result = find_enclosing_function(&tree, content, 2);
        assert_eq!(result, Some("doStuff".to_string()));
    }

    // ── find_call_sites ─────────────────────────────────────────

    #[test]
    fn finds_cross_file_call() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("main.rs"),
            "fn main() {\n    helper();\n}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("lib.rs"),
            "pub fn helper() {\n    println!(\"help\");\n}\n",
        )
        .unwrap();
        let s = sym("helper", "lib.rs", 1);
        let sites = find_call_sites(dir.path(), &s);
        assert!(!sites.is_empty());
        assert!(sites.iter().any(|s| s.file == "main.rs"));
    }

    #[test]
    fn skips_definition_span() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("lib.rs"),
            "pub fn helper() {\n    // helper is defined here\n}\nfn caller() {\n    helper();\n}\n",
        )
        .unwrap();
        let s = sym("helper", "lib.rs", 1);
        let sites = find_call_sites(dir.path(), &s);
        // Should find the call in caller() but not the definition itself
        assert!(sites.iter().all(|s| s.line > 3));
    }

    #[test]
    fn short_name_returns_empty() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("test.rs"), "let x = 1;\n").unwrap();
        let s = sym("x", "test.rs", 1);
        let sites = find_call_sites(dir.path(), &s);
        assert!(sites.is_empty());
    }

    #[test]
    fn truncates_at_30() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("pub fn target() {}\n");
        for i in 0..40 {
            content.push_str(&format!("fn f{}() {{ target(); }}\n", i));
        }
        fs::write(dir.path().join("test.rs"), &content).unwrap();
        let s = sym("target", "test.rs", 1);
        let sites = find_call_sites(dir.path(), &s);
        assert!(sites.len() <= 30);
    }

    #[test]
    fn includes_caller_name() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("test.rs"),
            "pub fn target() {}\nfn my_caller() {\n    target();\n}\n",
        )
        .unwrap();
        let s = sym("target", "test.rs", 1);
        let sites = find_call_sites(dir.path(), &s);
        assert!(
            sites
                .iter()
                .any(|s| s.caller.as_deref() == Some("my_caller"))
        );
    }
}
