use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::compress::{self, Mode};
use crate::symbol::{self, Symbol, SymbolKind};
use crate::why;

// ── Result type ─────────────────────────────────────────────────────

pub struct CtxResult {
    pub plain: String,
    pub target_file: String,
    pub target_lines: usize,
    pub dep_file_count: usize,
    pub used_by_count: usize,
}

// ── Import classification ───────────────────────────────────────────

struct ResolvedImport {
    name: String,
    file: Option<String>,     // Some(rel_path) for in-project
    line: Option<usize>,
    kind: Option<SymbolKind>,
    module: String,            // original module path
}

// ── Used-by reference ───────────────────────────────────────────────

struct UsedByRef {
    file: String,
    line: usize,
    symbol_name: String,
    caller: Option<String>,
}

// ── Public API ──────────────────────────────────────────────────────

pub fn analyze(root: &str, file: &str) -> Result<CtxResult> {
    let root_path = std::fs::canonicalize(root)?;
    let file_path = Path::new(file);

    // Resolve to relative path
    let rel_path = if file_path.is_absolute() {
        file_path
            .strip_prefix(&root_path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| file.to_string())
    } else {
        // Canonicalize then strip prefix to normalize ../foo paths
        let abs = root_path.join(file_path);
        if let Ok(canon) = std::fs::canonicalize(&abs) {
            canon
                .strip_prefix(&root_path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| file.to_string())
        } else {
            file.to_string()
        }
    };

    // Read target file
    let abs_target = root_path.join(&rel_path);
    let content = std::fs::read_to_string(&abs_target)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {}", rel_path, e))?;
    let target_lines = content.lines().count();

    // Extract imports
    let imports = why::extract_file_imports(&content, &rel_path);

    // Load project symbol index
    let all_symbols = symbol::load_symbols(&root_path);

    // Symbols defined in target file
    let target_symbols: Vec<&Symbol> = all_symbols
        .iter()
        .filter(|s| s.file == rel_path && s.kind != SymbolKind::File)
        .collect();

    // Resolve imports against symbol index
    let mut resolved: Vec<ResolvedImport> = Vec::new();
    let mut dep_files: HashMap<String, Vec<String>> = HashMap::new(); // file → [imported names]

    for (name, module) in &imports {
        // Try to find in symbol index
        let found = all_symbols.iter().find(|s| s.name == *name && s.file != rel_path);

        if let Some(sym) = found {
            dep_files
                .entry(sym.file.clone())
                .or_default()
                .push(name.clone());
            resolved.push(ResolvedImport {
                name: name.clone(),
                file: Some(sym.file.clone()),
                line: Some(sym.line),
                kind: Some(sym.kind),
                module: module.clone(),
            });
        } else if let Some(resolved_file) =
            why::resolve_relative_import(module, &rel_path, &root_path)
        {
            dep_files
                .entry(resolved_file.clone())
                .or_default()
                .push(name.clone());
            resolved.push(ResolvedImport {
                name: name.clone(),
                file: Some(resolved_file),
                line: None,
                kind: None,
                module: module.clone(),
            });
        } else {
            resolved.push(ResolvedImport {
                name: name.clone(),
                file: None,
                line: None,
                kind: None,
                module: module.clone(),
            });
        }
    }

    resolved.sort_by(|a, b| a.name.cmp(&b.name));

    // Generate dependency signatures (Map mode) for in-project deps
    let mut dep_sections: Vec<(String, Vec<String>, String)> = Vec::new(); // (file, imports, map_output)
    let mut dep_files_sorted: Vec<_> = dep_files.into_iter().collect();
    dep_files_sorted.sort_by(|a, b| a.0.cmp(&b.0));

    for (dep_file, imported_names) in &dep_files_sorted {
        let dep_abs = root_path.join(dep_file);
        if let Ok(dep_content) = std::fs::read_to_string(&dep_abs) {
            let map_output = compress::compress(&dep_content, dep_file, Mode::Map);
            if !map_output.trim().is_empty() {
                dep_sections.push((dep_file.clone(), imported_names.clone(), map_output));
            }
        }
    }

    // Used-by scan: walk project files looking for references to target's symbols
    let symbol_names: Vec<&str> = target_symbols
        .iter()
        .filter(|s| s.name.len() > 2)
        .map(|s| s.name.as_str())
        .collect();

    let used_by = find_file_references(&root_path, &rel_path, &symbol_names);

    // Target file in Slim mode
    let slim_content = compress::compress(&content, &rel_path, Mode::Slim);

    // Assemble markdown
    let plain = assemble_markdown(
        &rel_path,
        target_lines,
        &resolved,
        &target_symbols,
        &dep_sections,
        &used_by,
        &slim_content,
    );

    Ok(CtxResult {
        plain,
        target_file: rel_path,
        target_lines,
        dep_file_count: dep_files_sorted.len(),
        used_by_count: used_by.len(),
    })
}

