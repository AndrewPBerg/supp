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
