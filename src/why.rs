use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;

use crate::compress::{self, Lang};
use crate::symbol::{self, SearchResult, Symbol, SymbolKind};

// ── Result types ────────────────────────────────────────────────────

pub struct WhyResult {
    pub symbol: Symbol,
    pub doc_comment: Option<String>,
    pub full_definition: String,
    pub call_sites: Vec<CallSite>,
    pub dependencies: Vec<Dependency>,
    pub hierarchy: Option<Hierarchy>,
    pub plain: String,
}

pub struct CallSite {
    pub file: String,
    pub line: usize,
    pub context: String,
    pub caller: Option<String>,
}

pub struct Dependency {
    pub name: String,
    pub kind: Option<SymbolKind>,
    pub location: Option<(String, usize)>, // (file, line) for in-project
    pub import_from: Option<String>,       // module path if imported
}

pub struct Hierarchy {
    pub parents: Vec<HierarchyEntry>,
    pub children: Vec<HierarchyEntry>,
}

pub struct HierarchyEntry {
    pub name: String,
    pub location: Option<(String, usize)>,
    pub external_module: Option<String>,
}

// ── Public API ──────────────────────────────────────────────────────

pub fn explain(root: &str, query: &[String]) -> Result<WhyResult> {
    let root_path = std::fs::canonicalize(root)?;

    // 1. Find the symbol using the existing index
    let search = symbol::search(root, query)?;
    let sym = pick_best_match(&search, query)?;

    // 2. Read the source file
    let abs_path = root_path.join(&sym.file);
    let content = std::fs::read_to_string(&abs_path)?;

    // 3. Extract doc comments (language-aware: Python docstrings vs Rust /// comments)
    let doc_comment = extract_doc_comment(&content, &sym);

    // 4. Extract the full definition using tree-sitter
    let full_definition = extract_full_definition(&content, &sym);

    // 5. Find call sites across the codebase
    let call_sites = find_call_sites(&root_path, &sym);

    // 6. Load full symbol index + file imports for dependency resolution
    let all_symbols = symbol::load_symbols(&root_path);
    let imports = extract_file_imports(&content, &sym.file, &root_path);

    // 7. Find dependencies (what this symbol calls/uses)
    let dependencies = find_dependencies(&root_path, &sym, &content, &all_symbols, &imports);

    // 8. Extract class hierarchy (parents + children)
    let hierarchy = extract_hierarchy(&root_path, &sym, &content, &all_symbols, &imports);

    // 9. Build plain text for clipboard
    let plain = build_plain_text(
        &sym,
        &doc_comment,
        &full_definition,
        &call_sites,
        &dependencies,
        &hierarchy,
    );

    Ok(WhyResult {
        symbol: sym,
        doc_comment,
        full_definition,
        call_sites,
        dependencies,
        hierarchy,
        plain,
    })
}

// ── Symbol selection ────────────────────────────────────────────────

fn pick_best_match(search: &SearchResult, query: &[String]) -> Result<Symbol> {
    if search.matches.is_empty() {
        anyhow::bail!("no symbol found matching '{}'", query.join(" "));
    }

    let (sym, _score) = &search.matches[0];

    // Check for exact name match first
    let query_joined = query.join("_").to_lowercase();
    for (s, _) in &search.matches {
        if s.name.to_lowercase() == query_joined {
            return Ok(s.clone());
        }
    }

    // Also check "Parent::name" format
    for (s, _) in &search.matches {
        let full = if let Some(ref p) = s.parent {
            format!("{}::{}", p, s.name).to_lowercase()
        } else {
            s.name.to_lowercase()
        };
        if full == query_joined || full == query.join("::").to_lowercase() {
            return Ok(s.clone());
        }
    }

    Ok(sym.clone())
}

// ── Doc comment extraction (language-aware) ─────────────────────────

fn extract_doc_comment(content: &str, sym: &Symbol) -> Option<String> {
    let lang = compress::detect_lang(&sym.file);

    // Python: docstrings live inside the function/class body
    if lang == Some(Lang::Python) {
        if let Some(docstring) = extract_python_docstring(content, sym) {
            return Some(docstring);
        }
    }

    // Rust/C/JS/etc: comments live above the definition
    extract_comment_above(content, sym.line)
}

