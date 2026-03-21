use std::collections::HashMap;
use std::path::Path;

use crate::compress::{self, Lang};
use crate::symbol::{Symbol, SymbolKind};

use super::call_sites::contains_identifier;
use super::{Hierarchy, HierarchyEntry};

// ── Hierarchy extraction ────────────────────────────────────────────

pub(crate) fn extract_hierarchy(
    _root: &Path,
    sym: &Symbol,
    content: &str,
    all_symbols: &[Symbol],
    imports: &HashMap<String, String>,
    pre_parsed: Option<&tree_sitter::Tree>,
) -> Option<Hierarchy> {
    if !matches!(
        sym.kind,
        SymbolKind::Class | SymbolKind::Struct | SymbolKind::Trait | SymbolKind::Interface
    ) {
        return None;
    }

    let lang = compress::detect_lang(&sym.file)?;
    let owned_tree;
    let tree = match pre_parsed {
        Some(t) => t,
        None => {
            owned_tree = compress::parse_source(content, lang)?;
            &owned_tree
        }
    };

    // Extract direct parent names from the class definition
    let root_node = tree.root_node();
    let def_node = super::definition::find_definition_node(root_node, content, sym, lang)?;
    let parent_names = extract_parent_names(def_node, content, lang);

    // Resolve parents
    let parents: Vec<HierarchyEntry> = parent_names
        .iter()
        .map(|name| {
            // Try to find in project
            let project_sym = all_symbols
                .iter()
                .find(|s| s.name == *name && s.file != sym.file)
                .or_else(|| all_symbols.iter().find(|s| s.name == *name));

            if let Some(s) = project_sym {
                HierarchyEntry {
                    name: name.clone(),
                    location: Some((s.file.clone(), s.line)),
                    external_module: imports.get(name).cloned(),
                }
            } else {
                HierarchyEntry {
                    name: name.clone(),
                    location: None,
                    external_module: imports.get(name).cloned(),
                }
            }
        })
        .collect();

    // Find children: classes in the index whose signature shows this symbol as a parent
    let children: Vec<HierarchyEntry> = all_symbols
        .iter()
        .filter(|s| matches!(s.kind, SymbolKind::Class | SymbolKind::Struct))
        .filter(|s| s.name != sym.name)
        .filter(|s| {
            let sig = &s.signature;
            // Python: class Child(Parent, ...)
            if let Some(paren_start) = sig.find('(') {
                let after_paren = &sig[paren_start..];
                if contains_identifier(after_paren, &sym.name) {
                    return true;
                }
            }
            // Java/TS/JS: class Child extends Parent
            if sig.contains("extends") || sig.contains("implements") {
                let after_keyword = sig.find("extends").map(|i| &sig[i + 7..]).unwrap_or("");
                if contains_identifier(after_keyword, &sym.name) {
                    return true;
                }
                let after_impl = sig.find("implements").map(|i| &sig[i + 10..]).unwrap_or("");
                if contains_identifier(after_impl, &sym.name) {
                    return true;
                }
            }
            // C++: class Derived : public Base
            if sig.contains(':') && !sig.contains("::")
                || sig.matches(':').count() > sig.matches("::").count() * 2
            {
                // Has a non-scope-resolution colon — likely inheritance
                if let Some(colon_idx) = sig.find(':') {
                    let after_colon = &sig[colon_idx + 1..];
                    // Skip if this is just a scope resolution
                    if !after_colon.starts_with(':') && contains_identifier(after_colon, &sym.name)
                    {
                        return true;
                    }
                }
            }
            false
        })
        .map(|s| HierarchyEntry {
            name: s.name.clone(),
            location: Some((s.file.clone(), s.line)),
            external_module: None,
        })
        .collect();

    if parents.is_empty() && children.is_empty() {
        return None;
    }

    Some(Hierarchy { parents, children })
}

