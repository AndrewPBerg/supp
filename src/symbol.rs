use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::compress::{self, Lang};

// ── Data model ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Class,
    Interface,
    Method,
    Type,
    Const,
    Macro,
    File,
}

impl SymbolKind {
    pub fn tag(self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Struct => "st",
            SymbolKind::Enum => "en",
            SymbolKind::Trait => "tr",
            SymbolKind::Class => "cl",
            SymbolKind::Interface => "if",
            SymbolKind::Method => "me",
            SymbolKind::Type => "ty",
            SymbolKind::Const => "co",
            SymbolKind::Macro => "ma",
            SymbolKind::File => "fi",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file: String,
    pub line: usize,
    pub signature: String,
    pub parent: Option<String>,
    /// Extra searchable keywords (used for file-level symbols to index content words)
    #[serde(default)]
    pub keywords: Vec<String>,
}

pub struct SearchResult {
    // add comments here
    pub matches: Vec<(Symbol, f64)>,
    pub total_symbols: usize,
}

// ── Cache ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct Cache {
    symbols: Vec<Symbol>,
    ranks: Vec<f64>,
    file_meta: HashMap<PathBuf, (u64, u64)>, // mtime_secs, size
}

fn cache_path(root: &Path) -> PathBuf {
    // Try .git/supp/sym-cache
    if let Ok(repo) = gix::discover(root) {
        let git_dir = repo.git_dir().to_path_buf();
        return git_dir.join("supp").join("sym-cache");
    }
    // Fallback
    let mut hasher = 0u64;
    for b in root.to_string_lossy().bytes() {
        hasher = hasher.wrapping_mul(31).wrapping_add(b as u64);
    }
    PathBuf::from(format!("/tmp/supp-sym-{:x}", hasher))
}

fn file_meta(path: &Path) -> Option<(u64, u64)> {
    let meta = path.metadata().ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((mtime, meta.len()))
}

fn load_cache(root: &Path) -> Option<Cache> {
    let path = cache_path(root);
    let data = std::fs::read(&path).ok()?;
    bincode::deserialize(&data).ok()
}

fn save_cache(root: &Path, cache: &Cache) {
    let path = cache_path(root);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(data) = bincode::serialize(cache) {
        let _ = std::fs::write(&path, data);
    }
}

// ── Index ───────────────────────────────────────────────────────────

fn collect_files(root: &Path) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    let walker = WalkBuilder::new(root)
        .sort_by_file_name(|a, b| a.cmp(b))
        .build();
    for entry in walker.flatten() {
        let path = entry.path().to_path_buf();
        if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            files.push((path, rel));
        }
    }
    files
}

fn build_index(root: &Path) -> (Vec<Symbol>, Vec<f64>) {
    let files = collect_files(root);

    // Parse files in parallel (rayon thread pool)
    let mut all_results: Vec<(String, Vec<Symbol>, Vec<String>)> = files
        .par_iter()
        .filter_map(|(abs_path, rel_path)| {
            let content = std::fs::read_to_string(abs_path).ok()?;

            if let Some(lang) = compress::detect_lang(rel_path)
                && let Some(tree) = compress::parse_source(&content, lang)
            {
                let symbols = extract_symbols(rel_path, &content, lang, &tree);
                let refs = extract_references(&content, lang, &tree);
                return Some((rel_path.clone(), symbols, refs));
            }

            // Non-code file: create a file-level symbol and extract plain-text refs
            let filename = Path::new(rel_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(rel_path);
            let first_line = content.lines().next().unwrap_or("").to_string();
            let refs = extract_plaintext_refs(&content);
            let sym = Symbol {
                name: filename.to_string(),
                kind: SymbolKind::File,
                file: rel_path.clone(),
                line: 1,
                signature: signature_line(&first_line),
                parent: None,
                keywords: refs.clone(),
            };
            Some((rel_path.clone(), vec![sym], refs))
        })
        .collect();
    all_results.sort_by(|a, b| a.0.cmp(&b.0));

    let mut symbols: Vec<Symbol> = Vec::new();
    let mut file_refs: Vec<Vec<String>> = Vec::new();

    for (_file, file_syms, refs) in all_results {
        symbols.extend(file_syms);
        file_refs.push(refs);
    }

    // Build name → symbol indices map
    let mut name_to_indices: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, sym) in symbols.iter().enumerate() {
        name_to_indices.entry(&sym.name).or_default().push(i);
    }

    // Build edges: for each file's references, link symbols defined in that file to referenced symbols
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); symbols.len()];

    // Group symbols by file for edge building
    let mut file_to_sym_indices: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, sym) in symbols.iter().enumerate() {
        file_to_sym_indices.entry(&sym.file).or_default().push(i);
    }

    // For each file's references, create edges from function/method symbols in that file
    // to the referenced symbols
    let mut sorted_files: Vec<&str> = file_to_sym_indices.keys().copied().collect();
    sorted_files.sort();

    for (file_idx, refs) in file_refs.iter().enumerate() {
        if file_idx >= sorted_files.len() {
            break;
        }
        let file = sorted_files[file_idx];

        let source_indices: Vec<usize> = file_to_sym_indices
            .get(file)
            .map(|v| {
                v.iter()
                    .filter(|&&i| {
                        matches!(symbols[i].kind, SymbolKind::Function | SymbolKind::Method)
                    })
                    .copied()
                    .collect()
            })
            .unwrap_or_default();

        if source_indices.is_empty() {
            continue;
        }

        for ref_name in refs {
            if let Some(target_indices) = name_to_indices.get(ref_name.as_str()) {
                for &src in &source_indices {
                    for &tgt in target_indices {
                        if src != tgt {
                            edges[src].push(tgt);
                        }
                    }
                }
            }
        }
    }

    let ranks = pagerank(&edges, symbols.len(), 15, 0.85);
    (symbols, ranks)
}

