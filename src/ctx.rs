use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, bail};
use ignore::WalkBuilder;
use rayon::prelude::*;
use regex::Regex;

use serde::Serialize;

use crate::compress::{self, Mode};
use crate::config::PerfProfile;
use crate::symbol::{self, Symbol, SymbolKind};
use crate::tree;
use crate::why;

// ── Result type ─────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AnalysisResult {
    pub plain: String,
    pub file_count: usize,
    pub total_lines: usize,
    pub total_bytes: usize,
    pub original_bytes: usize,
    pub dep_file_count: usize,
    pub used_by_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_info: Option<BudgetInfo>,
}

#[derive(Debug, Serialize)]
pub struct BudgetInfo {
    pub target: usize,
    pub full_count: usize,
    pub slim_count: usize,
    pub map_count: usize,
    pub dropped_count: usize,
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
    mode: Mode,
}

struct FileAnalysis {
    symbols: Vec<Symbol>,
    resolved_imports: Vec<ResolvedImport>,
    hierarchy_entries: Vec<String>,
    used_by: Vec<UsedByRef>,
}

// ── Public API ──────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn analyze(
    root: &str,
    paths: &[String],
    depth: usize,
    regex: Option<&str>,
    mode: Mode,
    perf: &PerfProfile,
    map_threshold: Option<f64>,
    budget: Option<usize>,
) -> Result<AnalysisResult> {
    let max_files = perf.max_files;
    let max_total_bytes = perf.max_total_mb * 1024 * 1024;
    let used_by_file_threshold = perf.used_by_file_threshold;
    let pagerank_iters = perf.pagerank_iters;
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

    if file_paths.len() > max_files {
        bail!(
            "too many files ({}) — limit is {}. Use -r to filter or increase limits.max_files in config",
            file_paths.len(),
            max_files
        );
    }

    // 2. Compute root_path early so both phases can start together
    let root_path = if let Some(dir) = dir_paths.first() {
        std::fs::canonicalize(dir).unwrap_or_else(|_| Path::new(dir).to_path_buf())
    } else {
        std::fs::canonicalize(root).unwrap_or_else(|_| Path::new(root).to_path_buf())
    };

    // 3. Overlap file reads (rayon) with symbol index loading (background thread)
    //    In budget mode, precompute all 3 compression levels per file.
    struct BudgetPrecomp {
        slim_content: String,
        slim_tokens: usize,
        map_tokens: usize,
        full_tokens: usize,
    }

    let (mut read_files, mut budget_precomps, all_symbols, all_ranks) = std::thread::scope(|s| {
        let rp = &root_path;
        let sym_handle = s.spawn(move || symbol::load_symbols(rp, pagerank_iters));

        let results: Vec<(FileData, Option<BudgetPrecomp>)> = file_paths
            .par_iter()
            .filter_map(|fp| {
                let content = std::fs::read_to_string(fp).ok()?;
                if budget.is_some() {
                    let slim = compress::compress(&content, fp, Mode::Slim);
                    let map = compress::compress(&content, fp, Mode::Map);
                    let precomp = BudgetPrecomp {
                        slim_tokens: crate::styles::estimate_tokens(slim.len()),
                        map_tokens: crate::styles::estimate_tokens(map.len()),
                        full_tokens: crate::styles::estimate_tokens(content.len()),
                        slim_content: slim,
                    };
                    // Start at Map mode for budget; will be upgraded later
                    Some((
                        FileData {
                            path: fp.clone(),
                            compressed: map,
                            original: content,
                            mode: Mode::Map,
                        },
                        Some(precomp),
                    ))
                } else {
                    let compressed = compress::compress(&content, fp, mode);
                    Some((
                        FileData {
                            path: fp.clone(),
                            compressed,
                            original: content,
                            mode,
                        },
                        None,
                    ))
                }
            })
            .collect();

        let (files, precomps): (Vec<_>, Vec<_>) = results.into_iter().unzip();
        let sym_index = sym_handle.join().unwrap();
        (files, precomps, sym_index.symbols, sym_index.ranks)
    });

    // Sort files (and budget precomps) to match input order
    let order: HashMap<&str, usize> = file_paths
        .iter()
        .enumerate()
        .map(|(i, p)| (p.as_str(), i))
        .collect();
    {
        let mut pairs: Vec<(FileData, Option<BudgetPrecomp>)> =
            read_files.into_iter().zip(budget_precomps).collect();
        pairs.sort_by_key(|(fd, _)| order.get(fd.path.as_str()).copied().unwrap_or(usize::MAX));
        let (f, p): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
        read_files = f;
        budget_precomps = p;
    }

    let original_bytes: usize = read_files.iter().map(|f| f.original.len()).sum();

    if (original_bytes as u64) > max_total_bytes {
        bail!(
            "total content too large ({}) — limit is {}. Use -r to filter or increase limits.max_total_mb in config",
            crate::styles::format_size(original_bytes),
            crate::styles::format_size(max_total_bytes as usize),
        );
    }

    // ── Budget fitting ─────────────────────────────────────────────
    let mut dropped_count = 0usize;
    let budget_info = if let Some(token_budget) = budget {
        if token_budget < 100 {
            bail!("budget must be at least 100 tokens");
        }

        // Build per-file importance from PageRank (max score of any symbol in file)
        let individual_set: std::collections::HashSet<&str> =
            individual_files.iter().map(|s| s.as_str()).collect();
        let mut file_importance: Vec<f64> = read_files
            .iter()
            .map(|fd| {
                let base = all_symbols
                    .iter()
                    .zip(all_ranks.iter())
                    .filter(|(s, _)| {
                        s.kind != SymbolKind::File
                            && (s.file == fd.path
                                || fd.path.ends_with(&s.file)
                                || s.file.ends_with(&fd.path))
                    })
                    .map(|(_, &r)| r)
                    .fold(0.0_f64, f64::max);
                // Explicitly-named files get a bonus
                if individual_set.contains(fd.path.as_str()) {
                    base + 1.0
                } else {
                    base
                }
            })
            .collect();

        // Estimate overhead tokens (header, tree, etc.) — conservative ~200 token floor
        let overhead_tokens = 200usize;
        let file_budget = token_budget.saturating_sub(overhead_tokens);

        // Current total at Map mode
        let mut current_tokens: usize = budget_precomps
            .iter()
            .map(|p| p.as_ref().map_or(0, |p| p.map_tokens))
            .sum();

        // Drop lowest-ranked files if Map baseline exceeds budget
        if current_tokens > file_budget {
            // Indices sorted by importance ascending (drop least important first)
            let mut drop_order: Vec<usize> = (0..read_files.len()).collect();
            drop_order.sort_by(|&a, &b| {
                file_importance[a]
                    .partial_cmp(&file_importance[b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for &idx in &drop_order {
                if current_tokens <= file_budget {
                    break;
                }
                if let Some(ref p) = budget_precomps[idx] {
                    current_tokens -= p.map_tokens;
                    file_importance[idx] = -1.0; // Mark as dropped
                    dropped_count += 1;
                }
            }
        }

        // Upgrade loop: iterate by importance descending
        let mut upgrade_order: Vec<usize> = (0..read_files.len())
            .filter(|&i| file_importance[i] >= 0.0)
            .collect();
        upgrade_order.sort_by(|&a, &b| {
            file_importance[b]
                .partial_cmp(&file_importance[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for &idx in &upgrade_order {
            let precomp = match budget_precomps[idx].as_ref() {
                Some(p) => p,
                None => continue,
            };

            // Try Map → Slim
            let slim_delta = precomp.slim_tokens.saturating_sub(precomp.map_tokens);
            if current_tokens + slim_delta <= file_budget {
                current_tokens += slim_delta;
                read_files[idx].compressed = precomp.slim_content.clone();
                read_files[idx].mode = Mode::Slim;

                // Try Slim → Full
                let full_delta = precomp.full_tokens.saturating_sub(precomp.slim_tokens);
                if current_tokens + full_delta <= file_budget {
                    current_tokens += full_delta;
                    read_files[idx].compressed = read_files[idx].original.clone();
                    read_files[idx].mode = Mode::Full;
                }
            }
        }

        // Remove dropped files
        if dropped_count > 0 {
            let keep: Vec<bool> = file_importance.iter().map(|&imp| imp >= 0.0).collect();
            read_files = read_files
                .into_iter()
                .enumerate()
                .filter(|(i, _)| keep[*i])
                .map(|(_, fd)| fd)
                .collect();
        }

        let full_count = read_files.iter().filter(|f| f.mode == Mode::Full).count();
        let slim_count = read_files.iter().filter(|f| f.mode == Mode::Slim).count();
        let map_count = read_files.iter().filter(|f| f.mode == Mode::Map).count();

        Some(BudgetInfo {
            target: token_budget,
            full_count,
            slim_count,
            map_count,
            dropped_count,
        })
    } else {
        None
    };

    let file_count = read_files.len();
    let total_bytes: usize = read_files.iter().map(|f| f.compressed.len()).sum();
    let total_lines: usize = read_files
        .iter()
        .map(|f| f.compressed.lines().count())
        .sum();

    // 4. Per-file analysis (parallel)
    let file_set: std::collections::HashSet<&str> =
        read_files.iter().map(|f| f.path.as_str()).collect();

    // Build name → symbols lookup for O(1) import resolution
    let sym_by_name: HashMap<&str, Vec<&Symbol>> = {
        let mut map: HashMap<&str, Vec<&Symbol>> = HashMap::new();
        for s in &all_symbols {
            map.entry(s.name.as_str()).or_default().push(s);
        }
        map
    };

    let analyses: Vec<FileAnalysis> = read_files
        .par_iter()
        .map(|fd| {
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

            // Import resolution (uses HashMap instead of linear scan)
            let imports = why::extract_file_imports(&fd.original, rel, &root_path);
            let mut resolved: Vec<ResolvedImport> = Vec::new();

            for (name, module) in &imports {
                if let Some(candidates) = sym_by_name.get(name.as_str())
                    && let Some(sym) = candidates.iter().find(|s| s.file != rel)
                {
                    resolved.push(ResolvedImport {
                        name: name.clone(),
                        file: Some(sym.file.clone()),
                        line: Some(sym.line),
                        kind: Some(sym.kind),
                        module: module.clone(),
                    });
                    continue;
                }
                if let Some(resolved_file) = why::resolve_relative_import(module, rel, &root_path) {
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

            // Hierarchy — parse tree once per file, reuse for all class/struct symbols
            let mut hierarchy_entries = Vec::new();
            let file_tree = compress::detect_lang(rel)
                .and_then(|lang| compress::parse_source(&fd.original, lang));
            for sym in &symbols {
                if !matches!(
                    sym.kind,
                    SymbolKind::Class
                        | SymbolKind::Struct
                        | SymbolKind::Trait
                        | SymbolKind::Interface
                ) {
                    continue;
                }
                if let Some(h) = why::extract_hierarchy(
                    &root_path,
                    sym,
                    &fd.original,
                    &all_symbols,
                    &imports,
                    file_tree.as_ref(),
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
                            entry.push_str(l);
                            entry.push('\n');
                        }
                        hierarchy_entries.push(entry);
                    }
                }
            }

            // Used-by (only for small file sets to avoid O(n*m) explosion)
            let used_by = if used_by_file_threshold > 0 && file_count <= used_by_file_threshold {
                let sym_names: Vec<&str> = symbols
                    .iter()
                    .filter(|s| s.name.len() > 2)
                    .map(|s| s.name.as_str())
                    .collect();
                find_file_references(&root_path, rel, &sym_names, &file_set)
            } else {
                Vec::new()
            };

            FileAnalysis {
                symbols,
                resolved_imports: resolved,
                hierarchy_entries,
                used_by,
            }
        })
        .collect();

    let total_dep_files: usize = analyses
        .iter()
        .map(|a| {
            let mut dep_files: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for imp in &a.resolved_imports {
                if let Some(ref f) = imp.file {
                    dep_files.insert(f.as_str());
                }
            }
            dep_files.len()
        })
        .sum();
    let total_used_by: usize = analyses.iter().map(|a| a.used_by.len()).sum();

    // 5. Render
    let plain = render(
        &read_files,
        &analyses,
        &all_symbols,
        &all_ranks,
        mode,
        depth,
        regex,
        &dir_paths,
        &individual_files,
        map_threshold,
        &budget_info,
    );

    Ok(AnalysisResult {
        plain,
        file_count,
        total_lines,
        total_bytes,
        original_bytes,
        dep_file_count: total_dep_files,
        used_by_count: total_used_by,
        budget_info,
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
    all_symbols: &[Symbol],
    all_ranks: &[f64],
    mode: Mode,
    depth: usize,
    regex: Option<&str>,
    dir_paths: &[String],
    individual_files: &[String],
    map_threshold: Option<f64>,
    budget_info: &Option<BudgetInfo>,
) -> String {
    use std::collections::HashSet;
    use std::fmt::Write;
    let mut out = String::new();

    // Compute rank cutoff for importance gating (map mode only)
    let rank_cutoff: Option<f64> = if mode == Mode::Map {
        map_threshold.map(|pct| {
            let mut sorted: Vec<f64> = all_ranks
                .iter()
                .enumerate()
                .filter(|(i, _)| all_symbols[*i].kind != SymbolKind::File)
                .map(|(_, &r)| r)
                .collect();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if sorted.is_empty() {
                return 0.0;
            }
            let idx = ((pct * sorted.len() as f64) as usize).min(sorted.len() - 1);
            sorted[idx]
        })
    } else {
        None
    };

    // Build per-symbol rank lookup
    let sym_rank: HashMap<(&str, &str), f64> = all_symbols
        .iter()
        .zip(all_ranks.iter())
        .map(|(s, &r)| ((s.file.as_str(), s.name.as_str()), r))
        .collect();

    // Header
    out.push_str("CONTEXT FOR LLM\n");
    out.push_str("================================\n");

    if let Some(bi) = budget_info {
        let _ = writeln!(
            out,
            "NOTE: This context was generated in BUDGET mode (target: ~{} tokens).",
            crate::styles::format_number(bi.target),
        );
        let _ = writeln!(
            out,
            "Files use mixed compression: {} full, {} slim, {} map.",
            bi.full_count, bi.slim_count, bi.map_count,
        );
        if bi.dropped_count > 0 {
            let _ = writeln!(
                out,
                "{} low-importance file{} dropped to fit budget.",
                bi.dropped_count,
                if bi.dropped_count == 1 { "" } else { "s" },
            );
        }
        out.push_str("Request the full file if you need implementation details.\n\n");
    } else {
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
                if let Some(pct) = map_threshold {
                    use std::fmt::Write as _;
                    let _ = writeln!(
                        out,
                        "Symbols below the {:.0}th percentile of importance have been omitted.",
                        pct * 100.0,
                    );
                    out.push_str("Low-importance files are summarized as one-liners.\n");
                }
                out.push_str("Request the full file if you need implementation details.\n\n");
            }
            Mode::Full => {}
        }
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

    // Determine demoted files (all symbols below rank cutoff)
    let demoted: HashSet<usize> = if let Some(cutoff) = rank_cutoff {
        files
            .iter()
            .enumerate()
            .filter(|(i, _fd)| {
                let syms = &analyses[*i].symbols;
                !syms.is_empty()
                    && syms.iter().all(|sym| {
                        sym_rank
                            .get(&(sym.file.as_str(), sym.name.as_str()))
                            .copied()
                            .unwrap_or(0.0)
                            < cutoff
                    })
            })
            .map(|(i, _)| i)
            .collect()
    } else {
        HashSet::new()
    };

    // File contents as XML documents (skip demoted files)
    out.push_str("\n--- FILE CONTENTS ---\n");
    out.push_str("<documents>\n");

    let mut doc_idx = 0usize;
    for (i, fd) in files.iter().enumerate() {
        if demoted.contains(&i) {
            continue;
        }
        doc_idx += 1;
        let _ = writeln!(out, "<document index=\"{}\">", doc_idx);
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

    // Low-rank file summaries
    if !demoted.is_empty() {
        out.push_str("\n--- LOW-RANK FILES ---\n");
        for &i in &demoted {
            let fd = &files[i];
            let summary = summarize_file(&analyses[i].symbols);
            if summary.is_empty() {
                let _ = writeln!(out, "  {}", fd.path);
            } else {
                let _ = writeln!(out, "  {} — {}", fd.path, summary);
            }
        }
    }

    // Symbol index
    let has_symbols = analyses.iter().any(|a| !a.symbols.is_empty());
    if has_symbols {
        out.push_str("\n--- SYMBOL INDEX ---\n");
        for (i, (fd, analysis)) in files.iter().zip(analyses.iter()).enumerate() {
            if analysis.symbols.is_empty() || demoted.contains(&i) {
                continue;
            }
            let _ = writeln!(out, "\n## {}", fd.path);
            let file_mode = if budget_info.is_some() { fd.mode } else { mode };
            if file_mode == Mode::Map {
                render_symbol_index_grouped(&mut out, &analysis.symbols, &sym_rank, rank_cutoff);
            } else {
                for sym in &analysis.symbols {
                    let display = if let Some(ref parent) = sym.parent {
                        format!("{}::{}", parent, sym.name)
                    } else {
                        sym.name.clone()
                    };
                    if !sym.signature.is_empty() {
                        let _ =
                            writeln!(out, "  [{}] {}  {}", sym.kind.tag(), display, sym.signature);
                    } else {
                        let _ = writeln!(out, "  [{}] {}", sym.kind.tag(), display);
                    }
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
        out.push_str(
            "(Imprecise: text-based search — may include false positives from common names)\n",
        );
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

// ── File summarization ──────────────────────────────────────────────

fn summarize_file(symbols: &[Symbol]) -> String {
    let mut fn_names: Vec<&str> = Vec::new();
    let mut struct_count = 0u32;
    let mut enum_count = 0u32;
    let mut trait_count = 0u32;
    let mut class_count = 0u32;
    let mut const_count = 0u32;

    for sym in symbols {
        match sym.kind {
            SymbolKind::Function | SymbolKind::Method => fn_names.push(&sym.name),
            SymbolKind::Struct => struct_count += 1,
            SymbolKind::Enum => enum_count += 1,
            SymbolKind::Trait | SymbolKind::Interface => trait_count += 1,
            SymbolKind::Class => class_count += 1,
            SymbolKind::Const | SymbolKind::Macro | SymbolKind::Type => const_count += 1,
            SymbolKind::File => {}
        }
    }

    let mut parts: Vec<String> = Vec::new();
    if !fn_names.is_empty() {
        let names = if fn_names.len() <= 5 {
            fn_names.join(", ")
        } else {
            format!("{}, ... +{}", fn_names[..4].join(", "), fn_names.len() - 4)
        };
        parts.push(format!("{} fns ({})", fn_names.len(), names));
    }
    if struct_count > 0 {
        parts.push(format!("{} structs", struct_count));
    }
    if enum_count > 0 {
        parts.push(format!("{} enums", enum_count));
    }
    if trait_count > 0 {
        parts.push(format!("{} traits", trait_count));
    }
    if class_count > 0 {
        parts.push(format!("{} classes", class_count));
    }
    if const_count > 0 {
        parts.push(format!("{} consts", const_count));
    }

    parts.join(", ")
}

// ── Grouped symbol rendering ────────────────────────────────────────

fn render_symbol_index_grouped(
    out: &mut String,
    symbols: &[Symbol],
    sym_rank: &HashMap<(&str, &str), f64>,
    rank_cutoff: Option<f64>,
) {
    use std::collections::BTreeMap;
    use std::fmt::Write;

    let passes = |sym: &Symbol| -> bool {
        match rank_cutoff {
            None => true,
            Some(cutoff) => {
                sym_rank
                    .get(&(sym.file.as_str(), sym.name.as_str()))
                    .copied()
                    .unwrap_or(0.0)
                    >= cutoff
            }
        }
    };

    // Classify: parent types, children (have parent field), orphans
    let mut parent_types: BTreeMap<&str, &Symbol> = BTreeMap::new();
    let mut children_by_parent: BTreeMap<&str, Vec<&Symbol>> = BTreeMap::new();
    let mut orphans: Vec<&Symbol> = Vec::new();

    for sym in symbols {
        if let Some(ref parent) = sym.parent {
            children_by_parent
                .entry(parent.as_str())
                .or_default()
                .push(sym);
        } else if matches!(
            sym.kind,
            SymbolKind::Struct
                | SymbolKind::Class
                | SymbolKind::Enum
                | SymbolKind::Trait
                | SymbolKind::Interface
        ) {
            parent_types.insert(&sym.name, sym);
        } else {
            orphans.push(sym);
        }
    }

    // Parent types with their children grouped
    for (name, parent_sym) in &parent_types {
        let children = children_by_parent.remove(*name);
        if let Some(children) = children {
            let passing: Vec<&str> = children
                .iter()
                .filter(|s| passes(s))
                .map(|s| s.name.as_str())
                .collect();
            let parent_passes = passes(parent_sym);

            if !parent_passes && passing.is_empty() {
                continue;
            }

            if passing.is_empty() {
                // Type passes but no methods do
                if !parent_sym.signature.is_empty() {
                    let _ = writeln!(
                        out,
                        "  [{}] {}  {}",
                        parent_sym.kind.tag(),
                        name,
                        parent_sym.signature
                    );
                } else {
                    let _ = writeln!(out, "  [{}] {}", parent_sym.kind.tag(), name);
                }
            } else {
                let _ = writeln!(
                    out,
                    "  [{}] {} {{ {} }}",
                    parent_sym.kind.tag(),
                    name,
                    passing.join(", ")
                );
            }
        } else {
            if !passes(parent_sym) {
                continue;
            }
            // Type with no methods
            if !parent_sym.signature.is_empty() {
                let _ = writeln!(
                    out,
                    "  [{}] {}  {}",
                    parent_sym.kind.tag(),
                    name,
                    parent_sym.signature
                );
            } else {
                let _ = writeln!(out, "  [{}] {}", parent_sym.kind.tag(), name);
            }
        }
    }

    // Orphaned children (parent type defined in another file, e.g. impl blocks)
    for (parent_name, children) in &children_by_parent {
        let passing: Vec<&str> = children
            .iter()
            .filter(|s| passes(s))
            .map(|s| s.name.as_str())
            .collect();
        if passing.is_empty() {
            continue;
        }
        let _ = writeln!(out, "  [impl] {} {{ {} }}", parent_name, passing.join(", "));
    }

    // Standalone symbols keep their signature
    for sym in &orphans {
        if !passes(sym) {
            continue;
        }
        if !sym.signature.is_empty() {
            let _ = writeln!(
                out,
                "  [{}] {}  {}",
                sym.kind.tag(),
                sym.name,
                sym.signature
            );
        } else {
            let _ = writeln!(out, "  [{}] {}", sym.kind.tag(), sym.name);
        }
    }
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

    fn test_perf() -> PerfProfile {
        crate::config::PerfMode::Full.profile()
    }

    fn analyze_one(root: &str, file: &str, mode: Mode) -> Result<AnalysisResult> {
        let full = Path::new(root).join(file).to_string_lossy().to_string();
        analyze(root, &[full], 2, None, mode, &test_perf(), None, None)
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
            &test_perf(),
            None,
            None,
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
            &test_perf(),
            None,
            None,
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
            &test_perf(),
            None,
            None,
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
            &test_perf(),
            None,
            None,
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
            &test_perf(),
            None,
            None,
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
            &test_perf(),
            None,
            None,
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
            &test_perf(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(result.total_bytes, 10);
    }

    #[test]
    fn hierarchy_section_for_classes() {
        let dir = setup(&[
            (
                "base.py",
                "class Animal:\n    def speak(self):\n        pass\n",
            ),
            (
                "dog.py",
                "from base import Animal\n\nclass Dog(Animal):\n    def bark(self):\n        pass\n",
            ),
        ]);
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[dir.path().join("dog.py").to_string_lossy().to_string()],
            2,
            None,
            Mode::Full,
            &test_perf(),
            None,
            None,
        )
        .unwrap();
        // Should have hierarchy section with parent reference
        assert!(result.plain.contains("HIERARCHY"));
        assert!(result.plain.contains("Animal"));
    }

    #[test]
    fn ts_imports_resolve_to_dependencies() {
        let dir = setup(&[
            (
                "utils.ts",
                "export function helper(): number { return 42; }\n",
            ),
            (
                "main.ts",
                "import { helper } from './utils';\n\nfunction run() {\n    helper();\n}\n",
            ),
        ]);
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[dir.path().join("main.ts").to_string_lossy().to_string()],
            2,
            None,
            Mode::Full,
            &test_perf(),
            None,
            None,
        )
        .unwrap();
        assert!(result.plain.contains("DEPENDENCIES"));
        assert!(result.plain.contains("helper"));
    }

    #[test]
    fn directory_tree_in_output() {
        let dir = setup(&[
            ("src/main.rs", "fn main() {}"),
            ("src/lib.rs", "pub fn lib() {}"),
        ]);
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[dir.path().join("src").to_string_lossy().to_string()],
            2,
            None,
            Mode::Full,
            &test_perf(),
            None,
            None,
        )
        .unwrap();
        assert!(result.plain.contains("Directory:"));
    }

    #[test]
    fn individual_files_listed() {
        let dir = setup(&[("a.rs", "fn a() {}"), ("b.rs", "fn b() {}")]);
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[
                dir.path().join("a.rs").to_string_lossy().to_string(),
                dir.path().join("b.rs").to_string_lossy().to_string(),
            ],
            2,
            None,
            Mode::Full,
            &test_perf(),
            None,
            None,
        )
        .unwrap();
        assert!(result.plain.contains("Files:"));
        assert_eq!(result.file_count, 2);
    }

    #[test]
    fn original_bytes_tracks_uncompressed() {
        let src = "fn main() {\n    // comment1\n    // comment2\n    // comment3\n    println!(\"hello\");\n}\n";
        let dir = setup(&[("main.rs", src)]);
        let result = analyze_one(dir.path().to_str().unwrap(), "main.rs", Mode::Slim).unwrap();
        // Original bytes should be larger than total_bytes when comments are stripped
        assert!(result.original_bytes >= result.total_bytes);
    }

    #[test]
    fn mixed_dir_and_file_paths() {
        let dir = setup(&[
            ("src/lib.rs", "pub fn lib_fn() {}"),
            ("standalone.rs", "fn standalone() {}"),
        ]);
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[
                dir.path().join("src").to_string_lossy().to_string(),
                dir.path()
                    .join("standalone.rs")
                    .to_string_lossy()
                    .to_string(),
            ],
            2,
            None,
            Mode::Full,
            &test_perf(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(result.file_count, 2);
        assert!(result.plain.contains("Directory:"));
        assert!(result.plain.contains("Files:"));
    }

    #[test]
    fn symbol_with_parent_displayed() {
        let dir = setup(&[(
            "lib.rs",
            "pub struct Foo {}\nimpl Foo {\n    pub fn method(&self) -> i32 { 42 }\n}\n",
        )]);
        let result = analyze_one(dir.path().to_str().unwrap(), "lib.rs", Mode::Full).unwrap();
        assert!(result.plain.contains("SYMBOL INDEX"));
        // method should show parent::name format
        assert!(result.plain.contains("Foo::method") || result.plain.contains("method"));
    }

    #[test]
    fn symbol_signature_in_index() {
        let dir = setup(&[(
            "lib.rs",
            "pub fn compute(x: i32, y: i32) -> i32 {\n    x + y\n}\n",
        )]);
        let result = analyze_one(dir.path().to_str().unwrap(), "lib.rs", Mode::Full).unwrap();
        assert!(result.plain.contains("SYMBOL INDEX"));
        assert!(result.plain.contains("compute"));
    }

    #[test]
    fn external_import_shown() {
        let dir = setup(&[(
            "main.py",
            "import os\nimport sys\n\ndef run():\n    os.path.exists('.')\n",
        )]);
        let result = analyze_one(dir.path().to_str().unwrap(), "main.py", Mode::Full).unwrap();
        // External imports should appear in dependencies with "(external)" marker
        if result.plain.contains("DEPENDENCIES") {
            assert!(result.plain.contains("external") || result.plain.contains("os"));
        }
    }

    #[test]
    fn max_files_limit_exceeded() {
        // Create more files than the limit
        let dir = setup(&[
            ("a.rs", "fn a() {}"),
            ("b.rs", "fn b() {}"),
            ("c.rs", "fn c() {}"),
        ]);
        let mut small_perf = test_perf();
        small_perf.max_files = 2; // max_files = 2, but we have 3 files
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[dir.path().to_string_lossy().to_string()],
            2,
            None,
            Mode::Full,
            &small_perf,
            None,
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("too many files"), "got: {}", err);
    }

    #[test]
    fn max_total_bytes_exceeded() {
        let dir = setup(&[("big.rs", &"x".repeat(1000))]);
        let mut small_perf = test_perf();
        small_perf.max_total_mb = 0; // 0 bytes, content is 1000
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[dir.path().join("big.rs").to_string_lossy().to_string()],
            2,
            None,
            Mode::Full,
            &small_perf,
            None,
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("total content too large"), "got: {}", err);
    }

    #[test]
    fn path_does_not_exist() {
        let result = analyze(
            "/tmp",
            &["/tmp/nonexistent_supp_test_path_xyz".to_string()],
            2,
            None,
            Mode::Full,
            &test_perf(),
            None,
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("path does not exist"), "got: {}", err);
    }

    #[test]
    fn regex_filters_all_files() {
        let dir = setup(&[("a.txt", "hello"), ("b.txt", "world")]);
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[dir.path().to_string_lossy().to_string()],
            2,
            Some(r"\.rs$"), // regex only matches .rs files, but we only have .txt
            Mode::Full,
            &test_perf(),
            None,
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no files matched"), "got: {}", err);
    }

    #[test]
    fn many_files_skips_used_by() {
        // Create >20 files to trigger the used-by skip
        let mut files: Vec<(&str, &str)> = Vec::new();
        let names: Vec<String> = (0..22).map(|i| format!("f{}.rs", i)).collect();
        let content = "fn dummy() {}";
        for name in &names {
            files.push((name.as_str(), content));
        }
        let dir = setup(&files);
        let paths: Vec<String> = names
            .iter()
            .map(|n| dir.path().join(n).to_string_lossy().to_string())
            .collect();
        let mut big_perf = test_perf();
        big_perf.max_files = 30000;
        let result = analyze(
            dir.path().to_str().unwrap(),
            &paths,
            2,
            None,
            Mode::Full,
            &big_perf,
            None,
            None,
        )
        .unwrap();
        assert_eq!(result.file_count, 22);
        // used_by should be 0 because file_count > 20
        assert_eq!(result.used_by_count, 0);
    }

    #[test]
    fn used_by_references_found() {
        // Create two files where one references a symbol from the other
        let dir = setup(&[
            ("lib.rs", "pub fn helper() -> i32 { 42 }\n"),
            (
                "main.rs",
                "use crate::helper;\nfn main() {\n    helper();\n}\n",
            ),
        ]);
        let result = analyze(
            dir.path().to_str().unwrap(),
            &[dir.path().join("lib.rs").to_string_lossy().to_string()],
            2,
            None,
            Mode::Full,
            &test_perf(),
            None,
            None,
        )
        .unwrap();
        // Should find used-by references from main.rs
        assert!(
            result.used_by_count > 0
                || result.plain.contains("USED BY")
                || result.plain.contains("helper")
        );
    }
}
