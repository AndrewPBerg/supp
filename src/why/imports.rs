use std::collections::HashMap;
use std::path::Path;

use crate::compress::{self, Lang};

// ── Import extraction ───────────────────────────────────────────────

/// Maps imported name → module path (e.g. "BaseModel" → "pydantic")
pub(crate) fn extract_file_imports(
    content: &str,
    file_path: &str,
    root: &Path,
) -> HashMap<String, String> {
    let lang = compress::detect_lang(file_path);
    match lang {
        Some(Lang::Python) => extract_python_imports(content),
        Some(Lang::Rust) => extract_rust_imports(content),
        Some(Lang::JavaScript | Lang::TypeScript | Lang::Tsx) => extract_js_imports(content),
        Some(Lang::C | Lang::Cpp) => extract_c_includes(content, file_path, root),
        _ => HashMap::new(),
    }
}

pub(super) fn extract_python_imports(content: &str) -> HashMap<String, String> {
    let mut imports = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("from ") {
            // from module import name1, name2, ...
            if let Some((module, names_part)) = rest.split_once(" import ") {
                let module = module.trim();
                // Handle multiline (trailing backslash or paren) — just get first line
                let names_str = names_part.trim_start_matches('(').trim_end_matches(')');
                for name in names_str.split(',') {
                    let name = name.trim().trim_end_matches('\\').trim();
                    let actual = name
                        .split_once(" as ")
                        .map(|(n, _)| n)
                        .unwrap_or(name)
                        .trim();
                    if !actual.is_empty()
                        && actual.chars().next().is_some_and(|c| c.is_alphabetic())
                    {
                        imports.insert(actual.to_string(), module.to_string());
                    }
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("import ") {
            for part in rest.split(',') {
                let part = part.trim();
                let module = part
                    .split_once(" as ")
                    .map(|(m, _)| m)
                    .unwrap_or(part)
                    .trim();
                let short_name = module.rsplit('.').next().unwrap_or(module);
                if !short_name.is_empty() {
                    imports.insert(short_name.to_string(), module.to_string());
                }
            }
        }
    }
    imports
}

pub(super) fn extract_rust_imports(content: &str) -> HashMap<String, String> {
    let mut imports = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("use ") {
            let path = rest.trim_end_matches(';').trim();
            // use foo::bar::Baz → "Baz" from "foo::bar"
            // use foo::bar::{Baz, Qux} → "Baz" from "foo::bar", "Qux" from "foo::bar"
            if let Some(brace_start) = path.find('{') {
                let prefix = path[..brace_start]
                    .trim_end_matches(':')
                    .trim_end_matches(':');
                let inner = path[brace_start + 1..].trim_end_matches('}');
                for name in inner.split(',') {
                    let name = name
                        .trim()
                        .split_once(" as ")
                        .map(|(n, _)| n)
                        .unwrap_or(name.trim());
                    let name = name.trim();
                    if !name.is_empty() && name != "self" {
                        imports.insert(name.to_string(), prefix.to_string());
                    }
                }
            } else if let Some((prefix, name)) = path.rsplit_once("::") {
                let name = name
                    .split_once(" as ")
                    .map(|(n, _)| n)
                    .unwrap_or(name)
                    .trim();
                if !name.is_empty() {
                    imports.insert(name.to_string(), prefix.to_string());
                }
            }
        }
    }
    imports
}

pub(super) fn extract_js_imports(content: &str) -> HashMap<String, String> {
    let mut imports = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("import ") {
            continue;
        }
        // import { X, Y } from 'module'
        // import X from 'module'
        if let Some(from_idx) = trimmed.find(" from ") {
            let names_part = &trimmed[7..from_idx]; // skip "import "
            let module = trimmed[from_idx + 6..]
                .trim()
                .trim_matches(|c| c == '\'' || c == '"' || c == ';');

            let names_str = names_part
                .trim()
                .trim_start_matches('{')
                .trim_end_matches('}');

            for name in names_str.split(',') {
                let name = name
                    .trim()
                    .split_once(" as ")
                    .map(|(n, _)| n)
                    .unwrap_or(name.trim());
                if !name.is_empty() && name != "default" && name != "*" {
                    imports.insert(name.to_string(), module.to_string());
                }
            }
        }
    }
    imports
}

