use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use wincode::{SchemaRead, SchemaWrite};

use crate::compress::{self, Lang};

// ── Data model ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SchemaWrite, SchemaRead)]
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

#[derive(Debug, Clone, Serialize, Deserialize, SchemaWrite, SchemaRead)]
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

#[derive(Serialize)]
pub struct SearchResult {
    // add comments here
    pub matches: Vec<(Symbol, f64)>,
    pub total_symbols: usize,
}

// ── Cache ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, SchemaWrite, SchemaRead)]
struct Cache {
    symbols: Vec<Symbol>,
    ranks: Vec<f64>,
    file_meta: HashMap<String, (u64, u64)>, // path string → mtime_secs, size
    file_refs: HashMap<String, Vec<String>>, // rel_path → reference names
}

struct IndexResult {
    symbols: Vec<Symbol>,
    ranks: Vec<f64>,
    file_meta: HashMap<String, (u64, u64)>,
    file_refs: HashMap<String, Vec<String>>,
    changed: bool,
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
    wincode::deserialize(&data).ok()
}

fn save_cache(root: &Path, cache: &Cache) {
    let path = cache_path(root);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(data) = wincode::serialize(cache) {
        let _ = std::fs::write(&path, data);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn parse_file_entry(abs_path: &Path, rel_path: &str) -> Option<(Vec<Symbol>, Vec<String>)> {
    let content = std::fs::read_to_string(abs_path).ok()?;

    if let Some(lang) = compress::detect_lang(rel_path)
        && let Some(tree) = compress::parse_source(&content, lang)
    {
        let symbols = extract_symbols(rel_path, &content, lang, &tree);
        let refs = extract_references(&content, lang, &tree);
        return Some((symbols, refs));
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
        file: rel_path.to_string(),
        line: 1,
        signature: signature_line(&first_line),
        parent: None,
        keywords: refs.clone(),
    };
    Some((vec![sym], refs))
}

fn compute_ranks(
    symbols: &[Symbol],
    file_refs: &HashMap<String, Vec<String>>,
    pagerank_iters: usize,
) -> Vec<f64> {
    // Build name → symbol indices map
    let mut name_to_indices: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, sym) in symbols.iter().enumerate() {
        name_to_indices.entry(&sym.name).or_default().push(i);
    }

    // Build edges
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); symbols.len()];

    let mut file_to_sym_indices: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, sym) in symbols.iter().enumerate() {
        file_to_sym_indices.entry(&sym.file).or_default().push(i);
    }

    for (file, refs) in file_refs {
        let source_indices: Vec<usize> = file_to_sym_indices
            .get(file.as_str())
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

    pagerank(&edges, symbols.len(), pagerank_iters, 0.85)
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

fn build_index(root: &Path, pagerank_iters: usize) -> IndexResult {
    let files = collect_files(root);

    // Parse files in parallel (rayon thread pool)
    let all_results: Vec<(String, Vec<Symbol>, Vec<String>)> = files
        .par_iter()
        .filter_map(|(abs_path, rel_path)| {
            let (syms, refs) = parse_file_entry(abs_path, rel_path)?;
            Some((rel_path.clone(), syms, refs))
        })
        .collect();

    let mut symbols: Vec<Symbol> = Vec::new();
    let mut file_refs: HashMap<String, Vec<String>> = HashMap::new();

    for (file, file_syms, refs) in all_results {
        symbols.extend(file_syms);
        file_refs.insert(file, refs);
    }

    let ranks = compute_ranks(&symbols, &file_refs, pagerank_iters);

    let file_meta_map: HashMap<String, (u64, u64)> = files
        .iter()
        .filter_map(|(abs, _)| file_meta(abs).map(|m| (abs.to_string_lossy().into_owned(), m)))
        .collect();

    IndexResult {
        symbols,
        ranks,
        file_meta: file_meta_map,
        file_refs,
        changed: true,
    }
}

fn build_index_incremental(root: &Path, old_cache: &Cache, pagerank_iters: usize) -> IndexResult {
    let files = collect_files(root);

    // If old cache has no file_refs (pre-upgrade cache format), full rebuild
    if old_cache.file_refs.is_empty() && !old_cache.symbols.is_empty() {
        return build_index(root, pagerank_iters);
    }

    // Partition files into unchanged and changed/new
    let current_rels: std::collections::HashSet<&str> =
        files.iter().map(|(_, rel)| rel.as_str()).collect();
    let has_deletions = old_cache
        .file_refs
        .keys()
        .any(|k| !current_rels.contains(k.as_str()));

    let mut changed: Vec<&(PathBuf, String)> = Vec::new();
    let mut unchanged_rels: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for entry in &files {
        let (abs_path, rel_path) = entry;
        let is_unchanged = file_meta(abs_path)
            .and_then(|meta| {
                old_cache
                    .file_meta
                    .get(&abs_path.to_string_lossy().into_owned())
                    .map(|cached| meta == *cached)
            })
            .unwrap_or(false)
            && old_cache.file_refs.contains_key(rel_path.as_str());

        if is_unchanged {
            unchanged_rels.insert(rel_path.as_str());
        } else {
            changed.push(entry);
        }
    }

    if changed.is_empty() && !has_deletions {
        // Nothing changed — return cached data
        let file_meta_map: HashMap<String, (u64, u64)> = files
            .iter()
            .filter_map(|(abs, _)| file_meta(abs).map(|m| (abs.to_string_lossy().into_owned(), m)))
            .collect();
        return IndexResult {
            symbols: old_cache.symbols.clone(),
            ranks: old_cache.ranks.clone(),
            file_meta: file_meta_map,
            file_refs: old_cache.file_refs.clone(),
            changed: false,
        };
    }

    // Keep symbols and refs from unchanged files
    let mut symbols: Vec<Symbol> = old_cache
        .symbols
        .iter()
        .filter(|s| unchanged_rels.contains(s.file.as_str()))
        .cloned()
        .collect();

    let mut file_refs: HashMap<String, Vec<String>> = old_cache
        .file_refs
        .iter()
        .filter(|(k, _)| unchanged_rels.contains(k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    // Parse only changed/new files
    let new_results: Vec<(String, Vec<Symbol>, Vec<String>)> = changed
        .par_iter()
        .filter_map(|(abs_path, rel_path)| {
            let (syms, refs) = parse_file_entry(abs_path, rel_path)?;
            Some((rel_path.clone(), syms, refs))
        })
        .collect();

    for (file, file_syms, refs) in new_results {
        symbols.extend(file_syms);
        file_refs.insert(file, refs);
    }

    // Recompute PageRank on merged data
    let ranks = compute_ranks(&symbols, &file_refs, pagerank_iters);

    let file_meta_map: HashMap<String, (u64, u64)> = files
        .iter()
        .filter_map(|(abs, _)| file_meta(abs).map(|m| (abs.to_string_lossy().into_owned(), m)))
        .collect();

    IndexResult {
        symbols,
        ranks,
        file_meta: file_meta_map,
        file_refs,
        changed: true,
    }
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

/// Build (or incrementally update) the symbol index and save the cache if anything changed.
fn build_and_save(root: &Path, pagerank_iters: usize) -> IndexResult {
    let result = if let Some(cache) = load_cache(root) {
        build_index_incremental(root, &cache, pagerank_iters)
    } else {
        build_index(root, pagerank_iters)
    };

    if result.changed {
        save_cache(
            root,
            &Cache {
                symbols: result.symbols.clone(),
                ranks: result.ranks.clone(),
                file_meta: result.file_meta.clone(),
                file_refs: result.file_refs.clone(),
            },
        );
    }

    result
}

pub fn search(root: &str, query: &[String], pagerank_iters: usize) -> Result<SearchResult> {
    let root_path = std::fs::canonicalize(root)?;
    let result = build_and_save(&root_path, pagerank_iters);

    let total_symbols = result.symbols.len();
    let scored = score_query(&result.symbols, &result.ranks, query);
    let matches: Vec<(Symbol, f64)> = scored
        .into_iter()
        .take(20)
        .map(|(i, score)| (result.symbols[i].clone(), score))
        .collect();

    Ok(SearchResult {
        matches,
        total_symbols,
    })
}

/// Load all indexed symbols (from cache or fresh build). Used by `why` for dependency lookup.
pub fn load_symbols(root: &Path, pagerank_iters: usize) -> Vec<Symbol> {
    let result = build_and_save(root, pagerank_iters);
    result.symbols
}

/// Delete the symbol cache for the given project root.
pub fn clean_cache(root: &str) -> Result<()> {
    let root_path = std::fs::canonicalize(root)?;
    let path = cache_path(&root_path);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
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

    // ── signature_line ──────────────────────────────────────────────

    #[test]
    fn signature_line_truncates_long_lines() {
        let long = "a".repeat(200);
        let result = signature_line(&long);
        assert_eq!(result.len(), 100); // 97 chars + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn signature_line_short_unchanged() {
        let short = "pub fn foo(x: i32) -> bool";
        assert_eq!(signature_line(short), short);
    }

    #[test]
    fn signature_line_multiline_takes_first() {
        let multi = "pub fn foo() {\n    body\n}";
        assert_eq!(signature_line(multi), "pub fn foo() {");
    }

    // ── split_subwords edge cases ───────────────────────────────────

    #[test]
    fn split_subwords_kebab_case() {
        assert_eq!(split_subwords("my-component"), vec!["my", "component"]);
    }

    #[test]
    fn split_subwords_empty() {
        let result: Vec<String> = split_subwords("");
        assert!(result.is_empty());
    }

    #[test]
    fn split_subwords_leading_underscores() {
        // Leading separator: first segment is empty, skipped
        assert_eq!(split_subwords("_private_field"), vec!["private", "field"]);
    }

    // ── score_query edge cases ──────────────────────────────────────

    #[test]
    fn score_query_empty_tokens() {
        let symbols = vec![Symbol {
            name: "foo".to_string(),
            kind: SymbolKind::Function,
            file: "a.rs".to_string(),
            line: 1,
            signature: "fn foo()".to_string(),
            parent: None,
            keywords: Vec::new(),
        }];
        let ranks = vec![0.5];
        let results = score_query(&symbols, &ranks, &[]);
        assert!(results.is_empty());
    }

    #[test]
    fn score_query_no_match() {
        let symbols = vec![Symbol {
            name: "foo".to_string(),
            kind: SymbolKind::Function,
            file: "a.rs".to_string(),
            line: 1,
            signature: "fn foo()".to_string(),
            parent: None,
            keywords: Vec::new(),
        }];
        let ranks = vec![0.5];
        let results = score_query(&symbols, &ranks, &["zzzzz".to_string()]);
        assert!(results.is_empty());
    }

    #[test]
    fn score_query_path_prefix_match() {
        let symbols = vec![Symbol {
            name: "unrelated".to_string(),
            kind: SymbolKind::Function,
            file: "src/context.rs".to_string(),
            line: 1,
            signature: "fn unrelated()".to_string(),
            parent: None,
            keywords: Vec::new(),
        }];
        let ranks = vec![0.1];
        // "cont" should prefix-match "context" in path
        let results = score_query(&symbols, &ranks, &["cont".to_string()]);
        assert!(!results.is_empty(), "path prefix should match");
    }

    #[test]
    fn score_query_parent_exact_subword_match() {
        let symbols = vec![Symbol {
            name: "do_work".to_string(),
            kind: SymbolKind::Method,
            file: "a.rs".to_string(),
            line: 1,
            signature: "fn do_work()".to_string(),
            parent: Some("MyStruct".to_string()),
            keywords: Vec::new(),
        }];
        let ranks = vec![0.1];
        // "my" should exact-match the parent subword "my" (from split_subwords("MyStruct"))
        let results = score_query(&symbols, &ranks, &["struct".to_string()]);
        assert!(
            !results.is_empty(),
            "parent exact subword match should work"
        );
    }

    #[test]
    fn score_query_parent_prefix_match() {
        let symbols = vec![Symbol {
            name: "do_work".to_string(),
            kind: SymbolKind::Method,
            file: "a.rs".to_string(),
            line: 1,
            signature: "fn do_work()".to_string(),
            parent: Some("MyStruct".to_string()),
            keywords: Vec::new(),
        }];
        let ranks = vec![0.1];
        // "my" should prefix-match parent subword "my"
        let results = score_query(&symbols, &ranks, &["my".to_string()]);
        assert!(!results.is_empty(), "parent prefix match should work");
    }

    #[test]
    fn score_query_keyword_exact_match() {
        let symbols = vec![Symbol {
            name: "README.md".to_string(),
            kind: SymbolKind::File,
            file: "README.md".to_string(),
            line: 1,
            signature: "# My Project".to_string(),
            parent: None,
            keywords: vec!["installation".to_string(), "usage".to_string()],
        }];
        let ranks = vec![0.1];
        let results = score_query(&symbols, &ranks, &["installation".to_string()]);
        assert!(!results.is_empty(), "keyword exact match should work");
    }

    #[test]
    fn score_query_keyword_prefix_match() {
        let symbols = vec![Symbol {
            name: "README.md".to_string(),
            kind: SymbolKind::File,
            file: "README.md".to_string(),
            line: 1,
            signature: "# My Project".to_string(),
            parent: None,
            keywords: vec!["installation".to_string()],
        }];
        let ranks = vec![0.1];
        let results = score_query(&symbols, &ranks, &["instal".to_string()]);
        assert!(!results.is_empty(), "keyword prefix match should work");
    }

    #[test]
    fn score_query_keyword_substring_match() {
        let symbols = vec![Symbol {
            name: "README.md".to_string(),
            kind: SymbolKind::File,
            file: "README.md".to_string(),
            line: 1,
            signature: "# My Project".to_string(),
            parent: None,
            keywords: vec!["installation".to_string()],
        }];
        let ranks = vec![0.1];
        let results = score_query(&symbols, &ranks, &["stallat".to_string()]);
        assert!(!results.is_empty(), "keyword substring match should work");
    }

    #[test]
    fn score_query_multi_token_all_must_match() {
        let symbols = vec![Symbol {
            name: "generate_context".to_string(),
            kind: SymbolKind::Function,
            file: "src/context.rs".to_string(),
            line: 1,
            signature: "fn generate_context()".to_string(),
            parent: None,
            keywords: Vec::new(),
        }];
        let ranks = vec![0.1];
        // Both tokens must match
        let results = score_query(
            &symbols,
            &ranks,
            &["generate".to_string(), "context".to_string()],
        );
        assert!(!results.is_empty());
        // One token doesn't match
        let results = score_query(
            &symbols,
            &ranks,
            &["generate".to_string(), "zzzzz".to_string()],
        );
        assert!(results.is_empty());
    }

    // ── is_keyword ──────────────────────────────────────────────────

    #[test]
    fn is_keyword_rust() {
        assert!(is_keyword("self", Lang::Rust));
        assert!(is_keyword("let", Lang::Rust));
        assert!(!is_keyword("generate", Lang::Rust));
    }

    #[test]
    fn is_keyword_python() {
        assert!(is_keyword("self", Lang::Python));
        assert!(is_keyword("def", Lang::Python));
        assert!(is_keyword("None", Lang::Python));
        assert!(!is_keyword("generate", Lang::Python));
    }

    #[test]
    fn is_keyword_javascript() {
        assert!(is_keyword("this", Lang::JavaScript));
        assert!(is_keyword("const", Lang::JavaScript));
        assert!(!is_keyword("myFunc", Lang::JavaScript));
    }

    #[test]
    fn is_keyword_typescript() {
        assert!(is_keyword("async", Lang::TypeScript));
        assert!(!is_keyword("myFunc", Lang::TypeScript));
    }

    #[test]
    fn is_keyword_tsx() {
        assert!(is_keyword("await", Lang::Tsx));
        assert!(!is_keyword("Component", Lang::Tsx));
    }

    #[test]
    fn is_keyword_go() {
        assert!(is_keyword("func", Lang::Go));
        assert!(is_keyword("nil", Lang::Go));
        assert!(!is_keyword("Handler", Lang::Go));
    }

    #[test]
    fn is_keyword_c() {
        assert!(is_keyword("int", Lang::C));
        assert!(is_keyword("NULL", Lang::C));
        assert!(!is_keyword("myFunc", Lang::C));
    }

    #[test]
    fn is_keyword_cpp() {
        assert!(is_keyword("class", Lang::Cpp));
        assert!(is_keyword("this", Lang::Cpp));
        assert!(!is_keyword("MyClass", Lang::Cpp));
    }

    #[test]
    fn is_keyword_java() {
        assert!(is_keyword("public", Lang::Java));
        assert!(is_keyword("abstract", Lang::Java));
        assert!(!is_keyword("MyClass", Lang::Java));
    }

    // ── compute_ranks ───────────────────────────────────────────────

    #[test]
    fn compute_ranks_with_refs() {
        // fn bar() in a.rs calls "foo", fn foo() in b.rs
        let symbols = vec![
            Symbol {
                name: "bar".to_string(),
                kind: SymbolKind::Function,
                file: "a.rs".to_string(),
                line: 1,
                signature: "fn bar()".to_string(),
                parent: None,
                keywords: Vec::new(),
            },
            Symbol {
                name: "foo".to_string(),
                kind: SymbolKind::Function,
                file: "b.rs".to_string(),
                line: 1,
                signature: "fn foo()".to_string(),
                parent: None,
                keywords: Vec::new(),
            },
        ];
        let mut file_refs = HashMap::new();
        file_refs.insert("a.rs".to_string(), vec!["foo".to_string()]);
        file_refs.insert("b.rs".to_string(), vec![]);

        let ranks = compute_ranks(&symbols, &file_refs, 15);
        assert_eq!(ranks.len(), 2);
        // foo (target) should have higher rank than bar (source)
        assert!(ranks[1] > ranks[0], "foo should rank higher than bar");
    }

    #[test]
    fn compute_ranks_no_refs() {
        let symbols = vec![Symbol {
            name: "lonely".to_string(),
            kind: SymbolKind::Function,
            file: "a.rs".to_string(),
            line: 1,
            signature: "fn lonely()".to_string(),
            parent: None,
            keywords: Vec::new(),
        }];
        let file_refs = HashMap::new();
        let ranks = compute_ranks(&symbols, &file_refs, 15);
        assert_eq!(ranks.len(), 1);
    }

    #[test]
    fn compute_ranks_skips_non_function_sources() {
        // Only Function/Method kinds are used as source nodes for edges
        let symbols = vec![
            Symbol {
                name: "MyStruct".to_string(),
                kind: SymbolKind::Struct,
                file: "a.rs".to_string(),
                line: 1,
                signature: "struct MyStruct".to_string(),
                parent: None,
                keywords: Vec::new(),
            },
            Symbol {
                name: "foo".to_string(),
                kind: SymbolKind::Function,
                file: "b.rs".to_string(),
                line: 1,
                signature: "fn foo()".to_string(),
                parent: None,
                keywords: Vec::new(),
            },
        ];
        let mut file_refs = HashMap::new();
        // a.rs references foo, but only has a struct (not function), so no edge
        file_refs.insert("a.rs".to_string(), vec!["foo".to_string()]);

        let ranks = compute_ranks(&symbols, &file_refs, 15);
        assert_eq!(ranks.len(), 2);
        // Without edges, ranks should be roughly equal
        assert!(
            (ranks[0] - ranks[1]).abs() < 0.01,
            "without function sources, ranks should be equal"
        );
    }

    #[test]
    fn compute_ranks_self_ref_no_edge() {
        // A function referencing itself should not create a self-edge
        let symbols = vec![Symbol {
            name: "recurse".to_string(),
            kind: SymbolKind::Function,
            file: "a.rs".to_string(),
            line: 1,
            signature: "fn recurse()".to_string(),
            parent: None,
            keywords: Vec::new(),
        }];
        let mut file_refs = HashMap::new();
        file_refs.insert("a.rs".to_string(), vec!["recurse".to_string()]);

        let ranks = compute_ranks(&symbols, &file_refs, 15);
        assert_eq!(ranks.len(), 1);
        // Should still get a valid rank (no self-edge means no boost)
        assert!(ranks[0] > 0.0);
    }

    // ── extract_plaintext_refs ──────────────────────────────────────

    #[test]
    fn extract_plaintext_refs_basic() {
        let content = "hello world foo_bar baz-qux a";
        let refs = extract_plaintext_refs(content);
        assert!(refs.contains(&"hello".to_string()));
        assert!(refs.contains(&"world".to_string()));
        assert!(refs.contains(&"foo_bar".to_string()));
        assert!(refs.contains(&"baz-qux".to_string()));
        // Single char "a" should be excluded (len <= 1)
        assert!(!refs.contains(&"a".to_string()));
    }

    #[test]
    fn extract_plaintext_refs_deduplicates() {
        let content = "hello hello hello";
        let refs = extract_plaintext_refs(content);
        assert_eq!(refs.iter().filter(|r| *r == "hello").count(), 1);
    }

    // ── extract_symbols for Rust ────────────────────────────────────

    fn parse_rust(code: &str) -> (Vec<Symbol>, Vec<String>) {
        let tree = compress::parse_source(code, Lang::Rust).unwrap();
        let syms = extract_symbols("test.rs", code, Lang::Rust, &tree);
        let refs = extract_references(code, Lang::Rust, &tree);
        (syms, refs)
    }

    #[test]
    fn extract_rust_trait() {
        let code = r#"
trait Drawable {
    fn draw(&self);
    fn resize(&self, w: u32, h: u32);
}
"#;
        let (syms, _) = parse_rust(code);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Drawable"), "trait should be extracted");
        let trait_sym = syms.iter().find(|s| s.name == "Drawable").unwrap();
        assert_eq!(trait_sym.kind, SymbolKind::Trait);
    }

    #[test]
    fn extract_rust_trait_with_impl_methods() {
        // Methods with bodies are extracted from impl blocks
        let code = r#"
trait Drawable {
    fn draw(&self);
}

struct Circle;

impl Drawable for Circle {
    fn draw(&self) {
        println!("drawing");
    }
}
"#;
        let (syms, _) = parse_rust(code);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Drawable"), "trait should be extracted");
        assert!(names.contains(&"Circle"), "struct should be extracted");
        assert!(names.contains(&"draw"), "impl method should be extracted");
    }

    #[test]
    fn extract_rust_macro() {
        let code = r#"
macro_rules! my_macro {
    ($x:expr) => { $x + 1 };
}
"#;
        let (syms, _) = parse_rust(code);
        let mac = syms.iter().find(|s| s.name == "my_macro");
        assert!(mac.is_some(), "macro should be extracted");
        assert_eq!(mac.unwrap().kind, SymbolKind::Macro);
    }

    #[test]
    fn extract_rust_const_and_static() {
        let code = r#"
const MAX_SIZE: usize = 100;
static GLOBAL: i32 = 42;
"#;
        let (syms, _) = parse_rust(code);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"MAX_SIZE"));
        assert!(names.contains(&"GLOBAL"));
        for s in &syms {
            assert_eq!(s.kind, SymbolKind::Const);
        }
    }

    #[test]
    fn extract_rust_type_alias() {
        let code = "type MyResult = Result<(), String>;\n";
        let (syms, _) = parse_rust(code);
        let found = syms.iter().find(|s| s.name == "MyResult");
        assert!(found.is_some());
        assert_eq!(found.unwrap().kind, SymbolKind::Type);
    }

    #[test]
    fn extract_rust_enum() {
        let code = r#"
enum Color {
    Red,
    Green,
    Blue,
}
"#;
        let (syms, _) = parse_rust(code);
        let found = syms.iter().find(|s| s.name == "Color");
        assert!(found.is_some());
        assert_eq!(found.unwrap().kind, SymbolKind::Enum);
    }

    // ── extract_symbols for Python ──────────────────────────────────

    fn parse_python(code: &str) -> Vec<Symbol> {
        let tree = compress::parse_source(code, Lang::Python).unwrap();
        extract_symbols("test.py", code, Lang::Python, &tree)
    }

    #[test]
    fn extract_python_decorated_definition() {
        let code = r#"
@decorator
def my_func():
    pass

@other_decorator
class MyClass:
    pass
"#;
        let syms = parse_python(code);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"my_func"),
            "decorated function should be extracted"
        );
        assert!(
            names.contains(&"MyClass"),
            "decorated class should be extracted"
        );
    }

    // ── extract_symbols for Go ──────────────────────────────────────

    fn parse_go(code: &str) -> Vec<Symbol> {
        let tree = compress::parse_source(code, Lang::Go).unwrap();
        extract_symbols("test.go", code, Lang::Go, &tree)
    }

    #[test]
    fn extract_go_function_and_type() {
        let code = r#"
package main

func Hello() {}

type MyStruct struct {
    Name string
}
"#;
        let syms = parse_go(code);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Hello"), "Go function should be extracted");
        assert!(names.contains(&"MyStruct"), "Go struct should be extracted");
    }

    // ── extract_symbols for Java ────────────────────────────────────

    fn parse_java(code: &str) -> Vec<Symbol> {
        let tree = compress::parse_source(code, Lang::Java).unwrap();
        extract_symbols("Test.java", code, Lang::Java, &tree)
    }

    #[test]
    fn extract_java_interface() {
        let code = r#"
interface Drawable {
    void draw();
}
"#;
        let syms = parse_java(code);
        let iface = syms.iter().find(|s| s.name == "Drawable");
        assert!(iface.is_some(), "Java interface should be extracted");
        assert_eq!(iface.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn extract_java_enum() {
        let code = r#"
enum Direction {
    NORTH, SOUTH, EAST, WEST
}
"#;
        let syms = parse_java(code);
        let found = syms.iter().find(|s| s.name == "Direction");
        assert!(found.is_some(), "Java enum should be extracted");
        assert_eq!(found.unwrap().kind, SymbolKind::Enum);
    }

    // ── extract_symbols for JS/TS ───────────────────────────────────

    fn parse_js(code: &str) -> Vec<Symbol> {
        let tree = compress::parse_source(code, Lang::JavaScript).unwrap();
        extract_symbols("test.js", code, Lang::JavaScript, &tree)
    }

    #[test]
    fn extract_js_enum_declaration() {
        // TypeScript enum (also parsed in JS mode for tree-sitter)
        let code = r#"
function hello() {}
"#;
        let syms = parse_js(code);
        let found = syms.iter().find(|s| s.name == "hello");
        assert!(found.is_some());
        assert_eq!(found.unwrap().kind, SymbolKind::Function);
    }

    // ── cache round-trip ────────────────────────────────────────────

    #[test]
    fn cache_serialize_deserialize_roundtrip() {
        let cache = Cache {
            symbols: vec![Symbol {
                name: "test_fn".to_string(),
                kind: SymbolKind::Function,
                file: "src/lib.rs".to_string(),
                line: 10,
                signature: "fn test_fn()".to_string(),
                parent: None,
                keywords: vec!["helper".to_string()],
            }],
            ranks: vec![0.42],
            file_meta: {
                let mut m = HashMap::new();
                m.insert("/tmp/test.rs".to_string(), (1234u64, 5678u64));
                m
            },
            file_refs: {
                let mut m = HashMap::new();
                m.insert("src/lib.rs".to_string(), vec!["foo".to_string()]);
                m
            },
        };

        let data = wincode::serialize(&cache).expect("serialize");
        let restored: Cache = wincode::deserialize(&data).expect("deserialize");

        assert_eq!(restored.symbols.len(), 1);
        assert_eq!(restored.symbols[0].name, "test_fn");
        assert_eq!(restored.symbols[0].keywords, vec!["helper"]);
        assert_eq!(restored.ranks, vec![0.42]);
        assert_eq!(
            restored.file_meta.get("/tmp/test.rs"),
            Some(&(1234u64, 5678u64))
        );
        assert_eq!(
            restored.file_refs.get("src/lib.rs"),
            Some(&vec!["foo".to_string()])
        );
    }

    // ── save_cache / load_cache round-trip ───────────────────────────

    #[test]
    fn save_and_load_cache_via_filesystem() {
        let tmp = std::env::temp_dir().join("supp-test-cache-roundtrip");
        let _ = std::fs::create_dir_all(&tmp);

        // Write a cache file manually to the expected path
        let cp = cache_path(&tmp);
        if let Some(parent) = cp.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let cache = Cache {
            symbols: vec![Symbol {
                name: "hello".to_string(),
                kind: SymbolKind::Function,
                file: "main.rs".to_string(),
                line: 1,
                signature: "fn hello()".to_string(),
                parent: None,
                keywords: Vec::new(),
            }],
            ranks: vec![0.5],
            file_meta: HashMap::new(),
            file_refs: HashMap::new(),
        };

        save_cache(&tmp, &cache);
        let loaded = load_cache(&tmp);
        assert!(loaded.is_some(), "cache should load after save");
        let loaded = loaded.unwrap();
        assert_eq!(loaded.symbols.len(), 1);
        assert_eq!(loaded.symbols[0].name, "hello");

        // Cleanup
        let _ = std::fs::remove_file(&cp);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── clean_cache ─────────────────────────────────────────────────

    #[test]
    fn clean_cache_removes_file() {
        let tmp = std::env::temp_dir().join("supp-test-clean-cache");
        let _ = std::fs::create_dir_all(&tmp);

        let cache = Cache {
            symbols: Vec::new(),
            ranks: Vec::new(),
            file_meta: HashMap::new(),
            file_refs: HashMap::new(),
        };
        save_cache(&tmp, &cache);

        let cp = cache_path(&tmp);
        assert!(cp.exists(), "cache file should exist after save");

        // clean_cache expects a canonicalized string
        let result = clean_cache(tmp.to_str().unwrap());
        assert!(result.is_ok());
        assert!(!cp.exists(), "cache file should be removed after clean");

        // Cleaning again when no file exists should also succeed
        let result = clean_cache(tmp.to_str().unwrap());
        assert!(result.is_ok());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── SymbolKind::tag ─────────────────────────────────────────────

    #[test]
    fn symbol_kind_tags() {
        assert_eq!(SymbolKind::Function.tag(), "fn");
        assert_eq!(SymbolKind::Struct.tag(), "st");
        assert_eq!(SymbolKind::Enum.tag(), "en");
        assert_eq!(SymbolKind::Trait.tag(), "tr");
        assert_eq!(SymbolKind::Class.tag(), "cl");
        assert_eq!(SymbolKind::Interface.tag(), "if");
        assert_eq!(SymbolKind::Method.tag(), "me");
        assert_eq!(SymbolKind::Type.tag(), "ty");
        assert_eq!(SymbolKind::Const.tag(), "co");
        assert_eq!(SymbolKind::Macro.tag(), "ma");
        assert_eq!(SymbolKind::File.tag(), "fi");
    }

    // ── pagerank with disconnected nodes ────────────────────────────

    #[test]
    fn pagerank_disconnected() {
        // No edges at all among 3 nodes
        let edges = vec![vec![], vec![], vec![]];
        let ranks = pagerank(&edges, 3, 20, 0.85);
        // All ranks should be equal to each other (converges to (1-d)/n)
        assert!(
            (ranks[0] - ranks[1]).abs() < 0.001 && (ranks[1] - ranks[2]).abs() < 0.001,
            "disconnected nodes should have equal rank: {:?}",
            ranks
        );
    }

    // ── is_function_node ────────────────────────────────────────────

    #[test]
    fn is_function_node_matches() {
        assert!(is_function_node("function_item", Lang::Rust));
        assert!(is_function_node("function_definition", Lang::Python));
        assert!(is_function_node("function_declaration", Lang::Go));
        assert!(is_function_node("method_declaration", Lang::Java));
        assert!(is_function_node("method_definition", Lang::JavaScript));
        assert!(is_function_node("constructor_declaration", Lang::Java));
        assert!(!is_function_node("struct_item", Lang::Rust));
        assert!(!is_function_node("class_definition", Lang::Python));
    }

    // ── search public API ────────────────────────────────────────────

    #[test]
    fn search_finds_symbols_in_rust_project() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Create a mini Rust project with a function
        std::fs::write(
            tmp.path().join("lib.rs"),
            "pub fn find_me() -> i32 { 42 }\npub fn other() {}\n",
        )
        .unwrap();
        let result = search(tmp.path().to_str().unwrap(), &["find_me".to_string()], 15);
        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(result.total_symbols > 0);
        assert!(!result.matches.is_empty(), "should find find_me symbol");
        assert_eq!(result.matches[0].0.name, "find_me");
    }

    #[test]
    fn search_no_results_for_nonexistent() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("lib.rs"), "pub fn hello() {}\n").unwrap();
        let result = search(
            tmp.path().to_str().unwrap(),
            &["zzzznonexistent".to_string()],
            15,
        )
        .unwrap();
        assert!(result.matches.is_empty());
    }

    #[test]
    fn load_symbols_returns_symbols() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("lib.rs"),
            "pub fn my_func() {}\npub struct MyStruct {}\n",
        )
        .unwrap();
        let symbols = load_symbols(tmp.path(), 15);
        assert!(symbols.len() >= 2);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"my_func"));
        assert!(names.contains(&"MyStruct"));
    }

    // ── parse_file_entry for non-code files ──────────────────────────

    #[test]
    fn parse_file_entry_non_code_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("README.md");
        std::fs::write(&path, "# My Project\nThis is about foo_bar and baz.\n").unwrap();
        let result = parse_file_entry(&path, "README.md");
        assert!(result.is_some());
        let (symbols, refs) = result.unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].kind, SymbolKind::File);
        assert_eq!(symbols[0].name, "README.md");
        assert!(refs.contains(&"foo_bar".to_string()));
    }

    #[test]
    fn parse_file_entry_code_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("lib.rs");
        std::fs::write(&path, "pub fn hello() {}\n").unwrap();
        let result = parse_file_entry(&path, "lib.rs");
        assert!(result.is_some());
        let (symbols, _refs) = result.unwrap();
        assert!(symbols.iter().any(|s| s.name == "hello"));
    }

    #[test]
    fn parse_file_entry_nonexistent() {
        let result = parse_file_entry(Path::new("/tmp/nonexistent_supp_xyz.rs"), "nonexistent.rs");
        assert!(result.is_none());
    }

    // ── extract_python_assignment ────────────────────────────────────

    #[test]
    fn extract_python_module_level_assignment() {
        let code = "MAX_SIZE = 100\nDEFAULT_NAME = 'hello'\ndef func():\n    pass\n";
        let syms = parse_python(code);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"MAX_SIZE"),
            "module-level assignment should be extracted"
        );
        assert!(
            names.contains(&"DEFAULT_NAME"),
            "module-level assignment should be extracted"
        );
        assert!(names.contains(&"func"));
    }

    // ── build_and_save caching ───────────────────────────────────────

    #[test]
    fn build_and_save_creates_cache() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}\n").unwrap();
        let result = build_and_save(tmp.path(), 15);
        assert!(!result.symbols.is_empty());
        // Cache should have been saved
        let cp = cache_path(tmp.path());
        assert!(cp.exists(), "cache file should be created");
    }

    #[test]
    fn build_and_save_incremental_uses_cache() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}\n").unwrap();
        // First build
        let result1 = build_and_save(tmp.path(), 15);
        assert!(result1.changed);
        // Second build without changes - should use cache
        let result2 = build_and_save(tmp.path(), 15);
        assert!(
            !result2.changed,
            "second build should use cache and not be marked changed"
        );
        assert_eq!(result1.symbols.len(), result2.symbols.len());
    }

    // ── file_meta ────────────────────────────────────────────────────

    #[test]
    fn file_meta_existing_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");
        std::fs::write(&path, "hello").unwrap();
        let meta = file_meta(&path);
        assert!(meta.is_some());
        let (mtime, size) = meta.unwrap();
        assert!(mtime > 0);
        assert_eq!(size, 5);
    }

    #[test]
    fn file_meta_nonexistent() {
        let meta = file_meta(Path::new("/tmp/nonexistent_supp_xyz"));
        assert!(meta.is_none());
    }
}