fn build_index_incremental(root: &Path, old_cache: &Cache) -> (Vec<Symbol>, Vec<f64>) {
    let files = collect_files(root);

    // Check which files changed
    let mut any_changed = false;
    if files.len() != old_cache.file_meta.len() {
        any_changed = true;
    } else {
        for (abs_path, _) in &files {
            if let Some(meta) = file_meta(abs_path) {
                if let Some(cached_meta) = old_cache.file_meta.get(abs_path) {
                    if meta != *cached_meta {
                        any_changed = true;
                        break;
                    }
                } else {
                    any_changed = true;
                    break;
                }
            }
        }
    }

    if !any_changed {
        return (old_cache.symbols.clone(), old_cache.ranks.clone());
    }

    // Full rebuild if anything changed (incremental per-file is complex and the full build is fast)
    build_index(root)
}

// ── Symbol extraction ───────────────────────────────────────────────

fn extract_symbols(file: &str, content: &str, lang: Lang, tree: &tree_sitter::Tree) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let root = tree.root_node();
    extract_symbols_recursive(file, content, lang, root, &mut symbols, None);
    symbols
}

fn extract_symbols_recursive(
    file: &str,
    content: &str,
    lang: Lang,
    node: tree_sitter::Node,
    symbols: &mut Vec<Symbol>,
    parent: Option<&str>,
) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            extract_node_symbols(file, content, lang, child, symbols, parent);
            // Recurse into preprocessor directives so we index symbols inside #ifdef guards
            if child.kind().starts_with("preproc_") {
                extract_symbols_recursive(file, content, lang, child, symbols, parent);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn extract_node_symbols(
    file: &str,
    content: &str,
    lang: Lang,
    node: tree_sitter::Node,
    symbols: &mut Vec<Symbol>,
    parent: Option<&str>,
) {
    let kind = node.kind();
    let text = compress::node_text(content, node);

    match lang {
        Lang::Rust => extract_rust_symbols(file, content, node, kind, text, symbols, parent),
        Lang::Python => extract_python_symbols(file, content, node, kind, text, symbols, parent),
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
            extract_js_symbols(file, content, node, kind, text, symbols, parent, lang)
        }
        Lang::Go => extract_go_symbols(file, content, node, kind, text, symbols, parent),
        Lang::C | Lang::Cpp => {
            extract_c_symbols(file, content, node, kind, text, symbols, parent, lang)
        }
        Lang::Java => extract_java_symbols(file, content, node, kind, text, symbols, parent),
    }
}

fn signature_line(text: &str) -> String {
    // Take first line, trim, cap at 100 chars
    let line = text.lines().next().unwrap_or(text).trim();
    if line.len() > 100 {
        format!("{}...", &line[..97])
    } else {
        line.to_string()
    }
}

