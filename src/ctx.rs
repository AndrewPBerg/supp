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

pub fn analyze(root: &str, file: &str, mode: Mode) -> Result<CtxResult> {
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
    let imports = why::extract_file_imports(&content, &rel_path, &root_path);

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

    // Dependency info for in-project deps
    let mut dep_files_sorted: Vec<_> = dep_files.into_iter().collect();
    dep_files_sorted.sort_by(|a, b| a.0.cmp(&b.0));

    // In slim mode: compact symbol tables per dep file
    // Otherwise: full Map-mode code blocks
    let mut dep_sections: Vec<(String, Vec<String>, String)> = Vec::new();
    let mut dep_sym_sections: Vec<(String, Vec<String>, Vec<&Symbol>)> = Vec::new();

    for (dep_file, imported_names) in &dep_files_sorted {
        if mode == Mode::Slim {
            let dep_syms: Vec<&Symbol> = all_symbols
                .iter()
                .filter(|s| s.file == *dep_file && s.kind != SymbolKind::File)
                .collect();
            if !dep_syms.is_empty() {
                dep_sym_sections.push((dep_file.clone(), imported_names.clone(), dep_syms));
            }
        } else {
            let dep_abs = root_path.join(dep_file);
            if let Ok(dep_content) = std::fs::read_to_string(&dep_abs) {
                let map_output = compress::compress(&dep_content, dep_file, Mode::Map);
                if !map_output.trim().is_empty() {
                    dep_sections.push((dep_file.clone(), imported_names.clone(), map_output));
                }
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

    // Target file source section
    let source_content = match mode {
        Mode::Map => compress::compress(&content, &rel_path, Mode::Map),
        Mode::Slim => compress::compress(&content, &rel_path, Mode::Slim),
        Mode::Full => content.clone(),
    };

    // Assemble markdown
    let plain = assemble_markdown(
        &rel_path,
        target_lines,
        &resolved,
        &target_symbols,
        &dep_sections,
        &dep_sym_sections,
        &used_by,
        &source_content,
        mode,
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
        let lang = match compress::detect_lang(&rel) {
            Some(l) => l,
            None => continue,
        };

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let tree = compress::parse_source(&content, lang);

        for (line_idx, line) in content.lines().enumerate() {
            for &sym_name in symbol_names {
                if why::contains_identifier(line, sym_name) {
                    let caller = tree.as_ref().and_then(|t| {
                        why::find_enclosing_function(t, &content, line_idx)
                    });

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

// ── Markdown assembly ───────────────────────────────────────────────

fn assemble_markdown(
    target_file: &str,
    target_lines: usize,
    resolved: &[ResolvedImport],
    target_symbols: &[&Symbol],
    dep_sections: &[(String, Vec<String>, String)],
    dep_sym_sections: &[(String, Vec<String>, Vec<&Symbol>)],
    used_by: &[UsedByRef],
    source_content: &str,
    mode: Mode,
) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let slim = mode == Mode::Slim;

    // Header
    let mode_label = match mode {
        Mode::Slim => " [blast radius]",
        Mode::Map => " [signatures]",
        Mode::Full => "",
    };
    let _ = writeln!(out, "## Target: {} ({} lines){}", target_file, target_lines, mode_label);
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

    // Definitions — in slim mode include signatures for richer context
    if !target_symbols.is_empty() {
        let _ = writeln!(out, "## Definitions ({} symbols)", target_symbols.len());
        for sym in target_symbols {
            let display = if let Some(ref parent) = sym.parent {
                format!("{}::{}", parent, sym.name)
            } else {
                sym.name.clone()
            };
            if slim && !sym.signature.is_empty() {
                let _ = writeln!(
                    out,
                    "- [{}] {:<30} line {}  {}",
                    sym.kind.tag(),
                    display,
                    sym.line,
                    sym.signature
                );
            } else {
                let _ = writeln!(
                    out,
                    "- [{}] {:<30} line {}",
                    sym.kind.tag(),
                    display,
                    sym.line
                );
            }
        }
        let _ = writeln!(out);
    }

    // Dependency info — code blocks (default/map) or compact sym tables (slim)
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
    if !dep_sym_sections.is_empty() {
        let _ = writeln!(out, "## Dependencies");
        for (file, imported_names, syms) in dep_sym_sections {
            let _ = writeln!(
                out,
                "### {} (imports: {})",
                file,
                imported_names.join(", ")
            );
            for sym in syms {
                let display = if let Some(ref parent) = sym.parent {
                    format!("{}::{}", parent, sym.name)
                } else {
                    sym.name.clone()
                };
                if !sym.signature.is_empty() {
                    let _ = writeln!(out, "- [{}] {}  {}", sym.kind.tag(), display, sym.signature);
                } else {
                    let _ = writeln!(out, "- [{}] {}", sym.kind.tag(), display);
                }
            }
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

    // Source / Signatures
    let section_label = match mode {
        Mode::Map => "Signatures",
        _ => "Source",
    };
    let _ = writeln!(out, "## {}", section_label);
    let hint = compress::lang_hint(target_file);
    let _ = writeln!(out, "```{}", hint);
    let _ = write!(out, "{}", source_content.trim_end());
    let _ = writeln!(out);
    let _ = writeln!(out, "```");

    out
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn nonexistent_file_errors() {
        let dir = setup(&[]);
        let result = analyze(dir.path().to_str().unwrap(), "nope.rs", Mode::Full);
        assert!(result.is_err());
    }

    #[test]
    fn single_file_no_deps() {
        let dir = setup(&[("main.rs", "fn main() {\n    println!(\"hello\");\n}\n")]);
        let result = analyze(dir.path().to_str().unwrap(), "main.rs", Mode::Full).unwrap();
        assert_eq!(result.target_file, "main.rs");
        assert_eq!(result.target_lines, 3);
        assert_eq!(result.dep_file_count, 0);
        assert_eq!(result.used_by_count, 0);
        assert!(result.plain.contains("## Target: main.rs"));
        assert!(result.plain.contains("## Source"));
    }

    #[test]
    fn detects_definitions() {
        let dir = setup(&[(
            "lib.rs",
            "pub struct Foo {\n    pub x: i32,\n}\n\npub fn bar() -> Foo {\n    Foo { x: 1 }\n}\n",
        )]);
        let result = analyze(dir.path().to_str().unwrap(), "lib.rs", Mode::Full).unwrap();
        assert!(result.plain.contains("## Definitions"));
        assert!(result.plain.contains("[st] Foo"));
        assert!(result.plain.contains("[fn] bar"));
    }

    #[test]
    fn resolves_rust_imports() {
        let dir = setup(&[
            ("config.rs", "pub struct Config {\n    pub debug: bool,\n}\n"),
            (
                "main.rs",
                "use crate::config::Config;\n\nfn main() {\n    let _c = Config { debug: true };\n}\n",
            ),
        ]);
        let result = analyze(dir.path().to_str().unwrap(), "main.rs", Mode::Full).unwrap();
        assert!(result.plain.contains("## Imports"));
        assert!(result.plain.contains("Config"));
        assert!(result.dep_file_count >= 1);
    }

    #[test]
    fn finds_used_by_references() {
        let dir = setup(&[
            ("lib.rs", "pub fn helper() -> i32 {\n    42\n}\n"),
            ("main.rs", "fn main() {\n    let x = helper();\n}\n"),
        ]);
        let result = analyze(dir.path().to_str().unwrap(), "lib.rs", Mode::Full).unwrap();
        assert!(result.used_by_count >= 1);
        assert!(result.plain.contains("## Used By"));
        assert!(result.plain.contains("main.rs"));
    }

    #[test]
    fn map_mode_uses_signatures_label() {
        let dir = setup(&[("main.rs", "fn main() {}\nfn helper() -> i32 { 42 }\n")]);
        let result = analyze(dir.path().to_str().unwrap(), "main.rs", Mode::Map).unwrap();
        assert!(result.plain.contains("## Signatures"));
        assert!(result.plain.contains("[signatures]"));
        assert!(!result.plain.contains("## Source"));
    }

    #[test]
    fn slim_mode_uses_blast_radius_label() {
        let dir = setup(&[("main.rs", "fn main() {}\n")]);
        let result = analyze(dir.path().to_str().unwrap(), "main.rs", Mode::Slim).unwrap();
        assert!(result.plain.contains("[blast radius]"));
        assert!(result.plain.contains("## Source"));
    }

    #[test]
    fn full_mode_includes_raw_source() {
        let src = "fn main() {\n    // this is a comment\n    println!(\"hello\");\n}\n";
        let dir = setup(&[("main.rs", src)]);
        let result = analyze(dir.path().to_str().unwrap(), "main.rs", Mode::Full).unwrap();
        // Full mode preserves comments
        assert!(result.plain.contains("// this is a comment"));
    }

    #[test]
    fn slim_mode_strips_comments() {
        let src = "fn main() {\n    // this is a comment\n    println!(\"hello\");\n}\n";
        let dir = setup(&[("main.rs", src)]);
        let result = analyze(dir.path().to_str().unwrap(), "main.rs", Mode::Slim).unwrap();
        assert!(!result.plain.contains("// this is a comment"));
    }

    #[test]
    fn map_mode_dep_signatures_present() {
        let dir = setup(&[
            ("config.rs", "pub struct Config {\n    pub debug: bool,\n}\n\npub fn load() -> Config {\n    Config { debug: false }\n}\n"),
            ("main.rs", "use crate::config::Config;\n\nfn main() {\n    let _c = Config { debug: true };\n}\n"),
        ]);
        let result = analyze(dir.path().to_str().unwrap(), "main.rs", Mode::Map).unwrap();
        assert!(result.plain.contains("## Dependency Signatures"));
        assert!(result.plain.contains("config.rs"));
    }

    #[test]
    fn slim_mode_dep_sym_tables() {
        let dir = setup(&[
            ("config.rs", "pub struct Config {\n    pub debug: bool,\n}\n"),
            ("main.rs", "use crate::config::Config;\n\nfn main() {\n    let _c = Config { debug: true };\n}\n"),
        ]);
        let result = analyze(dir.path().to_str().unwrap(), "main.rs", Mode::Slim).unwrap();
        // Slim uses compact sym tables, not code blocks
        assert!(result.plain.contains("## Dependencies"));
        assert!(!result.plain.contains("## Dependency Signatures"));
    }

    #[test]
    fn python_imports_resolved() {
        let dir = setup(&[
            ("config.py", "class Config:\n    debug = True\n"),
            ("main.py", "from config import Config\n\ndef run():\n    c = Config()\n"),
        ]);
        let result = analyze(dir.path().to_str().unwrap(), "main.py", Mode::Full).unwrap();
        assert!(result.plain.contains("Config"));
        assert!(result.plain.contains("## Imports"));
    }

    #[test]
    fn short_symbol_names_skipped_in_used_by() {
        // Symbols with names <= 2 chars should not appear in used-by scan
        let dir = setup(&[
            ("lib.rs", "pub fn ab() -> i32 { 1 }\npub fn abc() -> i32 { 2 }\n"),
            ("main.rs", "fn main() {\n    let _ = ab();\n    let _ = abc();\n}\n"),
        ]);
        let result = analyze(dir.path().to_str().unwrap(), "lib.rs", Mode::Full).unwrap();
        // "ab" (2 chars) should be skipped, "abc" (3 chars) should be found
        let used_by_section = result.plain.split("## Used By").nth(1).unwrap_or("");
        assert!(!used_by_section.contains("→ ab (") && !used_by_section.contains("→ ab\n"));
        assert!(used_by_section.contains("→ abc"));
    }

    #[test]
    fn markdown_has_code_fences() {
        let dir = setup(&[("main.rs", "fn main() {}\n")]);
        let result = analyze(dir.path().to_str().unwrap(), "main.rs", Mode::Full).unwrap();
        assert!(result.plain.contains("```rust"));
        assert!(result.plain.contains("```\n"));
    }

    #[test]
    fn python_gets_python_fence() {
        let dir = setup(&[("app.py", "def main():\n    pass\n")]);
        let result = analyze(dir.path().to_str().unwrap(), "app.py", Mode::Full).unwrap();
        assert!(result.plain.contains("```python"));
    }
}