// ── Used-by scan ────────────────────────────────────────────────────

fn find_file_references(
    root: &Path,
    target_file: &str,
    symbol_names: &[&str],
) -> Vec<UsedByRef> {
    if symbol_names.is_empty() {
        return Vec::new();
    }

    let mut refs = Vec::new();
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

        // Skip target file itself
        if rel == target_file {
            continue;
        }

        // Only check source files
        if compress::detect_lang(&rel).is_none() {
            continue;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lang = compress::detect_lang(&rel);
        let mut tree_cache: Option<tree_sitter::Tree> = None;
        let mut parsed = false;

        for (line_idx, line) in content.lines().enumerate() {
            for &sym_name in symbol_names {
                if why::contains_identifier(line, sym_name) {
                    let caller = if !parsed {
                        tree_cache = lang.and_then(|l| compress::parse_source(&content, l));
                        parsed = true;
                        tree_cache.as_ref().and_then(|t| {
                            find_enclosing_fn(t, &content, line_idx)
                        })
                    } else {
                        tree_cache.as_ref().and_then(|t| {
                            find_enclosing_fn(t, &content, line_idx)
                        })
                    };

                    refs.push(UsedByRef {
                        file: rel.clone(),
                        line: line_idx + 1,
                        symbol_name: sym_name.to_string(),
                        caller,
                    });
                    break; // one match per line is enough
                }
            }
        }
    }

    refs.truncate(30);
    refs
}

fn find_enclosing_fn(
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
                    | "method_declaration"
                    | "function_declaration"
                    | "arrow_function"
            );

            if let Some(inner) = find_enclosing_fn_recursive(child, content, line) {
                return Some(inner);
            }

            if is_fn {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node
                        .utf8_text(content.as_bytes())
                        .unwrap_or_default()
                        .to_string();
                    return Some(name);
                }
            }
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

// ── Markdown assembly ───────────────────────────────────────────────

fn assemble_markdown(
    target_file: &str,
    target_lines: usize,
    resolved: &[ResolvedImport],
    target_symbols: &[&Symbol],
    dep_sections: &[(String, Vec<String>, String)],
    used_by: &[UsedByRef],
    slim_content: &str,
) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    // Header
    let _ = writeln!(out, "## Target: {} ({} lines)", target_file, target_lines);
    let _ = writeln!(out);

    // Imports → Resolved
    if !resolved.is_empty() {
        let _ = writeln!(out, "## Imports → Resolved");
        let max_name = resolved.iter().map(|r| r.name.len()).max().unwrap_or(0);
        for imp in resolved {
            if let Some(ref file) = imp.file {
                let loc = if let Some(line) = imp.line {
                    format!("{}:{}", file, line)
                } else {
                    file.clone()
                };
                let kind_tag = imp
                    .kind
                    .map(|k| format!(" [{}]", k.tag()))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "- {:<width$} → {}{}",
                    imp.name,
                    loc,
                    kind_tag,
                    width = max_name
                );
            } else {
                let _ = writeln!(
                    out,
                    "- {:<width$} → ({} — external)",
                    imp.name,
                    imp.module,
                    width = max_name
                );
            }
        }
        let _ = writeln!(out);
    }

    // Definitions
    if !target_symbols.is_empty() {
        let _ = writeln!(out, "## Definitions ({} symbols)", target_symbols.len());
        for sym in target_symbols {
            let display = if let Some(ref parent) = sym.parent {
                format!("{}::{}", parent, sym.name)
            } else {
                sym.name.clone()
            };
            let _ = writeln!(
                out,
                "- [{}] {:<30} line {}",
                sym.kind.tag(),
                display,
                sym.line
            );
        }
        let _ = writeln!(out);
    }

    // Dependency Signatures
    if !dep_sections.is_empty() {
        let _ = writeln!(out, "## Dependency Signatures");
        for (file, imported_names, map_output) in dep_sections {
            let _ = writeln!(
                out,
                "### {} (imports: {})",
                file,
                imported_names.join(", ")
            );
            let hint = compress::lang_hint(file);
            let _ = writeln!(out, "```{}", hint);
            let _ = write!(out, "{}", map_output.trim_end());
            let _ = writeln!(out);
            let _ = writeln!(out, "```");
            let _ = writeln!(out);
        }
    }

    // Used By
    if !used_by.is_empty() {
        let _ = writeln!(out, "## Used By");
        for r in used_by {
            let caller_str = r
                .caller
                .as_ref()
                .map(|c| format!(" (in {})", c))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "- {}:{} → {}{}",
                r.file, r.line, r.symbol_name, caller_str
            );
        }
        let _ = writeln!(out);
    }

    // Source
    let _ = writeln!(out, "## Source");
    let hint = compress::lang_hint(target_file);
    let _ = writeln!(out, "```{}", hint);
    let _ = write!(out, "{}", slim_content.trim_end());
    let _ = writeln!(out);
    let _ = writeln!(out, "```");

    out
}