fn extract_python_docstring(content: &str, sym: &Symbol) -> Option<String> {
    let tree = compress::parse_source(content, Lang::Python)?;
    let root = tree.root_node();
    let def_node = find_definition_node(root, content, sym, Lang::Python)?;

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
    let inner = if s.starts_with("\"\"\"") && s.ends_with("\"\"\"") && s.len() >= 6 {
        &s[3..s.len() - 3]
    } else if s.starts_with("'''") && s.ends_with("'''") && s.len() >= 6 {
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
        } else if trimmed.starts_with("/**")
            || trimmed.ends_with("*/")
            || trimmed.starts_with('*')
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

// ── Full definition extraction ──────────────────────────────────────

fn extract_full_definition(content: &str, sym: &Symbol) -> String {
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

fn find_definition_node<'a>(
    node: tree_sitter::Node<'a>,
    content: &str,
    sym: &Symbol,
    _lang: Lang,
) -> Option<tree_sitter::Node<'a>> {
    let line = sym.line - 1; // tree-sitter uses 0-based

    if node.start_position().row == line {
        // Standard named definitions (fn, class, struct, etc.)
        if let Some(name_node) = node.child_by_field_name("name") {
            if compress::node_text(content, name_node) == sym.name {
                return Some(node);
            }
        }

        // C/C++: function_definition → declarator → function_declarator → identifier
        if node.kind() == "function_definition" {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if find_c_name_in_declarator(declarator, content) == Some(&sym.name) {
                    return Some(node);
                }
            }
        }

        // C/C++: struct_specifier, enum_specifier, class_specifier with name field
        if matches!(node.kind(), "struct_specifier" | "enum_specifier" | "class_specifier") {
            if let Some(name_node) = node.child_by_field_name("name") {
                if compress::node_text(content, name_node) == sym.name {
                    return Some(node);
                }
            }
        }

        // JS/TS: const MyComponent = (...) => { ... } (lexical_declaration wrapping arrow fn)
        if matches!(node.kind(), "lexical_declaration" | "variable_declaration") {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "variable_declarator" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if compress::node_text(content, name_node) == sym.name {
                                return Some(node);
                            }
                        }
                    }
                    if !cursor.goto_next_sibling() { break; }
                }
            }
        }

        // Python module-level assignments: expression_statement → assignment → left
        if node.kind() == "expression_statement" {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                let child = cursor.node();
                if child.kind() == "assignment" {
                    if let Some(left) = child.child_by_field_name("left") {
                        if left.kind() == "identifier"
                            && compress::node_text(content, left) == sym.name
                        {
                            return Some(node);
                        }
                    }
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
    if node.kind() == "qualified_identifier" {
        if let Some(name) = node.child_by_field_name("name") {
            if name.kind() == "identifier" {
                return Some(compress::node_text(content, name));
            }
        }
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if let Some(name) = find_c_name_in_declarator(cursor.node(), content) {
                return Some(name);
            }
            if !cursor.goto_next_sibling() { break; }
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

// ── Import extraction ───────────────────────────────────────────────

/// Maps imported name → module path (e.g. "BaseModel" → "pydantic")
pub(crate) fn extract_file_imports(content: &str, file_path: &str, root: &Path) -> HashMap<String, String> {
    let lang = compress::detect_lang(file_path);
    match lang {
        Some(Lang::Python) => extract_python_imports(content),
        Some(Lang::Rust) => extract_rust_imports(content),
        Some(Lang::JavaScript | Lang::TypeScript | Lang::Tsx) => extract_js_imports(content),
        Some(Lang::C | Lang::Cpp) => extract_c_includes(content, file_path, root),
        _ => HashMap::new(),
    }
}

fn extract_python_imports(content: &str) -> HashMap<String, String> {
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
                    let actual = name.split_once(" as ").map(|(n, _)| n).unwrap_or(name).trim();
                    if !actual.is_empty() && actual.chars().next().is_some_and(|c| c.is_alphabetic()) {
                        imports.insert(actual.to_string(), module.to_string());
                    }
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("import ") {
            for part in rest.split(',') {
                let part = part.trim();
                let module = part.split_once(" as ").map(|(m, _)| m).unwrap_or(part).trim();
                let short_name = module.rsplit('.').next().unwrap_or(module);
                if !short_name.is_empty() {
                    imports.insert(short_name.to_string(), module.to_string());
                }
            }
        }
    }
    imports
}

fn extract_rust_imports(content: &str) -> HashMap<String, String> {
    let mut imports = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("use ") {
            let path = rest.trim_end_matches(';').trim();
            // use foo::bar::Baz → "Baz" from "foo::bar"
            // use foo::bar::{Baz, Qux} → "Baz" from "foo::bar", "Qux" from "foo::bar"
            if let Some(brace_start) = path.find('{') {
                let prefix = path[..brace_start].trim_end_matches(':').trim_end_matches(':');
                let inner = path[brace_start + 1..].trim_end_matches('}');
                for name in inner.split(',') {
                    let name = name.trim().split_once(" as ").map(|(n, _)| n).unwrap_or(name.trim());
                    let name = name.trim();
                    if !name.is_empty() && name != "self" {
                        imports.insert(name.to_string(), prefix.to_string());
                    }
                }
            } else if let Some((prefix, name)) = path.rsplit_once("::") {
                let name = name.split_once(" as ").map(|(n, _)| n).unwrap_or(name).trim();
                if !name.is_empty() {
                    imports.insert(name.to_string(), prefix.to_string());
                }
            }
        }
    }
    imports
}

fn extract_js_imports(content: &str) -> HashMap<String, String> {
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
                let name = name.trim().split_once(" as ").map(|(n, _)| n).unwrap_or(name.trim());
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
        if rest.starts_with('"') {
            if let Some(end) = rest[1..].find('"') {
                let header = &rest[1..1 + end];
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
        else if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                let header = &rest[1..end];
                imports.insert(header.to_string(), format!("<{}>", header));
            }
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
            std::path::Component::ParentDir => { parts.pop(); }
            std::path::Component::CurDir => {}
            std::path::Component::Normal(p) => parts.push(p),
            _ => {}
        }
    }
    parts.iter()
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
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if let Some(name) = find_c_decl_name(declarator, content) {
                    names.push(name);
                }
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
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if let Some(name) = find_c_decl_name(declarator, content) {
                    names.push(name);
                }
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
        "identifier" => Some(compress::node_text(content, node).to_string()),
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
pub(crate) fn resolve_relative_import(module: &str, from_file: &str, root: &Path) -> Option<String> {
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

// ── Call site discovery ─────────────────────────────────────────────

pub(crate) fn find_call_sites(root: &Path, sym: &Symbol) -> Vec<CallSite> {
    let mut sites = Vec::new();
    let name = &sym.name;

    if name.len() <= 2 {
        return sites;
    }

    // For same-file filtering: find the definition's line span so we can skip it
    let def_span = find_definition_span(root, sym);

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
            if is_same_file {
                if let Some((def_start, def_end)) = def_span {
                    if line_num >= def_start && line_num <= def_end {
                        continue;
                    }
                }
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

/// Find the line span (start, end) of a symbol's definition for exclusion.
fn find_definition_span(root: &Path, sym: &Symbol) -> Option<(usize, usize)> {
    let abs = root.join(&sym.file);
    let content = std::fs::read_to_string(&abs).ok()?;
    let lang = compress::detect_lang(&sym.file)?;
    let tree = compress::parse_source(&content, lang)?;
    let node = find_definition_node(tree.root_node(), &content, sym, lang)?;
    Some((
        node.start_position().row + 1,
        node.end_position().row + 1,
    ))
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

            if is_fn {
                if let Some(name_node) = child.child_by_field_name("name") {
                    return Some(compress::node_text(content, name_node).to_string());
                }
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

// ── Dependency discovery ────────────────────────────────────────────

fn find_dependencies(
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
    let def_node = match find_definition_node(root_node, content, sym, lang) {
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
            let resolved = resolve_relative_import(module, &sym.file, root);

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
    // This captures types in parameters, return types, generics, etc.
    // We skip only the function/class name itself to avoid self-reference.
    let name_text = node
        .child_by_field_name("name")
        .map(|n| compress::node_text(content, n).to_string());

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            // Skip the name node itself (avoids self-reference)
            let is_name = name_text
                .as_ref()
                .is_some_and(|n| child.kind() == "identifier" && compress::node_text(content, child) == n);
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

// ── Hierarchy extraction ────────────────────────────────────────────

fn extract_hierarchy(
    _root: &Path,
    sym: &Symbol,
    content: &str,
    all_symbols: &[Symbol],
    imports: &HashMap<String, String>,
) -> Option<Hierarchy> {
    if !matches!(
        sym.kind,
        SymbolKind::Class | SymbolKind::Struct | SymbolKind::Trait | SymbolKind::Interface
    ) {
        return None;
    }

    let lang = compress::detect_lang(&sym.file)?;
    let tree = compress::parse_source(content, lang)?;

    // Extract direct parent names from the class definition
    let root_node = tree.root_node();
    let def_node = find_definition_node(root_node, content, sym, lang)?;
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
                let after_keyword = sig
                    .find("extends")
                    .map(|i| &sig[i + 7..])
                    .unwrap_or("");
                if contains_identifier(after_keyword, &sym.name) {
                    return true;
                }
                let after_impl = sig
                    .find("implements")
                    .map(|i| &sig[i + 10..])
                    .unwrap_or("");
                if contains_identifier(after_impl, &sym.name) {
                    return true;
                }
            }
            // C++: class Derived : public Base
            if sig.contains(':') && !sig.contains("::") || sig.matches(':').count() > sig.matches("::").count() * 2 {
                // Has a non-scope-resolution colon — likely inheritance
                if let Some(colon_idx) = sig.find(':') {
                    let after_colon = &sig[colon_idx + 1..];
                    // Skip if this is just a scope resolution
                    if !after_colon.starts_with(':') {
                        if contains_identifier(after_colon, &sym.name) {
                            return true;
                        }
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

fn extract_parent_names(
    def_node: tree_sitter::Node,
    content: &str,
    lang: Lang,
) -> Vec<String> {
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
            // tree-sitter-java: superclass node contains the full "extends ClassName" clause
            let mut names = Vec::new();
            // Walk all children to find the superclass and interfaces clauses
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
            // tree-sitter varies: may use class_heritage, extends_clause, or direct children
            let mut names = Vec::new();
            extract_extends_names(def_node, content, &mut names);
            names
        }
        Lang::C | Lang::Cpp => {
            // class Derived : public Base, protected Other { ... }
            // tree-sitter-cpp: class_specifier > base_class_clause > base_class_specifier
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
                                if !inner.goto_next_sibling() { break; }
                            }
                        }
                    }
                    if !cursor.goto_next_sibling() { break; }
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
                        let cleaned = part.trim()
                            .trim_start_matches("public").trim()
                            .trim_start_matches("protected").trim()
                            .trim_start_matches("private").trim()
                            .trim_start_matches("virtual").trim();
                        let name = cleaned.split('<').next().unwrap_or("").trim();
                        let name = name.split_whitespace().next().unwrap_or("").trim();
                        if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_alphabetic()) {
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

// ── Plain text builder ──────────────────────────────────────────────

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
/// Works across Java, JS, TS by matching on text content.
fn extract_extends_names(
    def_node: tree_sitter::Node,
    content: &str,
    names: &mut Vec<String>,
) {
    // Recursively find all identifier/type_identifier nodes that appear after
    // an "extends" or "implements" keyword, but before the class body
    let full_text = compress::node_text(content, def_node);
    let first_line = full_text.lines().next().unwrap_or("");

    // Parse "class Name extends Parent" or "class Name extends Parent implements I1, I2"
    // from the first line / signature
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

fn build_plain_text(
    sym: &Symbol,
    doc_comment: &Option<String>,
    full_definition: &str,
    call_sites: &[CallSite],
    dependencies: &[Dependency],
    hierarchy: &Option<Hierarchy>,
) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    let kind_label = sym.kind.tag();
    let location = format!("{}:{}", sym.file, sym.line);
    let display_name = if let Some(ref parent) = sym.parent {
        format!("{}::{}", parent, sym.name)
    } else {
        sym.name.clone()
    };

    let _ = writeln!(out, "# {} [{}] {}", display_name, kind_label, location);
    let _ = writeln!(out);

    // Doc comments
    if let Some(doc) = doc_comment {
        let _ = writeln!(out, "## Documentation");
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", doc);
        let _ = writeln!(out);
    }

    // Hierarchy
    if let Some(h) = hierarchy {
        let _ = writeln!(out, "## Hierarchy");
        let _ = writeln!(out);
        if !h.parents.is_empty() {
            let _ = writeln!(out, "Parents:");
            for p in &h.parents {
                if let Some((ref file, line)) = p.location {
                    let _ = writeln!(out, "- {} ({}:{})", p.name, file, line);
                } else if let Some(ref module) = p.external_module {
                    let _ = writeln!(out, "- {} ({} — external)", p.name, module);
                } else {
                    let _ = writeln!(out, "- {} (external)", p.name);
                }
            }
        }
        if !h.children.is_empty() {
            if !h.parents.is_empty() {
                let _ = writeln!(out);
            }
            let _ = writeln!(out, "Children:");
            for c in &h.children {
                if let Some((ref file, line)) = c.location {
                    let _ = writeln!(out, "- {} ({}:{})", c.name, file, line);
                } else {
                    let _ = writeln!(out, "- {}", c.name);
                }
            }
        }
        let _ = writeln!(out);
    }

    // Full definition
    let _ = writeln!(out, "## Definition");
    let _ = writeln!(out);
    let lang_hint = match compress::detect_lang(&sym.file) {
        Some(Lang::Rust) => "rust",
        Some(Lang::Python) => "python",
        Some(Lang::JavaScript) => "javascript",
        Some(Lang::TypeScript | Lang::Tsx) => "typescript",
        Some(Lang::Go) => "go",
        Some(Lang::C) => "c",
        Some(Lang::Cpp) => "cpp",
        Some(Lang::Java) => "java",
        None => "",
    };
    let _ = writeln!(out, "```{}", lang_hint);
    let _ = writeln!(out, "{}", full_definition);
    let _ = writeln!(out, "```");
    let _ = writeln!(out);

    // Call sites
    if !call_sites.is_empty() {
        let _ = writeln!(out, "## Call Sites ({} references)", call_sites.len());
        let _ = writeln!(out);
        for site in call_sites {
            let caller_info = site
                .caller
                .as_ref()
                .map(|c| format!(" in {}", c))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "- {}:{}{} — `{}`",
                site.file, site.line, caller_info, site.context
            );
        }
        let _ = writeln!(out);
    }

    // Dependencies
    if !dependencies.is_empty() {
        let _ = writeln!(out, "## Dependencies ({} symbols)", dependencies.len());
        let _ = writeln!(out);
        for dep in dependencies {
            let kind_tag = dep.kind.map(|k| k.tag()).unwrap_or("--");
            let loc = if let Some((ref file, line)) = dep.location {
                format!("{}:{}", file, line)
            } else if let Some(ref module) = dep.import_from {
                format!("{} (external)", module)
            } else {
                "unknown".to_string()
            };
            let _ = writeln!(out, "- [{}] {} ({})", kind_tag, dep.name, loc);
        }
        let _ = writeln!(out);
    }

    out
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolKind;
    use std::fs;
    use tempfile::TempDir;

    fn setup(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, content).unwrap();
        }
        dir
    }

    fn run(dir: &TempDir, query: &[&str]) -> WhyResult {
        let query: Vec<String> = query.iter().map(|s| s.to_string()).collect();
        explain(dir.path().to_str().unwrap(), &query).unwrap()
    }

    fn run_err(dir: &TempDir, query: &[&str]) -> String {
        let query: Vec<String> = query.iter().map(|s| s.to_string()).collect();
        match explain(dir.path().to_str().unwrap(), &query) {
            Ok(_) => panic!("expected error"),
            Err(e) => e.to_string(),
        }
    }

    // ── Rust ────────────────────────────────────────────────────

    fn rust_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "src/lib.rs",
                r#"use crate::helper::Config;

/// Processes data with the given configuration.
///
/// Returns the processed result as a string.
pub fn process_data(config: &Config) -> String {
    let value = config.get_value();
    format!("processed: {}", value)
}

pub struct DataStore {
    items: Vec<String>,
}

impl DataStore {
    pub fn new() -> Self {
        DataStore { items: Vec::new() }
    }

    pub fn add(&mut self, item: String) {
        self.items.push(item);
    }
}

pub const MAX_ITEMS: usize = 100;

pub enum Status {
    Active,
    Inactive,
}

pub type ItemList = Vec<String>;

fn main() {
    let cfg = Config::default();
    let result = process_data(&cfg);
    println!("{}", result);
}
"#,
            ),
            (
                "src/helper.rs",
                r#"/// Configuration for data processing.
pub struct Config {
    pub name: String,
}

impl Config {
    pub fn default() -> Self {
        Config { name: "default".to_string() }
    }

    pub fn get_value(&self) -> &str {
        &self.name
    }
}
"#,
            ),
        ]
    }

    #[test]
    fn rust_doc_comment() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["process_data"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Processes data"), "doc={doc}");
        assert!(doc.contains("Returns the processed result"), "doc={doc}");
    }

    #[test]
    fn rust_no_doc_comment() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["DataStore"]);
        assert!(r.doc_comment.is_none());
    }

    #[test]
    fn rust_full_def_function() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["process_data"]);
        assert!(r.full_definition.contains("config.get_value()"));
        assert!(r.full_definition.contains("format!"));
    }

    #[test]
    fn rust_full_def_struct() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["DataStore"]);
        assert!(r.full_definition.contains("items: Vec<String>"));
    }

    #[test]
    fn rust_full_def_enum() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["Status"]);
        assert!(r.full_definition.contains("Active"));
        assert!(r.full_definition.contains("Inactive"));
    }

    #[test]
    fn rust_full_def_type_alias() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["ItemList"]);
        assert!(r.full_definition.contains("Vec<String>"));
    }

    #[test]
    fn rust_full_def_const() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["MAX_ITEMS"]);
        assert!(r.full_definition.contains("100"));
    }

    #[test]
    fn rust_call_sites_cross_file() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["Config"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"src/lib.rs"), "call sites={files:?}");
    }

    #[test]
    fn rust_call_sites_same_file_outside_def() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["process_data"]);
        // Should find usage in main()
        let in_main: Vec<_> = r
            .call_sites
            .iter()
            .filter(|s| s.caller.as_deref() == Some("main"))
            .collect();
        assert!(!in_main.is_empty(), "should find call in main()");
    }

    #[test]
    fn rust_deps_body_and_signature() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["process_data"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"Config"), "deps={dep_names:?}");
    }

    #[test]
    fn rust_imports_tracked() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["process_data"]);
        let config_dep = r.dependencies.iter().find(|d| d.name == "Config");
        assert!(config_dep.is_some(), "should have Config dep");
        // Config import should be tracked from the use statement
        let dep = config_dep.unwrap();
        assert!(dep.import_from.as_deref() == Some("crate::helper"), "import_from={:?}", dep.import_from);
    }

    // ── Python ──────────────────────────────────────────────────

    fn python_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "app/models.py",
                r#"from dataclasses import dataclass

