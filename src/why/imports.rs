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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── extract_python_imports ──────────────────────────────────

    #[test]
    fn python_from_import() {
        let imports = extract_python_imports("from os import path\n");
        assert_eq!(imports.get("path"), Some(&"os".to_string()));
    }

    #[test]
    fn python_from_import_multiple() {
        let imports = extract_python_imports("from os import path, getcwd\n");
        assert_eq!(imports.get("path"), Some(&"os".to_string()));
        assert_eq!(imports.get("getcwd"), Some(&"os".to_string()));
    }

    #[test]
    fn python_from_import_as() {
        let imports = extract_python_imports("from numpy import array as np_array\n");
        assert_eq!(imports.get("array"), Some(&"numpy".to_string()));
    }

    #[test]
    fn python_import_statement() {
        let imports = extract_python_imports("import os\n");
        assert_eq!(imports.get("os"), Some(&"os".to_string()));
    }

    #[test]
    fn python_import_dotted() {
        let imports = extract_python_imports("import os.path\n");
        assert_eq!(imports.get("path"), Some(&"os.path".to_string()));
    }

    #[test]
    fn python_import_as() {
        let imports = extract_python_imports("import numpy as np\n");
        assert_eq!(imports.get("numpy"), Some(&"numpy".to_string()));
    }

    #[test]
    fn python_import_multiple() {
        let imports = extract_python_imports("import os, sys\n");
        assert!(imports.contains_key("os"));
        assert!(imports.contains_key("sys"));
    }

    #[test]
    fn python_from_import_paren() {
        let imports = extract_python_imports("from os import (path, getcwd)\n");
        assert!(imports.contains_key("path"));
        assert!(imports.contains_key("getcwd"));
    }

    #[test]
    fn python_relative_import() {
        let imports = extract_python_imports("from . import utils\n");
        assert_eq!(imports.get("utils"), Some(&".".to_string()));
    }

    #[test]
    fn python_no_imports() {
        let imports = extract_python_imports("x = 1\nprint(x)\n");
        assert!(imports.is_empty());
    }

    // ── extract_rust_imports ────────────────────────────────────

    #[test]
    fn rust_simple_use() {
        let imports = extract_rust_imports("use std::collections::HashMap;\n");
        assert_eq!(
            imports.get("HashMap"),
            Some(&"std::collections".to_string())
        );
    }

    #[test]
    fn rust_brace_use() {
        let imports = extract_rust_imports("use std::collections::{HashMap, HashSet};\n");
        assert_eq!(
            imports.get("HashMap"),
            Some(&"std::collections".to_string())
        );
        assert_eq!(
            imports.get("HashSet"),
            Some(&"std::collections".to_string())
        );
    }

    #[test]
    fn rust_use_as() {
        let imports = extract_rust_imports("use std::io::Result as IoResult;\n");
        assert_eq!(imports.get("Result"), Some(&"std::io".to_string()));
    }

    #[test]
    fn rust_use_self_excluded() {
        let imports = extract_rust_imports("use std::io::{self, Read};\n");
        assert!(!imports.contains_key("self"));
        assert!(imports.contains_key("Read"));
    }

    #[test]
    fn rust_no_imports() {
        let imports = extract_rust_imports("fn main() {}\n");
        assert!(imports.is_empty());
    }

    // ── extract_js_imports ──────────────────────────────────────

    #[test]
    fn js_named_import() {
        let imports = extract_js_imports("import { useState } from 'react';\n");
        assert_eq!(imports.get("useState"), Some(&"react".to_string()));
    }

    #[test]
    fn js_multiple_named_imports() {
        let imports = extract_js_imports("import { useState, useEffect } from 'react';\n");
        assert!(imports.contains_key("useState"));
        assert!(imports.contains_key("useEffect"));
    }

    #[test]
    fn js_default_import() {
        let imports = extract_js_imports("import React from 'react';\n");
        assert_eq!(imports.get("React"), Some(&"react".to_string()));
    }

    #[test]
    fn js_import_as() {
        let imports = extract_js_imports("import { foo as bar } from 'baz';\n");
        assert_eq!(imports.get("foo"), Some(&"baz".to_string()));
    }

    #[test]
    fn js_star_import_excluded() {
        let imports = extract_js_imports("import * as utils from './utils';\n");
        assert!(!imports.contains_key("*"));
    }

    #[test]
    fn js_no_imports() {
        let imports = extract_js_imports("const x = 1;\n");
        assert!(imports.is_empty());
    }

    #[test]
    fn js_double_quote_import() {
        let imports = extract_js_imports("import { Foo } from \"bar\";\n");
        assert_eq!(imports.get("Foo"), Some(&"bar".to_string()));
    }

    // ── extract_c_includes ──────────────────────────────────────

    #[test]
    fn c_system_include() {
        let imports = extract_c_includes("#include <stdio.h>\n", "test.c", Path::new("/tmp"));
        assert!(imports.contains_key("stdio.h"));
        assert_eq!(imports.get("stdio.h"), Some(&"<stdio.h>".to_string()));
    }

    #[test]
    fn c_local_include_not_found() {
        let imports = extract_c_includes("#include \"myheader.h\"\n", "test.c", Path::new("/tmp"));
        assert!(imports.contains_key("myheader.h"));
    }

    #[test]
    fn c_local_include_resolved() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("header.h"),
            "int add(int a, int b);\nstruct Point { int x; int y; };\n",
        )
        .unwrap();
        let imports = extract_c_includes("#include \"header.h\"\n", "test.c", dir.path());
        // Should resolve symbols from the header
        assert!(!imports.is_empty());
    }

    #[test]
    fn c_no_includes() {
        let imports = extract_c_includes("int main() { return 0; }\n", "test.c", Path::new("/tmp"));
        assert!(imports.is_empty());
    }

    #[test]
    fn c_preproc_not_include() {
        let imports = extract_c_includes(
            "#define FOO 1\n#ifdef FOO\n#endif\n",
            "test.c",
            Path::new("/tmp"),
        );
        assert!(imports.is_empty());
    }

    // ── extract_file_imports ────────────────────────────────────

    #[test]
    fn dispatches_to_python() {
        let imports = extract_file_imports("from os import path\n", "test.py", Path::new("."));
        assert!(imports.contains_key("path"));
    }

    #[test]
    fn dispatches_to_rust() {
        let imports = extract_file_imports("use std::io::Read;\n", "test.rs", Path::new("."));
        assert!(imports.contains_key("Read"));
    }

    #[test]
    fn dispatches_to_js() {
        let imports =
            extract_file_imports("import { Foo } from 'bar';\n", "test.js", Path::new("."));
        assert!(imports.contains_key("Foo"));
    }

    #[test]
    fn dispatches_to_ts() {
        let imports =
            extract_file_imports("import { Foo } from 'bar';\n", "test.ts", Path::new("."));
        assert!(imports.contains_key("Foo"));
    }

    #[test]
    fn dispatches_to_c() {
        let imports = extract_file_imports("#include <stdio.h>\n", "test.c", Path::new("."));
        assert!(imports.contains_key("stdio.h"));
    }

    #[test]
    fn unsupported_lang_returns_empty() {
        let imports = extract_file_imports("something\n", "test.txt", Path::new("."));
        assert!(imports.is_empty());
    }

    #[test]
    fn go_returns_empty() {
        let imports = extract_file_imports("import \"fmt\"\n", "test.go", Path::new("."));
        assert!(imports.is_empty());
    }

    // ── resolve_relative_import ─────────────────────────────────

    #[test]
    fn non_relative_returns_none() {
        assert!(resolve_relative_import("os", "test.py", Path::new(".")).is_none());
    }

    #[test]
    fn dot_only_returns_none() {
        assert!(resolve_relative_import(".", "test.py", Path::new(".")).is_none());
    }

    #[test]
    fn single_dot_module() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("pkg")).unwrap();
        fs::write(dir.path().join("pkg/utils.py"), "x = 1\n").unwrap();
        let result = resolve_relative_import(".utils", "pkg/main.py", dir.path());
        assert!(result.is_some());
        assert!(result.unwrap().contains("utils.py"));
    }

    #[test]
    fn double_dot_module() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("pkg/sub")).unwrap();
        fs::write(dir.path().join("pkg/helper.py"), "x = 1\n").unwrap();
        let result = resolve_relative_import("..helper", "pkg/sub/main.py", dir.path());
        assert!(result.is_some());
        assert!(result.unwrap().contains("helper.py"));
    }

    #[test]
    fn package_init_resolution() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("pkg/utils")).unwrap();
        fs::write(dir.path().join("pkg/utils/__init__.py"), "").unwrap();
        let result = resolve_relative_import(".utils", "pkg/main.py", dir.path());
        assert!(result.is_some());
        assert!(result.unwrap().contains("__init__.py"));
    }

    #[test]
    fn module_not_found() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("pkg")).unwrap();
        let result = resolve_relative_import(".nonexistent", "pkg/main.py", dir.path());
        assert!(result.is_none());
    }

    // ── normalize_path ──────────────────────────────────────────

    #[test]
    fn normalize_collapses_parent() {
        let result = normalize_path(Path::new("a/b/../c"));
        assert_eq!(result, "a/c");
    }

    #[test]
    fn normalize_removes_curdir() {
        let result = normalize_path(Path::new("a/./b"));
        assert_eq!(result, "a/b");
    }

    #[test]
    fn normalize_simple() {
        let result = normalize_path(Path::new("a/b/c"));
        assert_eq!(result, "a/b/c");
    }
}