fn name_from_field(node: tree_sitter::Node, content: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|n| compress::node_text(content, n).to_string())
}

// ── Rust symbols ────────────────────────────────────────────────────

fn extract_rust_symbols(
    file: &str,
    content: &str,
    node: tree_sitter::Node,
    kind: &str,
    text: &str,
    symbols: &mut Vec<Symbol>,
    parent: Option<&str>,
) {
    match kind {
        "function_item" => {
            if let Some(name) = name_from_field(node, content) {
                let sk = if parent.is_some() {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                symbols.push(Symbol {
                    name,
                    kind: sk,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: parent.map(String::from),
                    keywords: Vec::new(),
                });
            }
        }
        "struct_item" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name: name.clone(),
                    kind: SymbolKind::Struct,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
            }
        }
        "enum_item" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Enum,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
            }
        }
        "trait_item" => {
            if let Some(name) = name_from_field(node, content) {
                let trait_name = name.clone();
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Trait,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
                // Recurse into trait body for methods
                if let Some(body) = node.child_by_field_name("body") {
                    recurse_children(file, content, Lang::Rust, body, symbols, Some(&trait_name));
                }
            }
        }
        "impl_item" => {
            // Extract the type name being implemented
            let impl_name = extract_impl_type(node, content);
            if let Some(body) = node.child_by_field_name("body") {
                recurse_children(
                    file,
                    content,
                    Lang::Rust,
                    body,
                    symbols,
                    impl_name.as_deref(),
                );
            }
        }
        "type_item" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Type,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
            }
        }
        "const_item" | "static_item" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Const,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
            }
        }
        "macro_definition" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Macro,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
            }
        }
        _ => {}
    }
}

fn extract_impl_type(node: tree_sitter::Node, content: &str) -> Option<String> {
    // impl Type { ... } or impl Trait for Type { ... }
    // The "type" field gives us the target type
    node.child_by_field_name("type")
        .map(|n| compress::node_text(content, n).to_string())
}

// ── Python symbols ──────────────────────────────────────────────────