class BaseModel:
    """Base model with common functionality.

    Provides serialization and validation.
    """
    def validate(self):
        return True

class User(BaseModel):
    """A user in the system."""
    def __init__(self, name, email):
        self.name = name
        self.email = email

    def greet(self):
        """Return a greeting string."""
        return f"Hello, {self.name}"

MAX_USERS = 1000
"#,
            ),
            (
                "app/service.py",
                r#"from app.models import User, MAX_USERS

def create_user(name, email):
    """Create a new user after validation."""
    if get_count() >= MAX_USERS:
        raise ValueError("Too many users")
    user = User(name, email)
    user.validate()
    return user

def get_count():
    return 0
"#,
            ),
        ]
    }

    #[test]
    fn python_doc_comment_class() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["BaseModel"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Base model with common functionality"), "doc={doc}");
        assert!(doc.contains("serialization and validation"), "doc={doc}");
    }

    #[test]
    fn python_doc_comment_method() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["greet"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Return a greeting string"), "doc={doc}");
    }

    #[test]
    fn python_doc_comment_function() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["create_user"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Create a new user"), "doc={doc}");
    }

    #[test]
    fn python_full_def_class() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["User"]);
        assert!(r.full_definition.contains("__init__"));
        assert!(r.full_definition.contains("greet"));
    }

    #[test]
    fn python_full_def_const() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["MAX_USERS"]);
        assert!(r.full_definition.contains("1000"));
    }

    #[test]
    fn python_call_sites_cross_file() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["User"]);
        let sites: Vec<_> = r
            .call_sites
            .iter()
            .filter(|s| s.file == "app/service.py")
            .collect();
        assert!(!sites.is_empty(), "should find usage in service.py");
        assert!(
            sites.iter().any(|s| s.caller.as_deref() == Some("create_user")),
            "caller should be create_user"
        );
    }

    #[test]
    fn python_deps_resolved() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["create_user"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"User"), "deps={dep_names:?}");
        assert!(dep_names.contains(&"MAX_USERS"), "deps={dep_names:?}");
        assert!(dep_names.contains(&"get_count"), "deps={dep_names:?}");
    }

    #[test]
    fn python_deps_import_tracked() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["create_user"]);
        let user_dep = r.dependencies.iter().find(|d| d.name == "User").unwrap();
        assert_eq!(user_dep.import_from.as_deref(), Some("app.models"));
    }

    #[test]
    fn python_hierarchy_parents() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["User"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let parent_names: Vec<&str> = h.parents.iter().map(|p| p.name.as_str()).collect();
        assert!(parent_names.contains(&"BaseModel"), "parents={parent_names:?}");
    }

    #[test]
    fn python_hierarchy_children() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["BaseModel"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let child_names: Vec<&str> = h.children.iter().map(|c| c.name.as_str()).collect();
        assert!(child_names.contains(&"User"), "children={child_names:?}");
    }

    #[test]
    fn python_hierarchy_external_parent() {
        let dir = setup(&[(
            "ext/child.py",
            r#"from pydantic import BaseModel

class MyModel(BaseModel):
    """A pydantic model."""
    name: str
"#,
        )]);
        let r = run(&dir, &["MyModel"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        assert_eq!(h.parents.len(), 1);
        assert_eq!(h.parents[0].name, "BaseModel");
        assert_eq!(
            h.parents[0].external_module.as_deref(),
            Some("pydantic"),
            "should tag external parent module"
        );
        assert!(h.parents[0].location.is_none(), "external parent has no project location");
    }

    #[test]
    fn python_hierarchy_none_for_function() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["create_user"]);
        assert!(r.hierarchy.is_none());
    }

    // ── TypeScript ──────────────────────────────────────────────

    fn ts_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "src/types.ts",
                r#"/** Represents a configuration option. */
