use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Result, bail};
use ignore::WalkBuilder;
use regex::Regex;

use crate::compress;
use crate::compress::Mode;
use crate::symbol::{self, SymbolKind};
use crate::tree;
use crate::why;

pub struct ContextResult {
    pub plain: String,
    pub file_count: usize,
    pub total_bytes: usize,
    pub total_lines: usize,
    /// Original bytes before compression (same as total_bytes in Full mode)
    pub original_bytes: usize,
}

pub fn generate_context(
    paths: &[String],
    depth: usize,
    regex: Option<&str>,
    mode: Mode,
) -> Result<ContextResult> {
    let re = regex.map(Regex::new).transpose()?;

    // Collect all file paths to read
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

    // Concurrent reads: (path, compressed_content, original_content)
    let results: Mutex<Vec<(String, String, String)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for fp in &file_paths {
            let results = &results;
            let fp = fp.clone();
            s.spawn(move || {
                if let Ok(content) = std::fs::read_to_string(&fp) {
                    let compressed = compress::compress(&content, &fp, mode);
                    results.lock().unwrap().push((fp, compressed, content));
                }
            });
        }
    });

    // Sort by original order
    let mut read_files = results.into_inner().unwrap();
    let order: std::collections::HashMap<&str, usize> = file_paths
        .iter()
        .enumerate()
        .map(|(i, p)| (p.as_str(), i))
        .collect();
    read_files.sort_by_key(|(p, _, _)| order.get(p.as_str()).copied().unwrap_or(usize::MAX));

    let file_count = read_files.len();
    let original_bytes: usize = read_files.iter().map(|(_, _, orig)| orig.len()).sum();
    let total_bytes: usize = read_files.iter().map(|(_, c, _)| c.len()).sum();
    let total_lines: usize = read_files.iter().map(|(_, c, _)| c.lines().count()).sum();

    // Assemble plain output
    let mut plain = String::new();
    plain.push_str("CONTEXT FOR LLM\n");
    plain.push_str("================================\n");

    match mode {
        Mode::Slim => {
            plain.push_str("NOTE: This context was generated in SLIM mode.\n");
            plain.push_str("Comments have been stripped and blank lines collapsed.\n");
            plain.push_str("All code is intact — only documentation artifacts were removed.\n\n");
        }
        Mode::Map => {
            plain.push_str("NOTE: This context was generated in MAP mode.\n");
            plain.push_str(
                "Only imports, type definitions, and function/method signatures are shown.\n",
            );
            plain.push_str(
                "Function bodies have been replaced with { ... } (or : ... for Python).\n",
            );
            plain.push_str("Request the full file if you need implementation details.\n\n");
        }
        Mode::Full => {}
    }

    // Directory/file listing header
    for dir in &dir_paths {
        plain.push_str(&format!("Directory: {}\n", dir));
        if let Ok(tree_result) = tree::build_tree(dir, Some(depth), regex, None) {
            plain.push_str(&tree_result.plain);
            if !tree_result.plain.ends_with('\n') {
                plain.push('\n');
            }
        }
    }
    if !individual_files.is_empty() {
        plain.push_str("Files:\n");
        for f in &individual_files {
            plain.push_str(&format!("  {}\n", f));
        }
    }

    plain.push_str("\n--- FILE CONTENTS ---\n");
    plain.push_str("<documents>\n");

    for (i, (path, content, _)) in read_files.iter().enumerate() {
        plain.push_str(&format!("<document index=\"{}\">\n", i + 1));
        plain.push_str(&format!("<source>{}</source>\n", path));
        plain.push_str("<document_content>\n");

        let lines: Vec<&str> = content.lines().collect();
        let width = if lines.is_empty() {
            1
        } else {
            lines.len().to_string().len()
        };
        for (line_num, line) in lines.iter().enumerate() {
            plain.push_str(&format!(
                "{:>width$}  {}\n",
                line_num + 1,
                line,
                width = width
            ));
        }

        plain.push_str("</document_content>\n");
        plain.push_str("</document>\n");
    }

    plain.push_str("</documents>\n");

    // Symbol index + dependency analysis
    {
        let included: std::collections::HashSet<&str> =
            read_files.iter().map(|(p, _, _)| p.as_str()).collect();

        // Try to find a project root for symbol loading
        let root = dir_paths
            .first()
            .map(|d| d.as_str())
            .unwrap_or(".");
        let root_path = std::fs::canonicalize(root).unwrap_or_else(|_| Path::new(root).to_path_buf());
        let all_symbols = symbol::load_symbols(&root_path);

        // Symbols defined in the included files
        let included_symbols: Vec<_> = all_symbols
            .iter()
            .filter(|s| {
                s.kind != SymbolKind::File
                    && included.iter().any(|inc| {
                        inc.ends_with(&s.file) || s.file.ends_with(inc)
                    })
            })
            .collect();

        if !included_symbols.is_empty() {
            plain.push_str("\n--- SYMBOL INDEX ---\n");

            // Group by file
            let mut by_file: HashMap<&str, Vec<_>> = HashMap::new();
            for sym in &included_symbols {
                by_file.entry(sym.file.as_str()).or_default().push(*sym);
            }
            let mut files: Vec<_> = by_file.into_iter().collect();
            files.sort_by_key(|(f, _)| *f);

            for (file, syms) in &files {
                plain.push_str(&format!("\n## {}\n", file));
                for sym in syms {
                    let display = if let Some(ref parent) = sym.parent {
                        format!("{}::{}", parent, sym.name)
                    } else {
                        sym.name.clone()
                    };
                    if !sym.signature.is_empty() {
                        plain.push_str(&format!(
                            "  [{}] {}  {}\n",
                            sym.kind.tag(),
                            display,
                            sym.signature
                        ));
                    } else {
                        plain.push_str(&format!("  [{}] {}\n", sym.kind.tag(), display));
                    }
                }
            }
        }

        // Hierarchy: inheritance / implements for classes, structs, traits
        let mut hierarchy_entries: Vec<(String, String)> = Vec::new();
        for (path, _, original) in &read_files {
            let rel = path.as_str();
            let imports = why::extract_file_imports(original, rel, &root_path);

            // Check each class/struct/trait symbol in this file
            let file_syms: Vec<_> = included_symbols
                .iter()
                .filter(|s| {
                    matches!(
                        s.kind,
                        SymbolKind::Class
                            | SymbolKind::Struct
                            | SymbolKind::Trait
                            | SymbolKind::Interface
                    ) && (path.ends_with(&s.file) || s.file.ends_with(rel))
                })
                .collect();

            for sym in &file_syms {
                if let Some(h) = why::extract_hierarchy(
                    &root_path,
                    sym,
                    original,
                    &all_symbols,
                    &imports,
                ) {
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
                            entry.push_str(&format!("{}\n", l));
                        }
                        hierarchy_entries.push((path.clone(), entry));
                    }
                }
            }
        }

        if !hierarchy_entries.is_empty() {
            plain.push_str("\n--- HIERARCHY ---\n");
            let mut current_file = "";
            for (file, entry) in &hierarchy_entries {
                if file.as_str() != current_file {
                    plain.push_str(&format!("\n## {}\n", file));
                    current_file = file;
                }
                plain.push_str(entry);
            }
        }

        // Import / dependency analysis per included file
        #[allow(clippy::type_complexity)]
        let mut all_imports: Vec<(String, Vec<(String, String, Option<String>)>)> = Vec::new();
        for (path, _, original) in &read_files {
            let rel = path.as_str();
            let imports = why::extract_file_imports(original, rel, &root_path);
            if imports.is_empty() {
                continue;
            }

            let mut resolved: Vec<(String, String, Option<String>)> = Vec::new();
            for (name, module) in &imports {
                if let Some(sym) = all_symbols.iter().find(|s| s.name == *name && s.file != rel) {
                    resolved.push((
                        name.clone(),
                        format!("{}:{}", sym.file, sym.line),
                        Some(sym.kind.tag().to_string()),
                    ));
                } else if let Some(resolved_file) =
                    why::resolve_relative_import(module, rel, &root_path)
                {
                    resolved.push((name.clone(), resolved_file, None));
                } else {
                    resolved.push((name.clone(), format!("{} (external)", module), None));
                }
            }
            resolved.sort_by(|a, b| a.0.cmp(&b.0));
            all_imports.push((path.clone(), resolved));
        }

        if !all_imports.is_empty() {
            plain.push_str("\n--- DEPENDENCIES ---\n");
            for (file, imports) in &all_imports {
                plain.push_str(&format!("\n## {}\n", file));
                let max_name = imports.iter().map(|(n, _, _)| n.len()).max().unwrap_or(0);
                for (name, loc, kind) in imports {
                    let tag = kind
                        .as_ref()
                        .map(|k| format!(" [{}]", k))
                        .unwrap_or_default();
                    plain.push_str(&format!(
                        "  {:<width$} → {}{}\n",
                        name,
                        loc,
                        tag,
                        width = max_name
                    ));
                }
            }
        }
    }

    Ok(ContextResult {
        plain,
        file_count,
        total_bytes,
        total_lines,
        original_bytes,
    })
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
    fn single_file_context() {
        let dir = setup(&[("hello.txt", "hello world")]);
        let file = dir.path().join("hello.txt");
        let result =
            generate_context(&[file.to_string_lossy().to_string()], 2, None, Mode::Full).unwrap();
        assert_eq!(result.file_count, 1);
        assert!(result.plain.contains("1  hello world"));
        assert!(result.plain.contains("CONTEXT FOR LLM"));
    }

    #[test]
    fn directory_reads_all_files() {
        let dir = setup(&[("a.txt", "aaa"), ("b.txt", "bbb")]);
        let result = generate_context(
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
        let result = generate_context(
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
        let result = generate_context(
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
        let result = generate_context(
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
        let result =
            generate_context(&[file.to_string_lossy().to_string()], 2, None, Mode::Full).unwrap();
        assert!(result.plain.contains("================================"));
        assert!(result.plain.contains("--- FILE CONTENTS ---"));
        assert!(result.plain.contains("<documents>"));
        assert!(result.plain.contains("<document_content>"));
    }

    #[test]
    fn total_bytes_correct() {
        let dir = setup(&[("a.txt", "12345"), ("b.txt", "67890")]);
        let result = generate_context(
            &[dir.path().to_string_lossy().to_string()],
            2,
            None,
            Mode::Full,
        )
        .unwrap();
        assert_eq!(result.total_bytes, 10);
    }
}
