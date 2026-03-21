use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Result, bail};
use ignore::WalkBuilder;
use regex::Regex;

use crate::compress::{self, Mode};
use crate::symbol::{self, Symbol, SymbolKind};
use crate::tree;
use crate::why;

// ── Result type ─────────────────────────────────────────────────────

pub struct AnalysisResult {
    pub plain: String,
    pub file_count: usize,
    pub total_lines: usize,
    pub total_bytes: usize,
    pub original_bytes: usize,
    pub dep_file_count: usize,
    pub used_by_count: usize,
}

// ── Internal types ──────────────────────────────────────────────────

struct ResolvedImport {
    name: String,
    file: Option<String>,
    line: Option<usize>,
    kind: Option<SymbolKind>,
    module: String,
}

struct UsedByRef {
    file: String,
    line: usize,
    symbol_name: String,
    caller: Option<String>,
}

struct FileData {
    path: String,
    compressed: String,
    original: String,
}

struct FileAnalysis {
    symbols: Vec<Symbol>,
    resolved_imports: Vec<ResolvedImport>,
    hierarchy_entries: Vec<String>,
    used_by: Vec<UsedByRef>,
}

// ── Public API ──────────────────────────────────────────────────────

pub fn analyze(
    root: &str,
    paths: &[String],
    depth: usize,
    regex: Option<&str>,
    mode: Mode,
) -> Result<AnalysisResult> {
    let re = regex.map(Regex::new).transpose()?;

    // 1. Resolve paths
    let mut file_paths: Vec<String> = Vec::new();
    let mut dir_paths: Vec<String> = Vec::new();
    let mut individual_files: Vec<String> = Vec::new();

    for p in paths {
        let path = Path::new(p);
        if !path.exists() {
            bail!("path does not exist: {}", p);
        }
        if path.is_file() {
            if let Some(ref re) = re
                && !re.is_match(p)
            {
                continue;
            }
            file_paths.push(p.clone());
            individual_files.push(p.clone());
        } else if path.is_dir() {
            dir_paths.push(p.clone());
            let walker = WalkBuilder::new(path)
                .sort_by_file_name(|a, b| a.cmp(b))
                .build();
            for entry in walker.flatten() {
                if entry.path().is_file() {
                    let rel = entry.path().to_string_lossy().to_string();
                    if let Some(ref re) = re
                        && !re.is_match(&rel)
                    {
                        continue;
                    }
                    file_paths.push(rel);
                }
            }
        }
    }

    if file_paths.is_empty() {
        bail!("no files matched the given paths/filters");
    }

    // 2. Concurrent reads
    let results: Mutex<Vec<FileData>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for fp in &file_paths {
            let results = &results;
            let fp = fp.clone();
            s.spawn(move || {
                if let Ok(content) = std::fs::read_to_string(&fp) {
                    let compressed = compress::compress(&content, &fp, mode);
                    results.lock().unwrap().push(FileData {
                        path: fp,
                        compressed,
                        original: content,
                    });
                }
            });
        }
    });

    let mut read_files = results.into_inner().unwrap();
    let order: HashMap<&str, usize> = file_paths
        .iter()
        .enumerate()
        .map(|(i, p)| (p.as_str(), i))
        .collect();
    read_files.sort_by_key(|f| order.get(f.path.as_str()).copied().unwrap_or(usize::MAX));

    let file_count = read_files.len();
    let original_bytes: usize = read_files.iter().map(|f| f.original.len()).sum();
    let total_bytes: usize = read_files.iter().map(|f| f.compressed.len()).sum();
    let total_lines: usize = read_files
        .iter()
        .map(|f| f.compressed.lines().count())
        .sum();

    // 3. Load project symbol index once
    let root_path = if let Some(dir) = dir_paths.first() {
        std::fs::canonicalize(dir).unwrap_or_else(|_| Path::new(dir).to_path_buf())
    } else {
        std::fs::canonicalize(root).unwrap_or_else(|_| Path::new(root).to_path_buf())
    };
    let all_symbols = symbol::load_symbols(&root_path);

    // 4. Per-file analysis
    let file_set: std::collections::HashSet<&str> =
        read_files.iter().map(|f| f.path.as_str()).collect();

    let mut analyses: Vec<FileAnalysis> = Vec::new();
    let mut total_dep_files = 0usize;
    let mut total_used_by = 0usize;

    for fd in &read_files {
        let rel = fd.path.as_str();

        // Symbols in this file
        let symbols: Vec<Symbol> = all_symbols
            .iter()
            .filter(|s| {
                s.kind != SymbolKind::File
                    && (s.file == rel || rel.ends_with(&s.file) || s.file.ends_with(rel))
            })
            .cloned()
            .collect();

        // Import resolution
        let imports = why::extract_file_imports(&fd.original, rel, &root_path);
        let mut resolved: Vec<ResolvedImport> = Vec::new();
        let mut dep_file_set: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (name, module) in &imports {
            if let Some(sym) = all_symbols
                .iter()
                .find(|s| s.name == *name && s.file != rel)
            {
                dep_file_set.insert(sym.file.clone());
                resolved.push(ResolvedImport {
                    name: name.clone(),
                    file: Some(sym.file.clone()),
                    line: Some(sym.line),
                    kind: Some(sym.kind),
                    module: module.clone(),
                });
            } else if let Some(resolved_file) =
                why::resolve_relative_import(module, rel, &root_path)
            {
                dep_file_set.insert(resolved_file.clone());
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
        total_dep_files += dep_file_set.len();

        // Hierarchy
        let mut hierarchy_entries = Vec::new();
        for sym in &symbols {
            if !matches!(
                sym.kind,
                SymbolKind::Class | SymbolKind::Struct | SymbolKind::Trait | SymbolKind::Interface
            ) {
                continue;
            }
            if let Some(h) =
                why::extract_hierarchy(&root_path, sym, &fd.original, &all_symbols, &imports)
            {
                let mut lines = Vec::new();
                for p in &h.parents {
                    let loc = if let Some((ref file, line)) = p.location {
                        format!("{}:{}", file, line)
                    } else {
                        p.external_module
                            .as_ref()
                            .map(|m| format!("({} — external)", m))
                            .unwrap_or_else(|| "(external)".to_string())
                    };
                    lines.push(format!("  ^ {}  {}", p.name, loc));
                }
                for c in &h.children {
                    let loc = if let Some((ref file, line)) = c.location {
                        format!("{}:{}", file, line)
                    } else {
                        "(external)".to_string()
                    };
                    lines.push(format!("  v {}  {}", c.name, loc));
                }
                if !lines.is_empty() {
                    let display = if let Some(ref parent) = sym.parent {
                        format!("{}::{}", parent, sym.name)
                    } else {
                        sym.name.clone()
                    };
                    let mut entry = format!("[{}] {}\n", sym.kind.tag(), display);
                    for l in &lines {
                        entry.push_str(l);
                        entry.push('\n');
                    }
                    hierarchy_entries.push(entry);
                }
            }
        }

        // Used-by (only for small file sets to avoid O(n*m) explosion)
        let used_by = if file_count <= 20 {
            let sym_names: Vec<&str> = symbols
                .iter()
                .filter(|s| s.name.len() > 2)
                .map(|s| s.name.as_str())
                .collect();
            let refs = find_file_references(&root_path, rel, &sym_names, &file_set);
            total_used_by += refs.len();
            refs
        } else {
            Vec::new()
        };

        analyses.push(FileAnalysis {
            symbols,
            resolved_imports: resolved,
            hierarchy_entries,
            used_by,
        });
    }

    // 5. Render
    let plain = render(
        &read_files,
        &analyses,
        &all_symbols,
        mode,
        depth,
        regex,
        &dir_paths,
        &individual_files,
    );

    Ok(AnalysisResult {
        plain,
        file_count,
        total_lines,
        total_bytes,
        original_bytes,
        dep_file_count: total_dep_files,
        used_by_count: total_used_by,
    })
}

// ── Used-by scan ────────────────────────────────────────────────────

fn find_file_references(
    root: &Path,
    target_file: &str,
    symbol_names: &[&str],
    skip_files: &std::collections::HashSet<&str>,
) -> Vec<UsedByRef> {
    if symbol_names.is_empty() {
        return Vec::new();
    }

    let mut refs = Vec::new();
    let walker = WalkBuilder::new(root)
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

        if rel == target_file {
            continue;
        }

        // Skip files already in the analysis set (they have their own context)
        if skip_files
            .iter()
            .any(|f| f.ends_with(&rel) || rel.ends_with(*f))
        {
            continue;
        }

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
                    let caller = tree
                        .as_ref()
                        .and_then(|t| why::find_enclosing_function(t, &content, line_idx));

                    refs.push(UsedByRef {
                        file: rel.clone(),
                        line: line_idx + 1,
                        symbol_name: sym_name.to_string(),
                        caller,
                    });
                    break;
                }
            }
        }
    }

    refs.truncate(30);
    refs
}