export interface AppConfig {
    name: string;
    debug: boolean;
}

/** Base service with logging. */
export class BaseService {
    protected log(msg: string): void {
        console.log(msg);
    }
}

export type StatusCode = number;

export const DEFAULT_PORT = 3000;
"#,
            ),
            (
                "src/app.ts",
                r#"import { AppConfig, BaseService, DEFAULT_PORT } from './types';

/** Application service that handles requests. */
export class AppService extends BaseService {
    private config: AppConfig;

    constructor(config: AppConfig) {
        super();
        this.config = config;
    }

    /** Start the application server. */
    start(): void {
        this.log(`Starting on port ${DEFAULT_PORT}`);
    }
}

export function createApp(config: AppConfig): AppService {
    return new AppService(config);
}
"#,
            ),
        ]
    }

    #[test]
    fn ts_doc_comment_class() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["AppService"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Application service"), "doc={doc}");
    }

    #[test]
    fn ts_full_def_interface() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["AppConfig"]);
        assert!(r.full_definition.contains("name: string"), "def={}", r.full_definition);
        assert!(r.full_definition.contains("debug: boolean"), "def={}", r.full_definition);
    }

    #[test]
    fn ts_full_def_class() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["AppService"]);
        assert!(r.full_definition.contains("constructor"), "def={}", r.full_definition);
        assert!(r.full_definition.contains("start()"), "def={}", r.full_definition);
    }

    #[test]
    fn ts_full_def_type_alias() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["StatusCode"]);
        assert!(r.full_definition.contains("number"), "def={}", r.full_definition);
    }

    #[test]
    fn ts_call_sites_cross_file() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["AppConfig"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"src/app.ts"), "call_sites files={files:?}");
    }

    #[test]
    fn ts_deps_resolved() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["createApp"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"AppConfig"), "deps={dep_names:?}");
        assert!(dep_names.contains(&"AppService"), "deps={dep_names:?}");
    }

    #[test]
    fn ts_deps_import_tracked() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["createApp"]);
        let cfg_dep = r.dependencies.iter().find(|d| d.name == "AppConfig").unwrap();
        assert_eq!(cfg_dep.import_from.as_deref(), Some("./types"));
    }

    #[test]
    fn ts_hierarchy_parents() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["AppService"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let parent_names: Vec<&str> = h.parents.iter().map(|p| p.name.as_str()).collect();
        assert!(parent_names.contains(&"BaseService"), "parents={parent_names:?}");
    }

    #[test]
    fn ts_hierarchy_children() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["BaseService"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let child_names: Vec<&str> = h.children.iter().map(|c| c.name.as_str()).collect();
        assert!(child_names.contains(&"AppService"), "children={child_names:?}");
    }

    // ── TSX ─────────────────────────────────────────────────────

    fn tsx_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "components/Button.tsx",
                r#"import React from 'react';