fn extract_parent_names(def_node: tree_sitter::Node, content: &str, lang: Lang) -> Vec<String> {
    match lang {
        Lang::Python => {
            // class Foo(Bar, Baz): → superclasses is an argument_list
            let supers = def_node.child_by_field_name("superclasses").or_else(|| {
                // Fallback: look for argument_list child
                let mut cursor = def_node.walk();
                if cursor.goto_first_child() {
                    loop {
                        if cursor.node().kind() == "argument_list" {
                            return Some(cursor.node());
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
                None
            });

            let Some(supers_node) = supers else {
                return Vec::new();
            };

            let mut names = Vec::new();
            let mut cursor = supers_node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    match child.kind() {
                        "identifier" => {
                            names.push(compress::node_text(content, child).to_string());
                        }
                        "attribute" => {
                            // e.g. models.Model — take the last part
                            let full = compress::node_text(content, child);
                            if let Some(last) = full.rsplit('.').next() {
                                names.push(last.to_string());
                            }
                        }
                        "keyword_argument" => {
                            // metaclass=ABCMeta — skip
                        }
                        _ => {}
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
            names
        }
        Lang::Java => {
            // class Foo extends Bar implements Baz, Qux
            let mut names = Vec::new();
            extract_type_identifiers_deep(def_node, content, "superclass", &mut names);
            extract_type_identifiers_deep(def_node, content, "interfaces", &mut names);
            // Fallback: walk children looking for type_identifier after "extends"/"implements"
            if names.is_empty() {
                extract_extends_names(def_node, content, &mut names);
            }
            names
        }
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
            // class Foo extends Bar { ... }
            let mut names = Vec::new();
            extract_extends_names(def_node, content, &mut names);
            names
        }
        Lang::C | Lang::Cpp => {
            // class Derived : public Base, protected Other { ... }
            let mut names = Vec::new();
            let mut cursor = def_node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "base_class_clause" {
                        let mut inner = child.walk();
                        if inner.goto_first_child() {
                            loop {
                                if inner.node().kind() == "base_class_specifier" {
                                    collect_type_idents(inner.node(), content, &mut names);
                                }
                                if !inner.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
            // Fallback: text-based "class X : public Y"
            if names.is_empty() {
                let text = compress::node_text(content, def_node);
                let first_line = text.lines().next().unwrap_or("");
                if let Some(colon_idx) = first_line.find(':') {
                    let after = &first_line[colon_idx + 1..];
                    let until_brace = after.split('{').next().unwrap_or(after);
                    for part in until_brace.split(',') {
                        let cleaned = part
                            .trim()
                            .trim_start_matches("public")
                            .trim()
                            .trim_start_matches("protected")
                            .trim()
                            .trim_start_matches("private")
                            .trim()
                            .trim_start_matches("virtual")
                            .trim();
                        let name = cleaned.split('<').next().unwrap_or("").trim();
                        let name = name.split_whitespace().next().unwrap_or("").trim();
                        if !name.is_empty()
                            && name.chars().next().is_some_and(|c| c.is_alphabetic())
                        {
                            names.push(name.to_string());
                        }
                    }
                }
            }
            names
        }
        _ => Vec::new(),
    }
}

/// Extract type identifiers from a named field node (e.g. "superclass" → dig into it for type_identifier)
fn extract_type_identifiers_deep(
    parent: tree_sitter::Node,
    content: &str,
    field_name: &str,
    names: &mut Vec<String>,
) {
    if let Some(field_node) = parent.child_by_field_name(field_name) {
        collect_type_idents(field_node, content, names);
    }
}

fn collect_type_idents(node: tree_sitter::Node, content: &str, names: &mut Vec<String>) {
    if matches!(node.kind(), "type_identifier" | "identifier") {
        let text = compress::node_text(content, node).to_string();
        // Skip keywords that might appear
        if !matches!(text.as_str(), "extends" | "implements" | "super" | "class") {
            names.push(text);
            return; // Don't recurse into this node
        }
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_type_idents(cursor.node(), content, names);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Walk children of a class node looking for identifiers after "extends" or "implements" keywords.
fn extract_extends_names(def_node: tree_sitter::Node, content: &str, names: &mut Vec<String>) {
    let full_text = compress::node_text(content, def_node);
    let first_line = full_text.lines().next().unwrap_or("");

    for keyword in ["extends", "implements"] {
        if let Some(idx) = first_line.find(keyword) {
            let after = &first_line[idx + keyword.len()..];
            // Take until { or end of line
            let until_brace = after.split('{').next().unwrap_or(after);
            // Also split on next keyword
            let until_keyword = until_brace
                .split("implements")
                .next()
                .unwrap_or(until_brace);
            for name in until_keyword.split(',') {
                let name = name.trim().split('<').next().unwrap_or("").trim();
                let name = name.split_whitespace().next().unwrap_or("").trim();
                if !name.is_empty()
                    && name.chars().next().is_some_and(|c| c.is_uppercase())
                    && !matches!(name, "extends" | "implements")
                {
                    names.push(name.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolKind;
    use std::collections::HashMap;

    fn sym(name: &str, file: &str, line: usize, kind: SymbolKind, sig: &str) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind,
            file: file.to_string(),
            line,
            signature: sig.to_string(),
            parent: None,
            keywords: Vec::new(),
        }
    }

    fn class_sym(name: &str, file: &str, line: usize, sig: &str) -> Symbol {
        sym(name, file, line, SymbolKind::Class, sig)
    }

    // ── extract_hierarchy ───────────────────────────────────────

    #[test]
    fn non_class_returns_none() {
        let s = sym("foo", "test.py", 1, SymbolKind::Function, "def foo():");
        let result = extract_hierarchy(
            Path::new("."),
            &s,
            "def foo():\n    pass\n",
            &[],
            &HashMap::new(),
            None,
        );
        assert!(result.is_none());
    }

    #[test]
    fn python_single_parent() {
        let content = "class Child(Parent):\n    pass\n";
        let s = class_sym("Child", "test.py", 1, "class Child(Parent):");
        let parent_sym = class_sym("Parent", "base.py", 1, "class Parent:");
        let result = extract_hierarchy(
            Path::new("."),
            &s,
            content,
            &[parent_sym],
            &HashMap::new(),
            None,
        );
        assert!(result.is_some());
        let h = result.unwrap();
        assert_eq!(h.parents.len(), 1);
        assert_eq!(h.parents[0].name, "Parent");
    }

    #[test]
    fn python_multiple_parents() {
        let content = "class Child(Base, Mixin):\n    pass\n";
        let s = class_sym("Child", "test.py", 1, "class Child(Base, Mixin):");
        let result = extract_hierarchy(Path::new("."), &s, content, &[], &HashMap::new(), None);
        assert!(result.is_some());
        let h = result.unwrap();
        assert_eq!(h.parents.len(), 2);
    }

    #[test]
    fn python_no_parents_no_children() {
        let content = "class Standalone:\n    pass\n";
        let s = class_sym("Standalone", "test.py", 1, "class Standalone:");
        let result = extract_hierarchy(Path::new("."), &s, content, &[], &HashMap::new(), None);
        assert!(result.is_none());
    }

    #[test]
    fn python_dotted_parent() {
        let content = "class MyModel(models.Model):\n    pass\n";
        let s = class_sym("MyModel", "test.py", 1, "class MyModel(models.Model):");
        let result = extract_hierarchy(Path::new("."), &s, content, &[], &HashMap::new(), None);
        assert!(result.is_some());
        let h = result.unwrap();
        assert_eq!(h.parents[0].name, "Model");
    }

    #[test]
    fn finds_children() {
        let content = "class Base:\n    pass\n";
        let s = class_sym("Base", "base.py", 1, "class Base:");
        let child = class_sym("Child", "child.py", 1, "class Child(Base):");
        let result =
            extract_hierarchy(Path::new("."), &s, content, &[child], &HashMap::new(), None);
        assert!(result.is_some());
        let h = result.unwrap();
        assert_eq!(h.children.len(), 1);
        assert_eq!(h.children[0].name, "Child");
    }

    #[test]
    fn java_extends_child_detection() {
        let content = "class Base {\n}\n";
        let s = class_sym("Base", "Base.java", 1, "class Base {");
        let child = class_sym("Child", "Child.java", 1, "class Child extends Base {");
        let result =
            extract_hierarchy(Path::new("."), &s, content, &[child], &HashMap::new(), None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().children.len(), 1);
    }

    #[test]
    fn java_implements_child_detection() {
        let content = "interface Runnable {\n}\n";
        let s = sym(
            "Runnable",
            "Runnable.java",
            1,
            SymbolKind::Interface,
            "interface Runnable {",
        );
        let child = class_sym(
            "Worker",
            "Worker.java",
            1,
            "class Worker implements Runnable {",
        );
        let result =
            extract_hierarchy(Path::new("."), &s, content, &[child], &HashMap::new(), None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().children.len(), 1);
    }

    #[test]
    fn external_parent_module() {
        let content = "class Child(ExternalBase):\n    pass\n";
        let s = class_sym("Child", "test.py", 1, "class Child(ExternalBase):");
        let mut imports = HashMap::new();
        imports.insert("ExternalBase".to_string(), "some_lib".to_string());
        let result = extract_hierarchy(Path::new("."), &s, content, &[], &imports, None);
        assert!(result.is_some());
        let h = result.unwrap();
        assert_eq!(h.parents[0].external_module.as_deref(), Some("some_lib"));
    }

    #[test]
    fn unsupported_lang_returns_none() {
        let s = class_sym("Foo", "test.txt", 1, "class Foo:");
        let result = extract_hierarchy(
            Path::new("."),
            &s,
            "class Foo:\n    pass\n",
            &[],
            &HashMap::new(),
            None,
        );
        assert!(result.is_none());
    }

    #[test]
    fn with_pre_parsed_tree() {
        let content = "class Child(Parent):\n    pass\n";
        let tree = compress::parse_source(content, Lang::Python).unwrap();
        let s = class_sym("Child", "test.py", 1, "class Child(Parent):");
        let result = extract_hierarchy(
            Path::new("."),
            &s,
            content,
            &[],
            &HashMap::new(),
            Some(&tree),
        );
        assert!(result.is_some());
    }

    #[test]
    fn ts_extends_parent() {
        let content = "class Child extends Parent {\n    constructor() { super(); }\n}\n";
        let s = class_sym("Child", "test.ts", 1, "class Child extends Parent {");
        let result = extract_hierarchy(Path::new("."), &s, content, &[], &HashMap::new(), None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().parents[0].name, "Parent");
    }

    #[test]
    fn struct_kind_accepted() {
        let content = "pub struct MyStruct {}\n";
        let s = sym(
            "MyStruct",
            "test.rs",
            1,
            SymbolKind::Struct,
            "pub struct MyStruct {}",
        );
        let result = extract_hierarchy(Path::new("."), &s, content, &[], &HashMap::new(), None);
        // No parents or children → None
        assert!(result.is_none());
    }

    #[test]
    fn trait_kind_accepted() {
        let content = "pub trait MyTrait {}\n";
        let s = sym(
            "MyTrait",
            "test.rs",
            1,
            SymbolKind::Trait,
            "pub trait MyTrait {}",
        );
        let result = extract_hierarchy(Path::new("."), &s, content, &[], &HashMap::new(), None);
        assert!(result.is_none());
    }

    #[test]
    fn cpp_inheritance() {
        let content = "class Derived : public Base {\n    void method() {}\n};\n";
        let s = class_sym("Derived", "test.cpp", 1, "class Derived : public Base {");
        let result = extract_hierarchy(Path::new("."), &s, content, &[], &HashMap::new(), None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().parents[0].name, "Base");
    }
}