// ── Renderer ────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render(
    files: &[FileData],
    analyses: &[FileAnalysis],
    _all_symbols: &[Symbol],
    mode: Mode,
    depth: usize,
    regex: Option<&str>,
    dir_paths: &[String],
    individual_files: &[String],
) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    // Header
    out.push_str("CONTEXT FOR LLM\n");
    out.push_str("================================\n");

    match mode {
        Mode::Slim => {
            out.push_str("NOTE: This context was generated in SLIM mode.\n");
            out.push_str("Comments have been stripped and blank lines collapsed.\n");
            out.push_str("All code is intact — only documentation artifacts were removed.\n\n");
        }
        Mode::Map => {
            out.push_str("NOTE: This context was generated in MAP mode.\n");
            out.push_str(
                "Only imports, type definitions, and function/method signatures are shown.\n",
            );
            out.push_str(
                "Function bodies have been replaced with { ... } (or : ... for Python).\n",
            );
            out.push_str("Request the full file if you need implementation details.\n\n");
        }
        Mode::Full => {}
    }

    // Directory tree listing
    for dir in dir_paths {
        let _ = writeln!(out, "Directory: {}", dir);
        if let Ok(tree_result) = tree::build_tree(dir, Some(depth), regex, None) {
            out.push_str(&tree_result.plain);
            if !tree_result.plain.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    if !individual_files.is_empty() {
        out.push_str("Files:\n");
        for f in individual_files {
            let _ = writeln!(out, "  {}", f);
        }
    }

    // File contents as XML documents
    out.push_str("\n--- FILE CONTENTS ---\n");
    out.push_str("<documents>\n");

    for (i, fd) in files.iter().enumerate() {
        let _ = writeln!(out, "<document index=\"{}\">", i + 1);
        let _ = writeln!(out, "<source>{}</source>", fd.path);
        out.push_str("<document_content>\n");

        let lines: Vec<&str> = fd.compressed.lines().collect();
        let width = if lines.is_empty() {
            1
        } else {
            lines.len().to_string().len()
        };
        for (line_num, line) in lines.iter().enumerate() {
            let _ = writeln!(out, "{:>width$}  {}", line_num + 1, line, width = width);
        }

        out.push_str("</document_content>\n");
        out.push_str("</document>\n");
    }

    out.push_str("</documents>\n");

    // Symbol index
    let has_symbols = analyses.iter().any(|a| !a.symbols.is_empty());
    if has_symbols {
        out.push_str("\n--- SYMBOL INDEX ---\n");
        for (fd, analysis) in files.iter().zip(analyses.iter()) {
            if analysis.symbols.is_empty() {
                continue;
            }
            let _ = writeln!(out, "\n## {}", fd.path);
            for sym in &analysis.symbols {
                let display = if let Some(ref parent) = sym.parent {
                    format!("{}::{}", parent, sym.name)
                } else {
                    sym.name.clone()
                };
                if !sym.signature.is_empty() {
                    let _ = writeln!(out, "  [{}] {}  {}", sym.kind.tag(), display, sym.signature);
                } else {
                    let _ = writeln!(out, "  [{}] {}", sym.kind.tag(), display);
                }
            }
        }
    }

    // Hierarchy
    let has_hierarchy = analyses.iter().any(|a| !a.hierarchy_entries.is_empty());
    if has_hierarchy {
        out.push_str("\n--- HIERARCHY ---\n");
        for (fd, analysis) in files.iter().zip(analyses.iter()) {
            if analysis.hierarchy_entries.is_empty() {
                continue;
            }
            let _ = writeln!(out, "\n## {}", fd.path);
            for entry in &analysis.hierarchy_entries {
                out.push_str(entry);
            }
        }
    }

    // Dependencies
    let has_deps = analyses.iter().any(|a| !a.resolved_imports.is_empty());
    if has_deps {
        out.push_str("\n--- DEPENDENCIES ---\n");
        for (fd, analysis) in files.iter().zip(analyses.iter()) {
            if analysis.resolved_imports.is_empty() {
                continue;
            }
            let _ = writeln!(out, "\n## {}", fd.path);
            let max_name = analysis
                .resolved_imports
                .iter()
                .map(|r| r.name.len())
                .max()
                .unwrap_or(0);
            for imp in &analysis.resolved_imports {
                let loc = if let Some(ref file) = imp.file {
                    if let Some(line) = imp.line {
                        format!("{}:{}", file, line)
                    } else {
                        file.clone()
                    }
                } else {
                    format!("{} (external)", imp.module)
                };
                let tag = imp
                    .kind
                    .map(|k| format!(" [{}]", k.tag()))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "  {:<width$} → {}{}",
                    imp.name,
                    loc,
                    tag,
                    width = max_name
                );
            }
        }
    }

    // Used by
    let has_used_by = analyses.iter().any(|a| !a.used_by.is_empty());
    if has_used_by {
        out.push_str("\n--- USED BY ---\n");
        out.push_str("(Imprecise: text-based search — may include false positives from common names)\n");
        for (fd, analysis) in files.iter().zip(analyses.iter()) {
            if analysis.used_by.is_empty() {
                continue;
            }
            let _ = writeln!(out, "\n## {}", fd.path);
            for r in &analysis.used_by {
                let caller_str = r
                    .caller
                    .as_ref()
                    .map(|c| format!(" (in {})", c))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "  {}:{} → {}{}",
                    r.file, r.line, r.symbol_name, caller_str
                );
            }
        }
    }

    out
}

