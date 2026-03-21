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
    let imports = extract_file_imports(&content, &sym.file);

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
            comment_lines.push(trimmed);
        } else if trimmed.starts_with("/**") || trimmed.ends_with("*/") || trimmed.starts_with('*') {
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
fn extract_file_imports(content: &str, file_path: &str) -> HashMap<String, String> {
    let lang = compress::detect_lang(file_path);
    match lang {
        Some(Lang::Python) => extract_python_imports(content),
        Some(Lang::Rust) => extract_rust_imports(content),
        Some(Lang::JavaScript | Lang::TypeScript | Lang::Tsx) => extract_js_imports(content),
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

/// Try to resolve a relative import to a project file path.
fn resolve_relative_import(module: &str, from_file: &str, root: &Path) -> Option<String> {
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

fn find_call_sites(root: &Path, sym: &Symbol) -> Vec<CallSite> {
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

fn contains_identifier(line: &str, name: &str) -> bool {
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

fn find_enclosing_function(
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
            // Check if this class extends our symbol
            // Look for (SymName) or (SymName, ...) or (..., SymName) in signature
            if let Some(paren_start) = s.signature.find('(') {
                let after_paren = &s.signature[paren_start..];
                contains_identifier(after_paren, &sym.name)
            } else {
                false
            }
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
            // tree-sitter-java: superclass field, interfaces field
            let mut names = Vec::new();
            if let Some(super_node) = def_node.child_by_field_name("superclass") {
                let name = compress::node_text(content, super_node);
                names.push(name.to_string());
            }
            // interfaces: super_interfaces node
            if let Some(interfaces) = def_node.child_by_field_name("interfaces") {
                let mut cursor = interfaces.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        if child.kind() == "type_identifier" {
                            names.push(compress::node_text(content, child).to_string());
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            names
        }
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
            // class Foo extends Bar { ... }
            let mut names = Vec::new();
            let mut cursor = def_node.walk();
            if cursor.goto_first_child() {
                let mut saw_extends = false;
                loop {
                    let child = cursor.node();
                    if child.kind() == "extends" || compress::node_text(content, child) == "extends"
                    {
                        saw_extends = true;
                    } else if saw_extends
                        && matches!(child.kind(), "identifier" | "type_identifier")
                    {
                        names.push(compress::node_text(content, child).to_string());
                        saw_extends = false;
                    } else if child.kind() == "class_heritage" {
                        // TS: class_heritage contains the extends clause
                        let mut inner = child.walk();
                        if inner.goto_first_child() {
                            loop {
                                let n = inner.node();
                                if matches!(n.kind(), "identifier" | "type_identifier") {
                                    names.push(compress::node_text(content, n).to_string());
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
            names
        }
        _ => Vec::new(),
    }
}

// ── Plain text builder ──────────────────────────────────────────────

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