fn extract_python_symbols(
    file: &str,
    content: &str,
    node: tree_sitter::Node,
    kind: &str,
    text: &str,
    symbols: &mut Vec<Symbol>,
    parent: Option<&str>,
) {
    match kind {
        "function_definition" => {
            if let Some(name) = name_from_field(node, content) {
                let sk = if parent.is_some() {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                symbols.push(Symbol {
                    name,
                    kind: sk,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: parent.map(String::from),
                    keywords: Vec::new(),
                });
            }
        }
        "class_definition" => {
            if let Some(name) = name_from_field(node, content) {
                let class_name = name.clone();
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
                if let Some(body) = node.child_by_field_name("body") {
                    recurse_children(
                        file,
                        content,
                        Lang::Python,
                        body,
                        symbols,
                        Some(&class_name),
                    );
                }
            }
        }
        "decorated_definition" => {
            // Recurse into the actual definition
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() != "decorator" {
                        extract_node_symbols(file, content, Lang::Python, child, symbols, parent);
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        // Module-level assignments: `model = OpenAIResponsesModel(...)` or
        // type-annotated: `main_agent: Agent[...] = Agent(...)`
        "expression_statement" if parent.is_none() => {
            extract_python_assignment(file, content, node, symbols);
        }
        _ => {}
    }
}

fn extract_python_assignment(
    file: &str,
    content: &str,
    node: tree_sitter::Node,
    symbols: &mut Vec<Symbol>,
) {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return;
    }
    loop {
        let child = cursor.node();
        if child.kind() == "assignment"
            && let Some(left) = child.child_by_field_name("left")
            && left.kind() == "identifier"
        {
            let name = compress::node_text(content, left).to_string();
            // Skip dunder names and _private (but keep __all__)
            if name.starts_with("__") && name.ends_with("__") && name != "__all__" {
                return;
            }
            if name.starts_with('_') {
                return;
            }
            // Build signature from the full assignment text
            let sig = signature_line(compress::node_text(content, child));
            symbols.push(Symbol {
                name,
                kind: SymbolKind::Const,
                file: file.to_string(),
                line: node.start_position().row + 1,
                signature: sig,
                parent: None,
                keywords: Vec::new(),
            });
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

// ── JS/TS symbols ───────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn extract_js_symbols(
    file: &str,
    content: &str,
    node: tree_sitter::Node,
    kind: &str,
    text: &str,
    symbols: &mut Vec<Symbol>,
    parent: Option<&str>,
    lang: Lang,
) {
    match kind {
        "function_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Function,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: parent.map(String::from),
                    keywords: Vec::new(),
                });
            }
        }
        "class_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                let class_name = name.clone();
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
                if let Some(body) = node.child_by_field_name("body") {
                    recurse_children(file, content, lang, body, symbols, Some(&class_name));
                }
            }
        }
        "method_definition" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Method,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: parent.map(String::from),
                    keywords: Vec::new(),
                });
            }
        }
        "interface_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Interface,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
            }
        }
        "type_alias_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Type,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
            }
        }
        "enum_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Enum,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            // const MyComponent = (props: Props) => { ... }
            // const MyComponent = function(...) { ... }
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "variable_declarator"
                        && let Some(name) = name_from_field(child, content)
                    {
                        let value = child.child_by_field_name("value");
                        let is_fn = value.is_some_and(|v| {
                            matches!(
                                v.kind(),
                                "arrow_function" | "function" | "function_expression"
                            )
                        });
                        if is_fn {
                            symbols.push(Symbol {
                                name,
                                kind: SymbolKind::Function,
                                file: file.to_string(),
                                line: node.start_position().row + 1,
                                signature: signature_line(text),
                                parent: parent.map(String::from),
                                keywords: Vec::new(),
                            });
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "export_statement" => {
            // Recurse into exported declarations
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    extract_node_symbols(file, content, lang, child, symbols, parent);
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        _ => {}
    }
}

// ── Go symbols ──────────────────────────────────────────────────────

fn extract_go_symbols(
    file: &str,
    content: &str,
    node: tree_sitter::Node,
    kind: &str,
    text: &str,
    symbols: &mut Vec<Symbol>,
    parent: Option<&str>,
) {
    match kind {
        "function_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Function,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: parent.map(String::from),
                    keywords: Vec::new(),
                });
            }
        }
        "method_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                // Try to get receiver type
                let receiver = node.child_by_field_name("receiver").and_then(|r| {
                    // Walk to find the type identifier
                    let mut cursor = r.walk();
                    find_type_identifier(&mut cursor, content)
                });
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Method,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: receiver,
                    keywords: Vec::new(),
                });
            }
        }
        "type_declaration" => {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    if cursor.node().kind() == "type_spec" {
                        let spec = cursor.node();
                        if let Some(name) = name_from_field(spec, content) {
                            // Determine if struct or interface
                            let sk = determine_go_type_kind(spec);
                            symbols.push(Symbol {
                                name,
                                kind: sk,
                                file: file.to_string(),
                                line: spec.start_position().row + 1,
                                signature: signature_line(compress::node_text(content, spec)),
                                parent: None,
                                keywords: Vec::new(),
                            });
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "const_declaration" | "var_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Const,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
            }
        }
        _ => {}
    }
}

fn find_type_identifier(cursor: &mut tree_sitter::TreeCursor, content: &str) -> Option<String> {
    loop {
        let node = cursor.node();
        if node.kind() == "type_identifier" {
            return Some(compress::node_text(content, node).to_string());
        }
        if cursor.goto_first_child() {
            if let Some(result) = find_type_identifier(cursor, content) {
                return Some(result);
            }
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            return None;
        }
    }
}