/** A reusable button component. */
export class Button extends React.Component {
    render() {
        return <button>{this.props.label}</button>;
    }
}

export function IconButton(props: { icon: string }) {
    return <button>{props.icon}</button>;
}
"#,
            ),
            (
                "components/App.tsx",
                r#"import { Button, IconButton } from './Button';

export function App() {
    return (
        <div>
            <Button label="Click" />
            <IconButton icon="star" />
        </div>
    );
}
"#,
            ),
        ]
    }

    #[test]
    fn tsx_doc_comment() {
        let dir = setup(&tsx_fixtures());
        let r = run(&dir, &["Button"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("reusable button component"), "doc={doc}");
    }

    #[test]
    fn tsx_call_sites_cross_file() {
        let dir = setup(&tsx_fixtures());
        let r = run(&dir, &["Button"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"components/App.tsx"), "call_sites={files:?}");
    }

    #[test]
    fn tsx_full_def_class() {
        let dir = setup(&tsx_fixtures());
        let r = run(&dir, &["Button"]);
        assert!(r.full_definition.contains("render()"), "def={}", r.full_definition);
    }

    // ── JavaScript ──────────────────────────────────────────────

    fn js_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "lib/utils.js",
                r#"/** Calculate the sum of two numbers. */
function calculate(a, b) {
    return a + b;
}

class EventEmitter {
    constructor() {
        this.listeners = {};
    }

    /** Register an event listener. */
    on(event, callback) {
        this.listeners[event] = callback;
    }
}

module.exports = { calculate, EventEmitter };
"#,
            ),
            (
                "lib/main.js",
                r#"const { calculate, EventEmitter } = require('./utils');

function run() {
    const result = calculate(1, 2);
    const emitter = new EventEmitter();
    emitter.on('data', console.log);
    return result;
}
"#,
            ),
        ]
    }

    #[test]
    fn js_doc_comment() {
        let dir = setup(&js_fixtures());
        let r = run(&dir, &["calculate"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Calculate the sum"), "doc={doc}");
    }

    #[test]
    fn js_doc_comment_method() {
        let dir = setup(&js_fixtures());
        let r = run(&dir, &["on"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Register an event listener"), "doc={doc}");
    }

    #[test]
    fn js_full_def_function() {
        let dir = setup(&js_fixtures());
        let r = run(&dir, &["calculate"]);
        assert!(r.full_definition.contains("return a + b"), "def={}", r.full_definition);
    }

    #[test]
    fn js_call_sites_cross_file() {
        let dir = setup(&js_fixtures());
        let r = run(&dir, &["calculate"]);
        let sites: Vec<_> = r
            .call_sites
            .iter()
            .filter(|s| s.file == "lib/main.js")
            .collect();
        assert!(!sites.is_empty(), "should find call in main.js");
        assert!(
            sites.iter().any(|s| s.caller.as_deref() == Some("run")),
            "caller should be run"
        );
    }

    // ── Go ──────────────────────────────────────────────────────

    fn go_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "pkg/server.go",
                r#"package pkg

// Server handles HTTP requests.
// It supports graceful shutdown.
type Server struct {
    Port    int
    Handler Handler
}

// NewServer creates a new Server with the given port.
func NewServer(port int) *Server {
    return &Server{Port: port}
}

// Start begins listening on the configured port.
func (s *Server) Start() error {
    return nil
}

type Handler interface {
    Handle(req string) string
}

const DefaultPort = 8080
"#,
            ),
            (
                "pkg/handler.go",
                r#"package pkg

// LogHandler logs and handles requests.
type LogHandler struct {
    Prefix string
}

// Handle processes the request.
func (h *LogHandler) Handle(req string) string {
    return h.Prefix + req
}

func UseServer() {
    srv := NewServer(DefaultPort)
    srv.Start()
}
"#,
            ),
        ]
    }

    #[test]
    fn go_doc_comment() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["Server"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Server handles HTTP requests"), "doc={doc}");
        assert!(doc.contains("graceful shutdown"), "doc={doc}");
    }

    #[test]
    fn go_full_def_struct() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["Server"]);
        assert!(r.full_definition.contains("Port"), "def={}", r.full_definition);
        assert!(r.full_definition.contains("Handler"), "def={}", r.full_definition);
    }

    #[test]
    fn go_full_def_function() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["NewServer"]);
        assert!(r.full_definition.contains("return &Server"), "def={}", r.full_definition);
    }

    #[test]
    fn go_full_def_interface() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["Handler"]);
        assert!(r.full_definition.contains("Handle"), "def={}", r.full_definition);
    }

    #[test]
    fn go_call_sites_cross_file() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["NewServer"]);
        let sites: Vec<_> = r
            .call_sites
            .iter()
            .filter(|s| s.file == "pkg/handler.go")
            .collect();
        assert!(!sites.is_empty(), "should find call in handler.go");
        assert!(
            sites.iter().any(|s| s.caller.as_deref() == Some("UseServer")),
            "caller should be UseServer, got {:?}",
            sites.iter().map(|s| &s.caller).collect::<Vec<_>>()
        );
    }

    #[test]
    fn go_deps_resolved() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["NewServer"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"Server"), "deps={dep_names:?}");
    }

    // ── Java ────────────────────────────────────────────────────

    fn java_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "src/Animal.java",
                r#"/**
 * Base class for all animals.
 * Provides common animal behavior.
 */
