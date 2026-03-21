use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::compress::{self, Lang};
use crate::symbol::{self, Symbol};

use super::Dependency;

// ── Dependency discovery ────────────────────────────────────────────

pub(super) fn find_dependencies(
    root: &Path,
    sym: &Symbol,
    content: &str,
    all_symbols: &[Symbol],
    imports: &HashMap<String, String>,
) -> Vec<Dependency> {
    let lang = match compress::detect_lang(&sym.file) {
        Some(l) => l,
        None => return Vec::new(),
    };

    let tree = match compress::parse_source(content, lang) {
        Some(t) => t,
        None => return Vec::new(),
    };

    // Get identifiers used within this symbol's body
    let root_node = tree.root_node();
    let def_node = match super::definition::find_definition_node(root_node, content, sym, lang) {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut idents: Vec<String> = Vec::new();
    collect_body_identifiers(def_node, content, lang, &mut idents);
    idents.sort();
    idents.dedup();
    idents.retain(|id| id != &sym.name);

    let mut deps = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for ident in &idents {
        if seen.contains(ident) {
            continue;
        }

        // Try to find in project symbol index (prefer cross-file, then same-file)
        let project_match = all_symbols
            .iter()
            .find(|s| s.name == *ident && s.file != sym.file)
            .or_else(|| all_symbols.iter().find(|s| s.name == *ident));

        let import_module = imports.get(ident);

        if let Some(s) = project_match {
            seen.insert(ident.clone());
            deps.push(Dependency {
                name: s.name.clone(),
                kind: Some(s.kind),
                location: Some((s.file.clone(), s.line)),
                import_from: import_module.cloned(),
            });
        } else if let Some(module) = import_module {
            // External dependency — not in project index but imported
            seen.insert(ident.clone());

            // Try resolving relative imports to project files
            let resolved = super::imports::resolve_relative_import(module, &sym.file, root);

            if let Some(ref file) = resolved {
                // Found in project via relative import — try to get the line
                let line = all_symbols
                    .iter()
                    .find(|s| s.name == *ident && &s.file == file)
                    .map(|s| s.line);
                deps.push(Dependency {
                    name: ident.clone(),
                    kind: line.and_then(|_| {
                        all_symbols
                            .iter()
                            .find(|s| s.name == *ident && &s.file == file)
                            .map(|s| s.kind)
                    }),
                    location: line.map(|l| (file.clone(), l)),
                    import_from: Some(module.clone()),
                });
            } else {
                deps.push(Dependency {
                    name: ident.clone(),
                    kind: None,
                    location: None,
                    import_from: Some(module.clone()),
                });
            }
        }
    }

    deps
}

fn collect_body_identifiers(
    node: tree_sitter::Node,
    content: &str,
    lang: Lang,
    idents: &mut Vec<String>,
) {
    // Collect from the entire definition node (body + signature).
    let name_text = node
        .child_by_field_name("name")
        .map(|n| compress::node_text(content, n).to_string());

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            // Skip the name node itself (avoids self-reference)
            let is_name = name_text.as_ref().is_some_and(|n| {
                child.kind() == "identifier" && compress::node_text(content, child) == n
            });
            if !is_name {
                collect_all_identifiers(child, content, lang, idents);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn collect_all_identifiers(
    node: tree_sitter::Node,
    content: &str,
    lang: Lang,
    idents: &mut Vec<String>,
) {
    if matches!(node.kind(), "identifier" | "type_identifier") {
        let text = compress::node_text(content, node);
        if text.len() > 1 && !symbol::is_keyword(text, lang) {
            idents.push(text.to_string());
        }
    }

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_all_identifiers(cursor.node(), content, lang, idents);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolKind;
    use std::collections::HashMap;

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

    fn make_root() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }

    #[test]
    fn finds_project_dependency() {
        let dir = make_root();
        let content = "fn caller() {\n    let x = helper();\n}\n";
        std::fs::write(dir.path().join("main.rs"), content).unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn helper() -> i32 { 42 }\n").unwrap();

        let s = sym("caller", "main.rs", 1, SymbolKind::Function);
        let all_symbols = vec![
            sym("caller", "main.rs", 1, SymbolKind::Function),
            sym("helper", "lib.rs", 1, SymbolKind::Function),
        ];
        let imports = HashMap::new();
        let deps = find_dependencies(dir.path(), &s, content, &all_symbols, &imports);
        assert!(deps.iter().any(|d| d.name == "helper"));
    }

    #[test]
    fn excludes_self_reference() {
        let dir = make_root();
        let content = "fn caller() {\n    caller();\n}\n";
        std::fs::write(dir.path().join("test.rs"), content).unwrap();

        let s = sym("caller", "test.rs", 1, SymbolKind::Function);
        let all_symbols = vec![sym("caller", "test.rs", 1, SymbolKind::Function)];
        let imports = HashMap::new();
        let deps = find_dependencies(dir.path(), &s, content, &all_symbols, &imports);
        assert!(deps.iter().all(|d| d.name != "caller"));
    }

    #[test]
    fn finds_external_import_dependency() {
        let dir = make_root();
        let content =
            "from requests import get\n\ndef fetch():\n    return get('http://example.com')\n";
        std::fs::write(dir.path().join("main.py"), content).unwrap();

        let s = sym("fetch", "main.py", 3, SymbolKind::Function);
        let all_symbols = vec![sym("fetch", "main.py", 3, SymbolKind::Function)];
        let mut imports = HashMap::new();
        imports.insert("get".to_string(), "requests".to_string());
        let deps = find_dependencies(dir.path(), &s, content, &all_symbols, &imports);
        assert!(
            deps.iter()
                .any(|d| d.name == "get" && d.import_from.as_deref() == Some("requests"))
        );
    }

    #[test]
    fn unsupported_lang_returns_empty() {
        let dir = make_root();
        let content = "some text\n";
        std::fs::write(dir.path().join("test.txt"), content).unwrap();

        let s = sym("something", "test.txt", 1, SymbolKind::Function);
        let deps = find_dependencies(dir.path(), &s, content, &[], &HashMap::new());
        assert!(deps.is_empty());
    }

    #[test]
    fn definition_not_found_returns_empty() {
        let dir = make_root();
        let content = "fn other() {}\n";
        std::fs::write(dir.path().join("test.rs"), content).unwrap();

        let s = sym("nonexistent", "test.rs", 99, SymbolKind::Function);
        let deps = find_dependencies(dir.path(), &s, content, &[], &HashMap::new());
        assert!(deps.is_empty());
    }

    #[test]
    fn filters_keywords() {
        let dir = make_root();
        let content = "fn caller() {\n    if true { return; }\n}\n";
        std::fs::write(dir.path().join("test.rs"), content).unwrap();

        let s = sym("caller", "test.rs", 1, SymbolKind::Function);
        let all_symbols = vec![sym("caller", "test.rs", 1, SymbolKind::Function)];
        let deps = find_dependencies(dir.path(), &s, content, &all_symbols, &HashMap::new());
        // "if", "true", "return" are keywords, should not appear
        assert!(
            deps.iter()
                .all(|d| !matches!(d.name.as_str(), "if" | "true" | "return"))
        );
    }

    #[test]
    fn deduplicates_identifiers() {
        let dir = make_root();
        let content = "fn caller() {\n    helper();\n    helper();\n}\n";
        std::fs::write(dir.path().join("test.rs"), content).unwrap();

        let s = sym("caller", "test.rs", 1, SymbolKind::Function);
        let all_symbols = vec![
            sym("caller", "test.rs", 1, SymbolKind::Function),
            sym("helper", "lib.rs", 1, SymbolKind::Function),
        ];
        let deps = find_dependencies(dir.path(), &s, content, &all_symbols, &HashMap::new());
        let helper_count = deps.iter().filter(|d| d.name == "helper").count();
        assert_eq!(helper_count, 1);
    }

    #[test]
    fn prefers_cross_file_match() {
        let dir = make_root();
        let content = "fn caller() {\n    helper();\n}\nfn helper() {}\n";
        std::fs::write(dir.path().join("main.rs"), content).unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn helper() {}\n").unwrap();

        let s = sym("caller", "main.rs", 1, SymbolKind::Function);
        let all_symbols = vec![
            sym("caller", "main.rs", 1, SymbolKind::Function),
            sym("helper", "main.rs", 4, SymbolKind::Function),
            sym("helper", "lib.rs", 1, SymbolKind::Function),
        ];
        let deps = find_dependencies(dir.path(), &s, content, &all_symbols, &HashMap::new());
        let h = deps.iter().find(|d| d.name == "helper").unwrap();
        assert_eq!(h.location.as_ref().unwrap().0, "lib.rs");
    }
}