fn determine_go_type_kind(spec: tree_sitter::Node) -> SymbolKind {
    let mut cursor = spec.walk();
    if cursor.goto_first_child() {
        loop {
            match cursor.node().kind() {
                "struct_type" => return SymbolKind::Struct,
                "interface_type" => return SymbolKind::Interface,
                _ => {}
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    SymbolKind::Type
}

// ── C/C++ symbols ───────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn extract_c_symbols(
    file: &str,
    content: &str,
    node: tree_sitter::Node,
    kind: &str,
    text: &str,
    symbols: &mut Vec<Symbol>,
    parent: Option<&str>,
    lang: Lang,
) {
    match kind {
        "function_definition" => {
            if let Some(declarator) = node.child_by_field_name("declarator")
                && let Some(name) = find_declarator_name(declarator, content)
            {
                // C++: detect Foo::bar() scope qualifier for method resolution
                let scope = find_scope_qualifier(declarator, content);
                let effective_parent = scope.as_deref().or(parent);
                let sk = if effective_parent.is_some() {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                symbols.push(Symbol {
                    name,
                    kind: sk,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: effective_parent.map(String::from),
                    keywords: Vec::new(),
                });
            }
        }
        "struct_specifier" | "enum_specifier" => {
            if let Some(name) = name_from_field(node, content) {
                let sk = if kind == "struct_specifier" {
                    SymbolKind::Struct
                } else {
                    SymbolKind::Enum
                };
                symbols.push(Symbol {
                    name,
                    kind: sk,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
            }
        }
        "class_specifier" if lang == Lang::Cpp => {
            if let Some(name) = name_from_field(node, content) {
                let class_name = name.clone();
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: None,
                    keywords: Vec::new(),
                });
                if let Some(body) = node.child_by_field_name("body") {
                    recurse_children(file, content, lang, body, symbols, Some(&class_name));
                }
            }
        }
        "namespace_definition" if lang == Lang::Cpp => {
            if let Some(body) = node.child_by_field_name("body") {
                recurse_children(file, content, lang, body, symbols, parent);
            }
        }
        _ => {}
    }
}

fn find_declarator_name(node: tree_sitter::Node, content: &str) -> Option<String> {
    // Recursively find the identifier in a declarator
    if node.kind() == "identifier" {
        return Some(compress::node_text(content, node).to_string());
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if let Some(name) = find_declarator_name(cursor.node(), content) {
                return Some(name);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Extract C++ scope qualifier from a declarator (e.g. "Foo" from "Foo::bar").
fn find_scope_qualifier(node: tree_sitter::Node, content: &str) -> Option<String> {
    if node.kind() == "qualified_identifier" {
        // qualified_identifier has scope (type_identifier / namespace_identifier) and name
        if let Some(scope) = node.child_by_field_name("scope") {
            let text = compress::node_text(content, scope);
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    // Recurse into children (e.g. function_declarator wrapping qualified_identifier)
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if let Some(q) = find_scope_qualifier(cursor.node(), content) {
                return Some(q);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

// ── Java symbols ────────────────────────────────────────────────────

fn extract_java_symbols(
    file: &str,
    content: &str,
    node: tree_sitter::Node,
    kind: &str,
    text: &str,
    symbols: &mut Vec<Symbol>,
    parent: Option<&str>,
) {
    match kind {
        "class_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                let class_name = name.clone();
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: parent.map(String::from),
                    keywords: Vec::new(),
                });
                if let Some(body) = node.child_by_field_name("body") {
                    recurse_children(file, content, Lang::Java, body, symbols, Some(&class_name));
                }
            }
        }
        "interface_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Interface,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: parent.map(String::from),
                    keywords: Vec::new(),
                });
            }
        }
        "enum_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Enum,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: parent.map(String::from),
                    keywords: Vec::new(),
                });
            }
        }
        "method_declaration" | "constructor_declaration" => {
            if let Some(name) = name_from_field(node, content) {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Method,
                    file: file.to_string(),
                    line: node.start_position().row + 1,
                    signature: signature_line(text),
                    parent: parent.map(String::from),
                    keywords: Vec::new(),
                });
            }
        }
        _ => {}
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn recurse_children(
    file: &str,
    content: &str,
    lang: Lang,
    body: tree_sitter::Node,
    symbols: &mut Vec<Symbol>,
    parent: Option<&str>,
) {
    let mut cursor = body.walk();
    if cursor.goto_first_child() {
        loop {
            extract_node_symbols(file, content, lang, cursor.node(), symbols, parent);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

// ── Reference extraction ────────────────────────────────────────────

fn extract_references(content: &str, lang: Lang, tree: &tree_sitter::Tree) -> Vec<String> {
    let mut refs = Vec::new();
    let root = tree.root_node();
    collect_identifiers_in_bodies(content, lang, root, &mut refs, false);
    refs.sort();
    refs.dedup();
    refs
}

fn collect_identifiers_in_bodies(
    content: &str,
    lang: Lang,
    node: tree_sitter::Node,
    refs: &mut Vec<String>,
    in_body: bool,
) {
    let kind = node.kind();

    // Check if this node is a function/method body
    let is_body = matches!(
        kind,
        "block" | "statement_block" | "compound_statement" | "expression_statement"
    ) && node
        .parent()
        .is_some_and(|p| is_function_node(p.kind(), lang));

    let inside = in_body || is_body;

    if inside && matches!(kind, "identifier" | "type_identifier") {
        let text = compress::node_text(content, node);
        // Skip very short identifiers and keywords
        if text.len() > 1 && !is_keyword(text, lang) {
            refs.push(text.to_string());
        }
    }

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_identifiers_in_bodies(content, lang, cursor.node(), refs, inside);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Extract word-like tokens from non-code files (markdown, toml, json, etc.)
fn extract_plaintext_refs(content: &str) -> Vec<String> {
    let mut refs: Vec<String> = Vec::new();
    for word in content.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        let w = word.trim_matches(|c: char| c == '-' || c == '_');
        if w.len() > 1 {
            refs.push(w.to_string());
        }
    }
    refs.sort();
    refs.dedup();
    refs
}

fn is_function_node(kind: &str, _lang: Lang) -> bool {
    matches!(
        kind,
        "function_item"
            | "function_definition"
            | "function_declaration"
            | "method_declaration"
            | "method_definition"
            | "constructor_declaration"
    )
}

pub fn is_keyword(s: &str, lang: Lang) -> bool {
    match lang {
        Lang::Rust => matches!(
            s,
            "self"
                | "Self"
                | "super"
                | "crate"
                | "let"
                | "mut"
                | "ref"
                | "if"
                | "else"
                | "match"
                | "for"
                | "while"
                | "loop"
                | "return"
                | "break"
                | "continue"
                | "fn"
                | "pub"
                | "struct"
                | "enum"
                | "impl"
                | "trait"
                | "use"
                | "mod"
                | "where"
                | "as"
                | "in"
                | "true"
                | "false"
                | "Some"
                | "None"
                | "Ok"
                | "Err"
        ),
        Lang::Python => matches!(
            s,
            "self"
                | "cls"
                | "if"
                | "else"
                | "elif"
                | "for"
                | "while"
                | "return"
                | "def"
                | "class"
                | "import"
                | "from"
                | "as"
                | "in"
                | "not"
                | "and"
                | "or"
                | "True"
                | "False"
                | "None"
                | "pass"
                | "break"
                | "continue"
                | "with"
        ),
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => matches!(
            s,
            "this"
                | "if"
                | "else"
                | "for"
                | "while"
                | "return"
                | "function"
                | "class"
                | "const"
                | "let"
                | "var"
                | "import"
                | "export"
                | "from"
                | "new"
                | "typeof"
                | "instanceof"
                | "true"
                | "false"
                | "null"
                | "undefined"
                | "async"
                | "await"
        ),
        Lang::Go => matches!(
            s,
            "if" | "else"
                | "for"
                | "return"
                | "func"
                | "type"
                | "struct"
                | "interface"
                | "package"
                | "import"
                | "var"
                | "const"
                | "range"
                | "defer"
                | "go"
                | "true"
                | "false"
                | "nil"
                | "err"
        ),
        Lang::C | Lang::Cpp => matches!(
            s,
            "if" | "else"
                | "for"
                | "while"
                | "return"
                | "int"
                | "void"
                | "char"
                | "float"
                | "double"
                | "struct"
                | "enum"
                | "typedef"
                | "sizeof"
                | "NULL"
                | "true"
                | "false"
                | "this"
                | "class"
                | "public"
                | "private"
        ),
        Lang::Java => matches!(
            s,
            "this"
                | "if"
                | "else"
                | "for"
                | "while"
                | "return"
                | "class"
                | "interface"
                | "new"
                | "public"
                | "private"
                | "protected"
                | "static"
                | "void"
                | "int"
                | "boolean"
                | "true"
                | "false"
                | "null"
                | "final"
                | "abstract"
        ),
    }
}

// ── PageRank ────────────────────────────────────────────────────────

fn pagerank(edges: &[Vec<usize>], n: usize, iterations: usize, damping: f64) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }

    let mut ranks = vec![1.0 / n as f64; n];
    let mut new_ranks = vec![0.0; n];

    // Precompute out-degrees
    let out_degree: Vec<usize> = edges.iter().map(|e| e.len()).collect();

    for _ in 0..iterations {
        new_ranks.fill((1.0 - damping) / n as f64);

        for (i, edges_i) in edges.iter().enumerate() {
            if out_degree[i] > 0 {
                let share = damping * ranks[i] / out_degree[i] as f64;
                for &j in edges_i {
                    new_ranks[j] += share;
                }
            }
        }

        std::mem::swap(&mut ranks, &mut new_ranks);
    }

    ranks
}

// ── Query scoring ───────────────────────────────────────────────────

pub fn split_subwords(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    for ch in name.chars() {
        if ch == '_' || ch == '-' {
            if !current.is_empty() {
                words.push(current.to_lowercase());
                current.clear();
            }
        } else if ch.is_uppercase()
            && !current.is_empty()
            && current.chars().last().is_some_and(|c| c.is_lowercase())
        {
            words.push(current.to_lowercase());
            current.clear();
            current.push(ch);
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(current.to_lowercase());
    }

    words
}

fn score_query(symbols: &[Symbol], ranks: &[f64], query_tokens: &[String]) -> Vec<(usize, f64)> {
    if query_tokens.is_empty() {
        return Vec::new();
    }

    let query_lower: Vec<String> = query_tokens.iter().map(|t| t.to_lowercase()).collect();

    let mut scored: Vec<(usize, f64)> = symbols
        .iter()
        .enumerate()
        .filter_map(|(i, sym)| {
            let name_words = split_subwords(&sym.name);
            let path_components: Vec<String> = sym
                .file
                .split('/')
                .flat_map(|c| c.split('.'))
                .map(|s| s.to_lowercase())
                .collect();
            let parent_words: Vec<String> = sym
                .parent
                .as_deref()
                .map(split_subwords)
                .unwrap_or_default();

            let mut matched_tokens = 0;
            let mut total_text_score = 0.0;

            for qt in &query_lower {
                let mut best = 0.0f64;

                // Name match
                for nw in &name_words {
                    if nw == qt {
                        best = best.max(1.0);
                    } else if nw.starts_with(qt) {
                        best = best.max(0.6);
                    } else if nw.contains(qt.as_str()) {
                        best = best.max(0.3);
                    }
                }

                // Also check full name (lowercase)
                let name_lower = sym.name.to_lowercase();
                if name_lower == *qt {
                    best = best.max(1.0);
                } else if name_lower.starts_with(qt) {
                    best = best.max(0.6);
                } else if name_lower.contains(qt.as_str()) {
                    best = best.max(0.3);
                }

                // Path match
                for pc in &path_components {
                    if pc == qt {
                        best = best.max(0.3);
                    } else if pc.starts_with(qt) {
                        best = best.max(0.2);
                    }
                }

                // Parent match
                for pw in &parent_words {
                    if pw == qt {
                        best = best.max(0.5);
                    } else if pw.starts_with(qt) {
                        best = best.max(0.3);
                    }
                }

                // Keyword match (content words for file-level symbols)
                for kw in &sym.keywords {
                    let kw_lower = kw.to_lowercase();
                    if kw_lower == *qt {
                        best = best.max(0.4);
                    } else if kw_lower.starts_with(qt) {
                        best = best.max(0.25);
                    } else if kw_lower.contains(qt.as_str()) {
                        best = best.max(0.15);
                    }
                }

                if best > 0.0 {
                    matched_tokens += 1;
                    total_text_score += best;
                }
            }

            // Require ALL query tokens to match something
            if matched_tokens != query_lower.len() {
                return None;
            }

            let text_match = total_text_score / query_lower.len() as f64;
            let rank = ranks.get(i).copied().unwrap_or(0.0);
            let rank_score = (1.0 + rank * 1000.0).ln();

            let score = 0.6 * text_match + 0.4 * rank_score;
            Some((i, score))
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

// ── Public API ──────────────────────────────────────────────────────

pub fn search(root: &str, query: &[String]) -> Result<SearchResult> {
    let root_path = std::fs::canonicalize(root)?;

    let (symbols, ranks) = if let Some(cache) = load_cache(&root_path) {
        build_index_incremental(&root_path, &cache)
    } else {
        build_index(&root_path)
    };

    // Save cache
    let files = collect_files(&root_path);
    let file_meta_map: HashMap<PathBuf, (u64, u64)> = files
        .iter()
        .filter_map(|(abs, _)| file_meta(abs).map(|m| (abs.clone(), m)))
        .collect();

    save_cache(
        &root_path,
        &Cache {
            symbols: symbols.clone(),
            ranks: ranks.clone(),
            file_meta: file_meta_map,
        },
    );

    let total_symbols = symbols.len();
    let scored = score_query(&symbols, &ranks, query);
    let matches: Vec<(Symbol, f64)> = scored
        .into_iter()
        .take(20)
        .map(|(i, score)| (symbols[i].clone(), score))
        .collect();

    Ok(SearchResult {
        matches,
        total_symbols,
    })
}

/// Load all indexed symbols (from cache or fresh build). Used by `why` for dependency lookup.
pub fn load_symbols(root: &Path) -> Vec<Symbol> {
    let (symbols, _ranks) = if let Some(cache) = load_cache(root) {
        build_index_incremental(root, &cache)
    } else {
        build_index(root)
    };
    symbols
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_subwords_snake_case() {
        assert_eq!(
            split_subwords("generate_context"),
            vec!["generate", "context"]
        );
    }

    #[test]
    fn split_subwords_camel_case() {
        assert_eq!(split_subwords("ContextResult"), vec!["context", "result"]);
    }

    #[test]
    fn split_subwords_mixed() {
        assert_eq!(split_subwords("myFunc_name"), vec!["my", "func", "name"]);
    }

    #[test]
    fn split_subwords_single() {
        assert_eq!(split_subwords("main"), vec!["main"]);
    }

    #[test]
    fn split_subwords_all_caps() {
        assert_eq!(split_subwords("MAX"), vec!["max"]);
    }

    #[test]
    fn pagerank_simple() {
        // A -> B -> C -> A (cycle)
        let edges = vec![vec![1], vec![2], vec![0]];
        let ranks = pagerank(&edges, 3, 20, 0.85);
        // All should be roughly equal in a cycle
        for r in &ranks {
            assert!((r - 1.0 / 3.0).abs() < 0.01, "rank {} not near 0.33", r);
        }
    }

    #[test]
    fn pagerank_star() {
        // 0, 1, 2 all point to 3
        let edges = vec![vec![3], vec![3], vec![3], vec![]];
        let ranks = pagerank(&edges, 4, 20, 0.85);
        // Node 3 should have highest rank
        assert!(ranks[3] > ranks[0]);
        assert!(ranks[3] > ranks[1]);
        assert!(ranks[3] > ranks[2]);
    }

    #[test]
    fn pagerank_empty() {
        let ranks = pagerank(&[], 0, 10, 0.85);
        assert!(ranks.is_empty());
    }

    #[test]
    fn score_query_basic() {
        let symbols = vec![
            Symbol {
                name: "generate_context".to_string(),
                kind: SymbolKind::Function,
                file: "src/context.rs".to_string(),
                line: 42,
                signature: "pub fn generate_context(...)".to_string(),
                parent: None,
                keywords: Vec::new(),
            },
            Symbol {
                name: "ContextResult".to_string(),
                kind: SymbolKind::Struct,
                file: "src/context.rs".to_string(),
                line: 8,
                signature: "pub struct ContextResult { ... }".to_string(),
                parent: None,
                keywords: Vec::new(),
            },
            Symbol {
                name: "compress".to_string(),
                kind: SymbolKind::Function,
                file: "src/compress.rs".to_string(),
                line: 60,
                signature: "pub fn compress(...)".to_string(),
                parent: None,
                keywords: Vec::new(),
            },
        ];
        let ranks = vec![0.5, 0.3, 0.2];
        let query = vec!["context".to_string()];
        let results = score_query(&symbols, &ranks, &query);

        assert!(!results.is_empty());
        // generate_context and ContextResult should both match
        let matched_indices: Vec<usize> = results.iter().map(|(i, _)| *i).collect();
        assert!(matched_indices.contains(&0));
        assert!(matched_indices.contains(&1));
    }
}