public class Animal {
    protected String name;

    public Animal(String name) {
        this.name = name;
    }

    /** Get the animal's name. */
    public String getName() {
        return name;
    }
}
"#,
            ),
            (
                "src/Dog.java",
                r#"/**
 * A dog that extends Animal.
 */
public class Dog extends Animal {
    private String breed;

    public Dog(String name, String breed) {
        super(name);
        this.breed = breed;
    }

    /** Make the dog bark. */
    public String bark() {
        return getName() + " says Woof!";
    }
}
"#,
            ),
        ]
    }

    #[test]
    fn java_doc_comment() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["Animal"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Base class for all animals"), "doc={doc}");
    }

    #[test]
    fn java_doc_comment_method() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["bark"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Make the dog bark"), "doc={doc}");
    }

    #[test]
    fn java_full_def_class() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["Dog"]);
        assert!(r.full_definition.contains("breed"), "def={}", r.full_definition);
        assert!(r.full_definition.contains("bark"), "def={}", r.full_definition);
    }

    #[test]
    fn java_call_sites_cross_file() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["getName"]);
        let sites: Vec<_> = r
            .call_sites
            .iter()
            .filter(|s| s.file == "src/Dog.java")
            .collect();
        assert!(!sites.is_empty(), "should find call in Dog.java");
        assert!(
            sites.iter().any(|s| s.caller.as_deref() == Some("bark")),
            "caller should be bark"
        );
    }

    #[test]
    fn java_hierarchy_parents() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["Dog"]);
        let h = r.hierarchy.as_ref().expect("Dog should have hierarchy");
        let parent_names: Vec<&str> = h.parents.iter().map(|p| p.name.as_str()).collect();
        assert!(parent_names.contains(&"Animal"), "parents={parent_names:?}");
    }

    #[test]
    fn java_hierarchy_children() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["Animal"]);
        let h = r.hierarchy.as_ref().expect("Animal should have hierarchy");
        let child_names: Vec<&str> = h.children.iter().map(|c| c.name.as_str()).collect();
        assert!(child_names.contains(&"Dog"), "children={child_names:?}");
    }

    #[test]
    fn java_deps_resolved() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["bark"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"getName"), "deps={dep_names:?}");
    }

    // ── JSON ────────────────────────────────────────────────────

    #[test]
    fn json_file_level_symbol() {
        let dir = setup(&[(
            "config.json",
            r#"{
    "name": "my-project",
    "version": "1.0.0",
    "database": {
        "host": "localhost",
        "port": 5432
    }
}"#,
        )]);
        let r = run(&dir, &["config.json"]);
        assert_eq!(r.symbol.kind, SymbolKind::File);
        assert!(r.doc_comment.is_none());
        assert!(r.hierarchy.is_none());
    }

    // ── Markdown ────────────────────────────────────────────────

    #[test]
    fn markdown_file_level_symbol() {
        let dir = setup(&[(
            "docs/README.md",
            "# My Project\n\n## Installation\n\nRun `pip install my-project`.\n\n## Usage\n\nImport and call `process_data`.\n",
        )]);
        let r = run(&dir, &["README.md"]);
        assert_eq!(r.symbol.kind, SymbolKind::File);
        assert!(r.hierarchy.is_none());
    }

    // ── Edge cases ──────────────────────────────────────────────

    #[test]
    fn edge_no_symbol_found() {
        let dir = setup(&[("src/lib.rs", "fn main() {}")]);
        let err = run_err(&dir, &["nonexistent_symbol_xyz"]);
        assert!(err.contains("no symbol found"), "err={err}");
    }

    #[test]
    fn edge_short_name_call_sites_empty() {
        // Symbols with name length <= 2 should skip call site search
        let dir = setup(&[(
            "lib.rs",
            "pub fn go() { }\nfn main() { go(); }\n",
        )]);
        let r = run(&dir, &["go"]);
        assert!(r.call_sites.is_empty(), "short names skip call site search");
    }

    // ── Import parsing unit tests ───────────────────────────────

    #[test]
    fn imports_python_from() {
        let imports = extract_python_imports("from app.models import User, Config\nimport os\n");
        assert_eq!(imports.get("User").map(String::as_str), Some("app.models"));
        assert_eq!(imports.get("Config").map(String::as_str), Some("app.models"));
        assert_eq!(imports.get("os").map(String::as_str), Some("os"));
    }

    #[test]
    fn imports_python_relative() {
        let imports = extract_python_imports("from .main_prompt import build\n");
        assert_eq!(imports.get("build").map(String::as_str), Some(".main_prompt"));
    }

    #[test]
    fn imports_python_as_alias() {
        let imports = extract_python_imports("from numpy import array as arr\n");
        assert_eq!(imports.get("array").map(String::as_str), Some("numpy"));
    }

    #[test]
    fn imports_rust_use() {
        let imports = extract_rust_imports("use anyhow::Result;\nuse std::collections::{HashMap, HashSet};\n");
        assert_eq!(imports.get("Result").map(String::as_str), Some("anyhow"));
        assert_eq!(imports.get("HashMap").map(String::as_str), Some("std::collections"));
        assert_eq!(imports.get("HashSet").map(String::as_str), Some("std::collections"));
    }

    #[test]
    fn imports_js_named() {
        let imports = extract_js_imports("import { AppConfig, BaseService } from './types';\n");
        assert_eq!(imports.get("AppConfig").map(String::as_str), Some("./types"));
        assert_eq!(imports.get("BaseService").map(String::as_str), Some("./types"));
    }

    #[test]
    fn imports_js_default() {
        let imports = extract_js_imports("import React from 'react';\n");
        assert_eq!(imports.get("React").map(String::as_str), Some("react"));
    }

    // ── Docstring edge cases ────────────────────────────────────

    #[test]
    fn python_single_line_docstring() {
        let dir = setup(&[(
            "mod.py",
            "def hello():\n    \"\"\"Say hello.\"\"\"\n    return 'hi'\n",
        )]);
        let r = run(&dir, &["hello"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert_eq!(doc, "Say hello.");
    }

    #[test]
    fn python_triple_single_quote_docstring() {
        let dir = setup(&[(
            "mod.py",
            "def hello():\n    '''Say hello.'''\n    return 'hi'\n",
        )]);
        let r = run(&dir, &["hello"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert_eq!(doc, "Say hello.");
    }

    // ── TSX component-aware tests ──────────────────────────────

    #[test]
    fn tsx_arrow_component_indexed() {
        let dir = setup(&[(
            "Button.tsx",
            r#"import { useState } from 'react';
interface ButtonProps { label: string; onClick: () => void; }
const Button = ({ label, onClick }: ButtonProps) => {
  const [clicks, setClicks] = useState(0);
  return <button onClick={() => { setClicks(clicks + 1); onClick(); }}>{label}</button>;
};
export default Button;
"#,
        )]);
        let r = run(&dir, &["Button"]);
        assert_eq!(r.symbol.kind, SymbolKind::Function);
        assert!(r.full_definition.contains("ButtonProps"));
    }

    #[test]
    fn tsx_props_interface_dep() {
        let dir = setup(&[
            (
                "types.tsx",
                "export interface CardProps { title: string; count: number; }\n",
            ),
            (
                "Card.tsx",
                r#"import { CardProps } from './types';
const Card = ({ title, count }: CardProps) => {
  return <div>{title}: {count}</div>;
};
export default Card;
"#,
            ),
        ]);
        let r = run(&dir, &["Card"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"CardProps"), "deps={dep_names:?}");
    }

    #[test]
    fn tsx_jsx_element_dep() {
        let dir = setup(&[
            (
                "Button.tsx",
                "export function Button({ label }: { label: string }) {\n  return <button>{label}</button>;\n}\n",
            ),
            (
                "App.tsx",
                r#"import { Button } from './Button';
export function App() {
  return <div><Button label="Click" /></div>;
}
"#,
            ),
        ]);
        let r = run(&dir, &["App"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"Button"), "deps={dep_names:?}");
    }

    #[test]
    fn tsx_custom_hook_dep() {
        let dir = setup(&[
            (
                "hooks.tsx",
                "import { useState } from 'react';\nexport function useAuth() {\n  const [user, setUser] = useState(null);\n  return user;\n}\n",
            ),
            (
                "App.tsx",
                r#"import { useAuth } from './hooks';
export function App() {
  const user = useAuth();
  return <div>{user}</div>;
}
"#,
            ),
        ]);
        let r = run(&dir, &["App"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"useAuth"), "deps={dep_names:?}");
    }

    #[test]
    fn tsx_builtin_hook_external() {
        let dir = setup(&[(
            "Counter.tsx",
            r#"import { useState, useEffect } from 'react';
export function Counter() {
  const [count, setCount] = useState(0);
  useEffect(() => { document.title = String(count); }, [count]);
  return <button onClick={() => setCount(count + 1)}>{count}</button>;
}
"#,
        )]);
        let r = run(&dir, &["Counter"]);
        let external: Vec<&str> = r
            .dependencies
            .iter()
            .filter(|d| d.import_from.as_deref() == Some("react"))
            .map(|d| d.name.as_str())
            .collect();
        assert!(external.contains(&"useState"), "react deps={external:?}");
        assert!(external.contains(&"useEffect"), "react deps={external:?}");
    }

    #[test]
    fn tsx_call_sites_jsx_usage() {
        let dir = setup(&[
            (
                "Card.tsx",
                "export function Card() { return <div>card</div>; }\n",
            ),
            (
                "Page.tsx",
                "import { Card } from './Card';\nexport function Page() { return <Card />; }\n",
            ),
        ]);
        let r = run(&dir, &["Card"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"Page.tsx"), "call_sites={files:?}");
    }

    // ── C tests ────────────────────────────────────────────────

    #[test]
    fn c_function_def_found() {
        let dir = setup(&[(
            "math.c",
            "int add(int a, int b) {\n    return a + b;\n}\n",
        )]);
        let r = run(&dir, &["add"]);
        assert_eq!(r.symbol.kind, SymbolKind::Function);
        assert!(r.full_definition.contains("return a + b"));
    }

    #[test]
    fn c_doc_comment() {
        let dir = setup(&[(
            "math.c",
            "/** Add two integers. */\nint add(int a, int b) {\n    return a + b;\n}\n",
        )]);
        let r = run(&dir, &["add"]);
        assert!(r.doc_comment.is_some());
        assert!(r.doc_comment.as_deref().unwrap().contains("Add two integers"));
    }

    #[test]
    fn c_include_local_resolved() {
        let dir = setup(&[
            (
                "types.h",
                "typedef struct { double x; double y; } Point;\ndouble distance(const Point *a, const Point *b);\n",
            ),
            (
                "math.c",
                "#include \"types.h\"\n#include <math.h>\ndouble distance(const Point *a, const Point *b) {\n    double dx = a->x - b->x;\n    return dx;\n}\n",
            ),
        ]);
        let r = run(&dir, &["distance"]);
        assert_eq!(r.symbol.kind, SymbolKind::Function);
        // Point should be a dep (from header include)
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"Point"), "deps={dep_names:?}");
    }

    #[test]
    fn c_call_sites_across_files() {
        let dir = setup(&[
            ("util.c", "int square(int x) { return x * x; }\n"),
            ("main.c", "int square(int x);\nint main() { return square(5); }\n"),
        ]);
        let r = run(&dir, &["square"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"main.c"), "call_sites={files:?}");
    }

    #[test]
    fn c_struct_in_header() {
        let dir = setup(&[(
            "types.h",
            "#ifndef TYPES_H\n#define TYPES_H\ntypedef struct {\n    int x;\n    int y;\n} Vec2;\n#endif\n",
        )]);
        // Vec2 is on the typedef, not the struct itself — currently indexed as type_definition
        // This just verifies we don't crash on header-only files
        let query: Vec<String> = vec!["Vec2".to_string()];
        let result = explain(dir.path().to_str().unwrap(), &query);
        // May or may not find it depending on indexing of typedef — just ensure no panic
        let _ = result;
    }

    // ── C++ tests ──────────────────────────────────────────────

    #[test]
    fn cpp_class_hierarchy_parents() {
        let dir = setup(&[
            (
                "base.hpp",
                "class Base {\npublic:\n    virtual void run() = 0;\n};\n",
            ),
            (
                "derived.hpp",
                "#include \"base.hpp\"\nclass Derived : public Base {\npublic:\n    void run() override;\n};\n",
            ),
        ]);
        let r = run(&dir, &["Derived"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let parent_names: Vec<&str> = h.parents.iter().map(|p| p.name.as_str()).collect();
        assert!(parent_names.contains(&"Base"), "parents={parent_names:?}");
    }

    #[test]
    fn cpp_class_hierarchy_children() {
        let dir = setup(&[
            (
                "base.hpp",
                "class Animal {\npublic:\n    virtual void speak() = 0;\n};\n",
            ),
            (
                "dog.hpp",
                "class Dog : public Animal {\npublic:\n    void speak() override;\n};\n",
            ),
            (
                "cat.hpp",
                "class Cat : public Animal {\npublic:\n    void speak() override;\n};\n",
            ),
        ]);
        let r = run(&dir, &["Animal"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let child_names: Vec<&str> = h.children.iter().map(|c| c.name.as_str()).collect();
        assert!(child_names.contains(&"Dog"), "children={child_names:?}");
        assert!(child_names.contains(&"Cat"), "children={child_names:?}");
    }

    #[test]
    fn cpp_scope_qualifier_method() {
        let dir = setup(&[
            (
                "widget.hpp",
                "class Widget {\npublic:\n    void draw();\n    int width();\n};\n",
            ),
            (
                "widget.cpp",
                "#include \"widget.hpp\"\nvoid Widget::draw() {\n    // render\n}\nint Widget::width() {\n    return 100;\n}\n",
            ),
        ]);
        let r = run(&dir, &["draw"]);
        assert_eq!(r.symbol.parent.as_deref(), Some("Widget"));
        assert_eq!(r.symbol.kind, SymbolKind::Method);
    }

    #[test]
    fn cpp_include_resolved_deps() {
        let dir = setup(&[
            (
                "vec.hpp",
                "struct Vec3 {\n    double x, y, z;\n};\ndouble length(const Vec3& v);\n",
            ),
            (
                "math.cpp",
                "#include \"vec.hpp\"\n#include <cmath>\ndouble length(const Vec3& v) {\n    return sqrt(v.x*v.x + v.y*v.y + v.z*v.z);\n}\n",
            ),
        ]);
        let r = run(&dir, &["length"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"Vec3"), "deps={dep_names:?}");
    }

    #[test]
    fn cpp_call_sites_cross_file() {
        let dir = setup(&[
            (
                "engine.hpp",
                "class Engine {\npublic:\n    void start();\n};\n",
            ),
            (
                "engine.cpp",
                "#include \"engine.hpp\"\nvoid Engine::start() {}\n",
            ),
            (
                "main.cpp",
                "#include \"engine.hpp\"\nint main() {\n    Engine e;\n    e.start();\n    return 0;\n}\n",
            ),
        ]);
        let r = run(&dir, &["start"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"main.cpp"), "call_sites={files:?}");
    }

    #[test]
    fn cpp_multiple_inheritance() {
        let dir = setup(&[
            ("a.hpp", "class Drawable {\npublic:\n    virtual void draw() = 0;\n};\n"),
            ("b.hpp", "class Clickable {\npublic:\n    virtual void click() = 0;\n};\n"),
            (
                "button.hpp",
                "#include \"a.hpp\"\n#include \"b.hpp\"\nclass Button : public Drawable, public Clickable {\npublic:\n    void draw() override;\n    void click() override;\n};\n",
            ),
        ]);
        let r = run(&dir, &["Button"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let parent_names: Vec<&str> = h.parents.iter().map(|p| p.name.as_str()).collect();
        assert!(parent_names.contains(&"Drawable"), "parents={parent_names:?}");
        assert!(parent_names.contains(&"Clickable"), "parents={parent_names:?}");
    }

    // ── C/C++ include import tests ─────────────────────────────

    #[test]
    fn imports_c_local_include() {
        let dir = setup(&[
            ("types.h", "typedef struct { int x; } Point;\n"),
            ("main.c", "#include \"types.h\"\nint main() { return 0; }\n"),
        ]);
        let content = std::fs::read_to_string(dir.path().join("main.c")).unwrap();
        eprintln!("content={content:?}");
        eprintln!("root={:?}", dir.path());
        eprintln!("types.h exists={}", dir.path().join("types.h").exists());
        let header_content = std::fs::read_to_string(dir.path().join("types.h")).unwrap();
        eprintln!("header_content={header_content:?}");
        let syms = scan_header_symbols(&header_content, "types.h");
        eprintln!("header_syms={syms:?}");
        let imports = extract_file_imports(&content, "main.c", dir.path());
        eprintln!("imports={imports:?}");
        assert!(imports.contains_key("Point"), "imports={imports:?}");
    }

    #[test]
    fn imports_c_system_include() {
        let dir = setup(&[(
            "main.c",
            "#include <stdio.h>\n#include <stdlib.h>\nint main() { return 0; }\n",
        )]);
        let content = std::fs::read_to_string(dir.path().join("main.c")).unwrap();
        let imports = extract_file_imports(&content, "main.c", dir.path());
        assert!(imports.contains_key("stdio.h"), "imports={imports:?}");
        assert!(imports.contains_key("stdlib.h"), "imports={imports:?}");
    }

    #[test]
    fn imports_cpp_include_subdir() {
        let dir = setup(&[
            ("include/vec.hpp", "struct Vec2 { double x, y; };\n"),
            ("src/main.cpp", "#include \"../include/vec.hpp\"\nint main() { return 0; }\n"),
        ]);
        let content = std::fs::read_to_string(dir.path().join("src/main.cpp")).unwrap();
        let imports = extract_file_imports(&content, "src/main.cpp", dir.path());
        assert!(imports.contains_key("Vec2"), "imports={imports:?}");
    }
}