// ── Tests ───────────────────────────────────────────────────────────

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

    fn analyze_one(root: &str, file: &str, mode: Mode) -> Result<AnalysisResult> {
        let full = Path::new(root).join(file).to_string_lossy().to_string();
        analyze(root, &[full], 2, None, mode)
    }

    // ── Single-file tests (migrated from old ctx.rs) ────────────

    #[test]
    fn nonexistent_file_errors() {
        let dir = setup(&[]);
        let result = analyze_one(dir.path().to_str().unwrap(), "nope.rs", Mode::Full);
        assert!(result.is_err());
    }

    #[test]
    fn single_file_no_deps() {
        let dir = setup(&[("main.rs", "fn main() {\n    println!(\"hello\");\n}\n")]);
        let result = analyze_one(dir.path().to_str().unwrap(), "main.rs", Mode::Full).unwrap();
        assert_eq!(result.file_count, 1);
        assert_eq!(result.dep_file_count, 0);
        assert_eq!(result.used_by_count, 0);
        assert!(result.plain.contains("main.rs</source>"));
        assert!(result.plain.contains("println"));
    }

    #[test]
    fn detects_definitions() {
        let dir = setup(&[(
            "lib.rs",
            "pub struct Foo {\n    pub x: i32,\n}\n\npub fn bar() -> Foo {\n    Foo { x: 1 }\n}\n",
        )]);
        let result = analyze_one(dir.path().to_str().unwrap(), "lib.rs", Mode::Full).unwrap();
        assert!(result.plain.contains("SYMBOL INDEX"));
        assert!(result.plain.contains("[st] Foo"));
        assert!(result.plain.contains("[fn] bar"));
    }

    #[test]
    fn resolves_rust_imports() {
        let dir = setup(&[
            (
                "config.rs",
                "pub struct Config {\n    pub debug: bool,\n}\n",
            ),
            (
                "main.rs",
                "use crate::config::Config;\n\nfn main() {\n    let _c = Config { debug: true };\n}\n",
            ),
        ]);
        let result = analyze_one(dir.path().to_str().unwrap(), "main.rs", Mode::Full).unwrap();
        assert!(result.plain.contains("DEPENDENCIES"));
        assert!(result.plain.contains("Config"));
        assert!(result.dep_file_count >= 1);
    }

    #[test]
    fn finds_used_by_references() {
        let dir = setup(&[
            ("lib.rs", "pub fn helper() -> i32 {\n    42\n}\n"),
            ("main.rs", "fn main() {\n    let x = helper();\n}\n"),
        ]);
        let result = analyze_one(dir.path().to_str().unwrap(), "lib.rs", Mode::Full).unwrap();
        assert!(result.used_by_count >= 1);
        assert!(result.plain.contains("USED BY"));
        assert!(result.plain.contains("main.rs"));
    }

    #[test]
    fn map_mode_note() {
        let dir = setup(&[("main.rs", "fn main() {}\nfn helper() -> i32 { 42 }\n")]);
        let result = analyze_one(dir.path().to_str().unwrap(), "main.rs", Mode::Map).unwrap();
        assert!(result.plain.contains("MAP mode"));
    }

    #[test]
    fn slim_mode_note() {
        let dir = setup(&[("main.rs", "fn main() {}\n")]);
        let result = analyze_one(dir.path().to_str().unwrap(), "main.rs", Mode::Slim).unwrap();
        assert!(result.plain.contains("SLIM mode"));
    }

    #[test]
    fn full_mode_includes_raw_source() {
        let src = "fn main() {\n    // this is a comment\n    println!(\"hello\");\n}\n";
        let dir = setup(&[("main.rs", src)]);
        let result = analyze_one(dir.path().to_str().unwrap(), "main.rs", Mode::Full).unwrap();
        assert!(result.plain.contains("// this is a comment"));
    }

    #[test]
    fn slim_mode_strips_comments() {
        let src = "fn main() {\n    // this is a comment\n    println!(\"hello\");\n}\n";
        let dir = setup(&[("main.rs", src)]);
        let result = analyze_one(dir.path().to_str().unwrap(), "main.rs", Mode::Slim).unwrap();
        assert!(!result.plain.contains("// this is a comment"));
    }

    #[test]
    fn python_imports_resolved() {
        let dir = setup(&[
            ("config.py", "class Config:\n    debug = True\n"),
            (
                "main.py",
                "from config import Config\n\ndef run():\n    c = Config()\n",
            ),
        ]);
        let result = analyze_one(dir.path().to_str().unwrap(), "main.py", Mode::Full).unwrap();
        assert!(result.plain.contains("Config"));
        assert!(result.plain.contains("DEPENDENCIES"));
    }

    #[test]
    fn short_symbol_names_skipped_in_used_by() {
        let dir = setup(&[
            (
                "lib.rs",
                "pub fn ab() -> i32 { 1 }\npub fn abc() -> i32 { 2 }\n",
            ),
            (
                "main.rs",
                "fn main() {\n    let _ = ab();\n    let _ = abc();\n}\n",
            ),
        ]);
        let result = analyze_one(dir.path().to_str().unwrap(), "lib.rs", Mode::Full).unwrap();
        let used_by_section = result.plain.split("USED BY").nth(1).unwrap_or("");
        assert!(!used_by_section.contains("→ ab (") && !used_by_section.contains("→ ab\n"));
        assert!(used_by_section.contains("→ abc"));
    }

    // ── Multi-file tests (migrated from old context.rs) ─────────

    #[test]
    fn single_file_context() {
        let dir = setup(&[("hello.txt", "hello world")]);
        let file = dir.path().join("hello.txt");
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[file.to_string_lossy().to_string()],
            2,
            None,
            Mode::Full,
        )
        .unwrap();
        assert_eq!(result.file_count, 1);
        assert!(result.plain.contains("1  hello world"));
        assert!(result.plain.contains("CONTEXT FOR LLM"));
    }

    #[test]
    fn directory_reads_all_files() {
        let dir = setup(&[("a.txt", "aaa"), ("b.txt", "bbb")]);
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[dir.path().to_string_lossy().to_string()],
            2,
            None,
            Mode::Full,
        )
        .unwrap();
        assert_eq!(result.file_count, 2);
        assert!(result.plain.contains("aaa"));
        assert!(result.plain.contains("bbb"));
    }

    #[test]
    fn regex_filters_files() {
        let dir = setup(&[("main.rs", "fn main()"), ("readme.md", "# Readme")]);
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[dir.path().to_string_lossy().to_string()],
            2,
            Some(r"\.rs$"),
            Mode::Full,
        )
        .unwrap();
        assert_eq!(result.file_count, 1);
        assert!(result.plain.contains("fn main()"));
        assert!(!result.plain.contains("# Readme"));
    }

    #[test]
    fn nonexistent_path_errors() {
        let result = analyze(
            ".",
            &["/tmp/nonexistent_supp_test_path".to_string()],
            2,
            None,
            Mode::Full,
        );
        assert!(result.is_err());
    }

    #[test]
    fn no_matching_files_errors() {
        let dir = setup(&[("readme.md", "hi")]);
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[dir.path().to_string_lossy().to_string()],
            2,
            Some(r"\.rs$"),
            Mode::Full,
        );
        assert!(result.is_err());
    }

    #[test]
    fn output_contains_metadata() {
        let dir = setup(&[("test.txt", "content")]);
        let file = dir.path().join("test.txt");
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[file.to_string_lossy().to_string()],
            2,
            None,
            Mode::Full,
        )
        .unwrap();
        assert!(result.plain.contains("================================"));
        assert!(result.plain.contains("--- FILE CONTENTS ---"));
        assert!(result.plain.contains("<documents>"));
        assert!(result.plain.contains("<document_content>"));
    }

    #[test]
    fn total_bytes_correct() {
        let dir = setup(&[("a.txt", "12345"), ("b.txt", "67890")]);
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[dir.path().to_string_lossy().to_string()],
            2,
            None,
            Mode::Full,
        )
        .unwrap();
        assert_eq!(result.total_bytes, 10);
    }
}