fn extract_c_includes(content: &str, file_path: &str, root: &Path) -> HashMap<String, String> {
    let mut imports = HashMap::new();
    let file_dir = Path::new(file_path).parent().unwrap_or(Path::new(""));

    for line in content.lines() {
        let trimmed = line.trim();
        // Normalize: strip '#', optional whitespace, then "include"
        let rest = if let Some(after) = trimmed.strip_prefix('#') {
            let after = after.trim_start();
            if let Some(rest) = after.strip_prefix("include") {
                rest.trim()
            } else {
                continue;
            }
        } else {
            continue;
        };

        // "file.h" — local include
        if let Some(stripped) = rest.strip_prefix('"') {
            if let Some(end) = stripped.find('"') {
                let header = &stripped[..end];
                if let Some(resolved) = resolve_c_include(header, file_dir, root) {
                    let abs = root.join(&resolved);
                    if let Ok(header_content) = std::fs::read_to_string(&abs) {
                        let header_syms = scan_header_symbols(&header_content, &resolved);
                        for sym_name in header_syms {
                            imports.insert(sym_name, resolved.clone());
                        }
                    }
                } else {
                    imports.insert(header.to_string(), header.to_string());
                }
            }
        }
        // <stdlib.h> — system include
        else if rest.starts_with('<')
            && let Some(end) = rest.find('>')
        {
            let header = &rest[1..end];
            imports.insert(header.to_string(), format!("<{}>", header));
        }
    }
    imports
}

/// Resolve a local #include path relative to the file and common search dirs.
fn resolve_c_include(header: &str, file_dir: &Path, root: &Path) -> Option<String> {
    // 1. Relative to including file's directory
    let candidate = file_dir.join(header);
    let norm = normalize_path(&candidate);
    if root.join(&norm).exists() {
        return Some(norm);
    }
    // 2. Relative to project root
    if root.join(header).exists() {
        return Some(header.to_string());
    }
    // 3. Common include dirs
    for dir in &["include", "inc", "src"] {
        let candidate = Path::new(dir).join(header);
        if root.join(&candidate).exists() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

/// Normalize a relative path (collapse `..` and `.`).
fn normalize_path(path: &Path) -> String {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::CurDir => {}
            std::path::Component::Normal(p) => parts.push(p),
            _ => {}
        }
    }
    parts
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Quickly scan a C/C++ header for symbol-like declarations (struct/class/enum names, function names).
fn scan_header_symbols(content: &str, file_path: &str) -> Vec<String> {
    let lang = match compress::detect_lang(file_path) {
        Some(l) => l,
        None => return Vec::new(),
    };
    let tree = match compress::parse_source(content, lang) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut names = Vec::new();
    collect_header_decl_names(tree.root_node(), content, &mut names);
    names
}

fn collect_header_decl_names(node: tree_sitter::Node, content: &str, names: &mut Vec<String>) {
    match node.kind() {
        "function_definition" | "declaration" => {
            if let Some(declarator) = node.child_by_field_name("declarator")
                && let Some(name) = find_c_decl_name(declarator, content)
            {
                names.push(name);
            }
        }
        "struct_specifier" | "enum_specifier" | "class_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let text = compress::node_text(content, name_node);
                if !text.is_empty() {
                    names.push(text.to_string());
                }
            }
        }
        "type_definition" => {
            // typedef struct { ... } Name; → declarator has the typedef name
            if let Some(declarator) = node.child_by_field_name("declarator")
                && let Some(name) = find_c_decl_name(declarator, content)
            {
                names.push(name);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_header_decl_names(cursor.node(), content, names);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn find_c_decl_name(node: tree_sitter::Node, content: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" => Some(compress::node_text(content, node).to_string()),
        "function_declarator" | "pointer_declarator" | "array_declarator" | "init_declarator" => {
            if let Some(decl) = node.child_by_field_name("declarator") {
                return find_c_decl_name(decl, content);
            }
            // Fallback: first child
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    if let Some(name) = find_c_decl_name(cursor.node(), content) {
                        return Some(name);
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Try to resolve a relative import to a project file path.
pub(crate) fn resolve_relative_import(
    module: &str,
    from_file: &str,
    root: &Path,
) -> Option<String> {
    if !module.starts_with('.') {
        return None;
    }

    let dots = module.chars().take_while(|&c| c == '.').count();
    let module_name = &module[dots..];

    let from_dir = Path::new(from_file).parent().unwrap_or(Path::new(""));

    // Walk up `dots - 1` directories
    let mut base = from_dir.to_path_buf();
    for _ in 1..dots {
        base = base.parent().unwrap_or(Path::new("")).to_path_buf();
    }

    // Convert module.name → module/name.py
    let rel_path = if module_name.is_empty() {
        // `from . import X` — look for X.py in same dir
        return None; // Can't resolve without the imported name
    } else {
        let parts: Vec<&str> = module_name.split('.').collect();
        base.join(parts.join("/"))
    };

    // Try .py extension
    let py_path = rel_path.with_extension("py");
    let abs = root.join(&py_path);
    if abs.exists() {
        return Some(py_path.to_string_lossy().to_string());
    }

    // Try as package (__init__.py)
    let init_path = rel_path.join("__init__.py");
    let abs = root.join(&init_path);
    if abs.exists() {
        return Some(init_path.to_string_lossy().to_string());
    }

    None
}
