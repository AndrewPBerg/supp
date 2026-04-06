use std::collections::BTreeMap;

use crate::ctx::AnalysisResult;
use crate::git::{DeltaStatus, DiffResult, FileEntry, FileStatus};
use crate::tree::TreeResult;
use colored::Colorize;

// ── Token estimation ────────────────────────────────────────────────

/// Estimate token count from byte length.
/// Code tokenizes at roughly 3–4 bytes/token with BPE tokenizers.
/// We use 3.0 as a conservative divisor so estimates lean slightly high
/// rather than under-counting (whitespace-heavy code tokenizes worse).
pub fn estimate_tokens(byte_len: usize) -> usize {
    (byte_len as f64 / 3.0).round() as usize
}

// ── Shared utilities ───────────────────────────────────────────────

pub fn format_size(bytes: usize) -> String {
    match bytes {
        b if b < 1_024 => format!("{} B", b),
        b if b < 1_048_576 => format!("{:.1} KB", b as f64 / 1_024.0),
        b if b < 1_073_741_824 => format!("{:.1} MB", b as f64 / 1_048_576.0),
        b => format!("{:.1} GB", b as f64 / 1_073_741_824.0),
    }
}

pub fn format_elapsed(elapsed: std::time::Duration) -> String {
    let ms = elapsed.as_secs_f64() * 1000.0;
    if ms < 1000.0 {
        format!("{:.0}ms", ms)
    } else {
        format!("{:.2}s", elapsed.as_secs_f64())
    }
}

#[cfg(target_os = "linux")]
pub fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let tools: &[(&str, &[&str])] = &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])];
    for (tool, args) in tools {
        if let Ok(mut child) = Command::new(tool).args(*args).stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(text.as_bytes())?;
            }
            // Drop stdin (above) signals EOF; don't wait — wl-copy/xclip detach on their own
            return Ok(());
        }
    }
    Err(anyhow::anyhow!(
        "no clipboard tool found — install wl-clipboard (Wayland) or xclip (X11)"
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    let mut cb = arboard::Clipboard::new()?;
    cb.set_text(text)?;
    Ok(())
}

// ── Git file status indicator (used by tree) ───────────────────────

/// Returns `(plain, colored)` indicator for a git file status.
pub fn file_status_indicator(status: FileStatus) -> (&'static str, String) {
    match status {
        FileStatus::Modified => ("[M]", "[M]".yellow().to_string()),
        FileStatus::Added => ("[A]", "[A]".green().to_string()),
        FileStatus::Deleted => ("[D]", "[D]".red().to_string()),
        FileStatus::Renamed => ("[R]", "[R]".cyan().to_string()),
        FileStatus::Untracked => ("[?]", "[?]".dimmed().to_string()),
    }
}

// ── Diff display ───────────────────────────────────────────────────

fn status_label(delta: DeltaStatus) -> colored::ColoredString {
    match delta {
        DeltaStatus::Added | DeltaStatus::Untracked => " added   ".green(),
        DeltaStatus::Deleted => " deleted ".red(),
        DeltaStatus::Modified => " modified".yellow(),
        DeltaStatus::Renamed => " renamed ".cyan(),
        DeltaStatus::Copied => " copied  ".cyan(),
    }
}

fn print_file_tree(files: &[FileEntry]) -> (usize, usize, usize) {
    let mut tree: BTreeMap<String, Vec<&FileEntry>> = BTreeMap::new();

    for entry in files {
        let path = std::path::Path::new(&entry.path);
        let parent = path
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string();
        tree.entry(parent).or_default().push(entry);
    }

    let max_add_w = files
        .iter()
        .map(|e| e.additions.to_string().len() + 1)
        .max()
        .unwrap_or(2);
    let max_del_w = files
        .iter()
        .map(|e| e.deletions.to_string().len() + 1)
        .max()
        .unwrap_or(2);

    let global_max_name_col: usize = tree
        .iter()
        .flat_map(|(dir, entries)| {
            let prefix_w: usize = if dir.is_empty() { 0 } else { 4 };
            entries.iter().map(move |e| {
                let fname_len = std::path::Path::new(&e.path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&e.path)
                    .len();
                prefix_w + 4 + fname_len
            })
        })
        .max()
        .unwrap_or(0);

    let dirs: Vec<&String> = tree.keys().collect();
    let dir_count = dirs.len();

    for (di, dir) in dirs.iter().enumerate() {
        let is_last_dir = di == dir_count - 1;
        let entries = &tree[*dir];

        if !dir.is_empty() {
            let dir_prefix = if is_last_dir {
                "└── "
            } else {
                "├── "
            };
            println!("{}{}", dir_prefix.dimmed(), format!("{}/", dir).bold());
        }

        let (file_prefix, file_prefix_w) = if dir.is_empty() {
            ("", 0)
        } else if is_last_dir {
            ("    ", 4)
        } else {
            ("│   ", 4)
        };

        let entry_count = entries.len();
        for (fi, entry) in entries.iter().enumerate() {
            let is_last_file = fi == entry_count - 1;
            let branch_char = if is_last_file {
                "└── "
            } else {
                "├── "
            };
            let filename = std::path::Path::new(&entry.path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&entry.path);
            let rename_hint = entry
                .old_path
                .as_deref()
                .map(|old| format!("  ← {}", old).dimmed().to_string())
                .unwrap_or_default();
            let add_str = format!(
                "{:>width$}",
                format!("+{}", entry.additions),
                width = max_add_w
            );
            let del_str = format!(
                "{:>width$}",
                format!("-{}", entry.deletions),
                width = max_del_w
            );
            let line_delta = format!("  {} {}", add_str.green(), del_str.red());
            println!(
                "{}{}{:<width$}{}{}{}",
                file_prefix.dimmed(),
                branch_char.dimmed(),
                filename,
                status_label(entry.status),
                line_delta,
                rename_hint,
                width = global_max_name_col - file_prefix_w - 4,
            );
        }
    }
    (global_max_name_col, max_add_w, max_del_w)
}

fn print_summary(
    files: &[FileEntry],
    global_max_name_col: usize,
    max_add_w: usize,
    max_del_w: usize,
) {
    let added = files
        .iter()
        .filter(|f| matches!(f.status, DeltaStatus::Added | DeltaStatus::Untracked))
        .count();
    let modified = files
        .iter()
        .filter(|f| f.status == DeltaStatus::Modified)
        .count();
    let deleted = files
        .iter()
        .filter(|f| f.status == DeltaStatus::Deleted)
        .count();
    let renamed = files
        .iter()
        .filter(|f| f.status == DeltaStatus::Renamed)
        .count();

    let total = files.len();
    let mut parts: Vec<String> = Vec::new();
    let mut parts_plain: Vec<String> = Vec::new();
    if added > 0 {
        let s = format!("{}+", added);
        parts.push(s.green().to_string());
        parts_plain.push(s);
    }
    if modified > 0 {
        let s = format!("{}~", modified);
        parts.push(s.yellow().to_string());
        parts_plain.push(s);
    }
    if deleted > 0 {
        let s = format!("{}-", deleted);
        parts.push(s.red().to_string());
        parts_plain.push(s);
    }
    if renamed > 0 {
        let s = format!("{}~", renamed);
        parts.push(s.cyan().to_string());
        parts_plain.push(s);
    }

    let total_adds: usize = files.iter().map(|f| f.additions).sum();
    let total_dels: usize = files.iter().map(|f| f.deletions).sum();

    let total_str = total.to_string();
    let suffix = if total == 1 { "" } else { "s" };

    let detail = parts.join(" ");
    let detail_plain = parts_plain.join(" ");

    let left_visible_w = 2 + total_str.len() + 5 + suffix.len();
    let status_col = global_max_name_col;
    let pad_to_status = if status_col > left_visible_w {
        status_col - left_visible_w
    } else {
        1
    };
    let detail_pad = if 9 > detail_plain.len() {
        9 - detail_plain.len()
    } else {
        0
    };
    let detail_aligned = format!("{}{}", " ".repeat(detail_pad), detail);

    let add_str = format!("{:>width$}", format!("+{}", total_adds), width = max_add_w);
    let del_str = format!("{:>width$}", format!("-{}", total_dels), width = max_del_w);

    println!(
        "\n  {} file{}{}{}  {}  {}",
        total_str.bold(),
        suffix,
        " ".repeat(pad_to_status),
        detail_aligned,
        add_str.green().bold(),
        del_str.red().bold(),
    );
}

pub(crate) fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

pub fn print_diff_result(result: DiffResult, no_copy: bool, start: std::time::Instant) {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();

    let mut meta_parts: Vec<String> = Vec::new();
    if let Some(count) = result.commit_count {
        let suffix = if count == 1 { "commit" } else { "commits" };
        meta_parts.push(format!("{} {}", count, suffix));
    }
    meta_parts.push(now.clone());
    let meta = meta_parts.join("  ·  ");

    println!();
    if result.files.is_empty() {
        println!("  {}", "No changes found.".dimmed());
    } else {
        let (name_col, add_w, del_w) = print_file_tree(&result.files);
        print_summary(&result.files, name_col, add_w, del_w);
        println!();
    }

    println!(
        "  {}  {}  ·  {}",
        "supp diff".bold().cyan(),
        result.label.dimmed(),
        meta.dimmed()
    );
    println!("  {}", "─".repeat(40).dimmed());

    if result.is_branch_comparison {
        if result.has_conflicts {
            println!("  {}", "✗ Merge conflicts detected".red().bold());
        } else {
            println!("  {}", "✓ No merge conflicts".green());
        }
    }
    if let Some(rx) = result.stale_check
        && let Ok(true) = rx.recv_timeout(std::time::Duration::from_millis(300))
    {
        println!(
            "  {} {}",
            "⚠".yellow().bold(),
            format!(
                "{} has new commits — re-run for latest",
                result.label.split(" ... ").next().unwrap_or(&result.label)
            )
            .yellow()
        );
    }
    let mut clipboard_header = format!("supp diff  {}\n", result.label);
    clipboard_header.push_str(&meta);
    clipboard_header.push_str("\n---\n\n");
    let clipboard_text = format!("{}{}", clipboard_header, result.text);

    print_footer(&clipboard_text, no_copy, start, None, false);
}

// ── Tree display ───────────────────────────────────────────────────

pub fn print_tree_result(result: TreeResult, root: &str, no_copy: bool, start: std::time::Instant) {
    println!();
    println!("  {}  {}", "supp tree".bold().cyan(), root.dimmed());
    println!("  {}", "─".repeat(40).dimmed());
    println!();

    for line in result.display.lines() {
        println!("  {}", line);
    }

    let dir_s = if result.dir_count == 1 {
        "directory"
    } else {
        "directories"
    };
    let file_s = if result.file_count == 1 {
        "file"
    } else {
        "files"
    };

    // Build status summary parts
    let mut status_parts: Vec<String> = Vec::new();
    let modified = result
        .status_counts
        .get(&FileStatus::Modified)
        .copied()
        .unwrap_or(0);
    let added = result
        .status_counts
        .get(&FileStatus::Added)
        .copied()
        .unwrap_or(0);
    let untracked = result
        .status_counts
        .get(&FileStatus::Untracked)
        .copied()
        .unwrap_or(0);
    let renamed = result
        .status_counts
        .get(&FileStatus::Renamed)
        .copied()
        .unwrap_or(0);

    if modified > 0 {
        status_parts.push(format!("{} modified", modified).yellow().to_string());
    }
    if added > 0 {
        status_parts.push(format!("{} added", added).green().to_string());
    }
    if untracked > 0 {
        status_parts.push(format!("{} untracked", untracked).dimmed().to_string());
    }
    if renamed > 0 {
        status_parts.push(format!("{} renamed", renamed).cyan().to_string());
    }

    if status_parts.is_empty() {
        println!(
            "\n  {} {}, {} {}",
            result.dir_count.to_string().bold(),
            dir_s,
            result.file_count.to_string().bold(),
            file_s,
        );
    } else {
        println!(
            "\n  {} {}, {} {} ({})",
            result.dir_count.to_string().bold(),
            dir_s,
            result.file_count.to_string().bold(),
            file_s,
            status_parts.join(", "),
        );
    }
    println!();

    print_footer(&result.plain, no_copy, start, None, false);
}

// ── Shared footer (clipboard, compression, tokens, timing) ──────

fn print_footer(
    text: &str,
    no_copy: bool,
    start: std::time::Instant,
    original_bytes: Option<(usize, usize)>,
    use_stderr: bool,
) {
    macro_rules! out {
        ($($arg:tt)*) => {
            if use_stderr { eprintln!($($arg)*); } else { println!($($arg)*); }
        };
    }

    if no_copy {
        out!(
            "  {} {}",
            "–".dimmed(),
            format!("({}, not copied)", format_size(text.len())).dimmed(),
        );
    } else {
        match copy_to_clipboard(text) {
            Ok(()) => {
                out!(
                    "  {} {} {}",
                    "✓".green().bold(),
                    "Copied to clipboard".green(),
                    format!("({})", format_size(text.len())).dimmed(),
                );
            }
            Err(e) => {
                out!(
                    "  {} {}",
                    "✗".red().bold(),
                    format!("Clipboard error: {}", e).red(),
                );
            }
        }
    }
    if let Some((original, total)) = original_bytes
        && original > total
    {
        let pct = 100.0 * (1.0 - total as f64 / original as f64);
        out!(
            "  {} {}",
            "↓".dimmed(),
            format!(
                "{} → {} ({:.0}% reduction)",
                format_size(original),
                format_size(total),
                pct,
            )
            .dimmed(),
        );
    }
    let tokens = estimate_tokens(text.len());
    if let Some((original, total)) = original_bytes
        && original > total
    {
        let orig_tokens = estimate_tokens(original);
        out!(
            "  {} {}",
            "≈".dimmed(),
            format!(
                "~{} → ~{} tokens (est.)",
                format_number(orig_tokens),
                format_number(tokens)
            )
            .dimmed(),
        );
    } else {
        out!(
            "  {} {}",
            "≈".dimmed(),
            format!("~{} tokens (est.)", format_number(tokens)).dimmed(),
        );
    }
    out!(
        "  {}",
        format!("Done in {}", format_elapsed(start.elapsed())).dimmed()
    );
    out!();
}

// ── Sym display ─────────────────────────────────────────────────

pub fn print_sym_results(
    result: &crate::symbol::SearchResult,
    no_copy: bool,
    start: std::time::Instant,
) {
    println!();
    let mut plain = String::new();

    if result.matches.is_empty() {
        println!("  {}", "No matching symbols found.".dimmed());
    } else {
        // Compute column widths based on display name (with parent prefix)
        let display_names: Vec<String> = result
            .matches
            .iter()
            .map(|(sym, _)| {
                if let Some(ref parent) = sym.parent {
                    format!("{}::{}", parent, sym.name)
                } else {
                    sym.name.clone()
                }
            })
            .collect();
        let max_name: usize = display_names.iter().map(|n| n.len()).max().unwrap_or(0);
        let max_file: usize = result
            .matches
            .iter()
            .map(|(s, _)| format!("{}:{}", s.file, s.line).len())
            .max()
            .unwrap_or(0);

        for (idx, (sym, _score)) in result.matches.iter().enumerate() {
            let tag = sym.kind.tag();
            let tag_colored = color_kind_tag(sym.kind);

            let location = format!("{}:{}", sym.file, sym.line);
            let name_display = if let Some(ref parent) = sym.parent {
                format!("{}::{}", parent.dimmed(), sym.name.bold())
            } else {
                sym.name.bold().to_string()
            };

            println!(
                " {} {:<width_name$}  {:<width_file$}  {}",
                tag_colored,
                name_display,
                location.dimmed(),
                sym.signature.dimmed(),
                width_name = max_name,
                width_file = max_file,
            );

            // Build plain text line for clipboard
            use std::fmt::Write;
            let _ = writeln!(
                plain,
                " {} {:<width_name$}  {:<width_file$}  {}",
                tag,
                &display_names[idx],
                location,
                sym.signature,
                width_name = max_name,
                width_file = max_file,
            );
        }
    }

    println!(
        "\n{} symbols · {} indexed · {}",
        result.matches.len().to_string().bold(),
        format_number(result.total_symbols),
        format_elapsed(start.elapsed()).dimmed(),
    );

    print_clipboard_status(&plain, no_copy);
    println!();
}

// ── Why display ─────────────────────────────────────────────────

fn print_clipboard_status(text: &str, no_copy: bool) {
    if text.is_empty() {
        return;
    }
    if no_copy {
        println!(
            "  {} {}",
            "–".dimmed(),
            format!("({}, not copied)", format_size(text.len())).dimmed(),
        );
    } else {
        match copy_to_clipboard(text) {
            Ok(()) => {
                println!(
                    "  {} {} {}",
                    "✓".green().bold(),
                    "Copied to clipboard".green(),
                    format!("({})", format_size(text.len())).dimmed(),
                );
            }
            Err(e) => {
                println!(
                    "  {} {}",
                    "✗".red().bold(),
                    format!("Clipboard error: {}", e).red(),
                );
            }
        }
    }
}

fn color_kind_tag(kind: crate::symbol::SymbolKind) -> String {
    let tag = kind.tag();
    match kind {
        crate::symbol::SymbolKind::Function => tag.cyan().bold().to_string(),
        crate::symbol::SymbolKind::Struct => tag.yellow().bold().to_string(),
        crate::symbol::SymbolKind::Enum => tag.green().bold().to_string(),
        crate::symbol::SymbolKind::Trait => tag.magenta().bold().to_string(),
        crate::symbol::SymbolKind::Class => tag.yellow().bold().to_string(),
        crate::symbol::SymbolKind::Interface => tag.magenta().bold().to_string(),
        crate::symbol::SymbolKind::Method => tag.cyan().to_string(),
        crate::symbol::SymbolKind::Type => tag.blue().to_string(),
        crate::symbol::SymbolKind::Const => tag.red().to_string(),
        crate::symbol::SymbolKind::Macro => tag.red().bold().to_string(),
        crate::symbol::SymbolKind::File => tag.dimmed().to_string(),
    }
}

pub fn print_why_result(result: &crate::why::WhyResult, no_copy: bool, start: std::time::Instant) {
    println!();

    let sym = &result.symbol;
    let tag_colored = color_kind_tag(sym.kind);

    let display_name = if let Some(ref parent) = sym.parent {
        format!("{}::{}", parent.dimmed(), sym.name.bold())
    } else {
        sym.name.bold().to_string()
    };
    let location = format!("{}:{}", sym.file, sym.line);

    println!(
        "  {} {} {}  {}",
        "supp why".bold().cyan(),
        tag_colored,
        display_name,
        location.dimmed()
    );
    println!("  {}", "─".repeat(40).dimmed());

    // Doc comment
    if let Some(ref doc) = result.doc_comment {
        println!();
        for line in doc.lines() {
            println!("  {}", line.dimmed());
        }
    }

    // Hierarchy (parents/children)
    if let Some(ref h) = result.hierarchy {
        println!();
        if !h.parents.is_empty() {
            println!("  {}", "Parents".bold());
            for p in &h.parents {
                if let Some((ref file, line)) = p.location {
                    let loc = format!("{}:{}", file, line);
                    let module_hint = p
                        .external_module
                        .as_ref()
                        .map(|m| format!("  ({})", m).dimmed().to_string())
                        .unwrap_or_default();
                    println!(
                        "    {} {}  {}{}",
                        "^".dimmed(),
                        p.name.bold(),
                        loc.dimmed(),
                        module_hint
                    );
                } else {
                    let module = p.external_module.as_deref().unwrap_or("external");
                    println!(
                        "    {} {}  {}",
                        "^".dimmed(),
                        p.name.bold(),
                        format!("({})", module).dimmed()
                    );
                }
            }
        }
        if !h.children.is_empty() {
            println!("  {}", "Children".bold());
            for c in &h.children {
                if let Some((ref file, line)) = c.location {
                    let loc = format!("{}:{}", file, line);
                    println!("    {} {}  {}", "v".dimmed(), c.name.bold(), loc.dimmed());
                } else {
                    println!("    {} {}", "v".dimmed(), c.name.bold());
                }
            }
        }
    }

    // Definition preview (first ~25 lines, full in clipboard)
    println!();
    let def_lines: Vec<&str> = result.full_definition.lines().collect();
    let show_lines = def_lines.len().min(25);
    for line in &def_lines[..show_lines] {
        println!("  {}", line);
    }
    if def_lines.len() > show_lines {
        println!(
            "  {} {}",
            "...".dimmed(),
            format!("({} more lines)", def_lines.len() - show_lines).dimmed()
        );
    }

    // Call sites
    if !result.call_sites.is_empty() {
        println!();
        println!(
            "  {} {}",
            "Referenced in".bold(),
            format!(
                "{} location{}",
                result.call_sites.len(),
                if result.call_sites.len() == 1 {
                    ""
                } else {
                    "s"
                }
            )
            .dimmed()
        );
        let show_count = result.call_sites.len().min(10);
        for site in &result.call_sites[..show_count] {
            let loc = format!("{}:{}", site.file, site.line);
            let caller_str = site
                .caller
                .as_ref()
                .map(|c| format!(" in {}", c.cyan()))
                .unwrap_or_default();
            println!(
                "    {}{}  {}",
                loc.dimmed(),
                caller_str,
                site.context.dimmed()
            );
        }
        if result.call_sites.len() > show_count {
            println!(
                "    {}",
                format!("... and {} more", result.call_sites.len() - show_count).dimmed()
            );
        }
    }

    // Dependencies
    if !result.dependencies.is_empty() {
        println!();
        println!(
            "  {} {}",
            "Depends on".bold(),
            format!(
                "{} symbol{}",
                result.dependencies.len(),
                if result.dependencies.len() == 1 {
                    ""
                } else {
                    "s"
                }
            )
            .dimmed()
        );
        let show_count = result.dependencies.len().min(15);
        for dep in &result.dependencies[..show_count] {
            let tag = dep
                .kind
                .map(color_kind_tag)
                .unwrap_or_else(|| "--".dimmed().to_string());
            let loc = if let Some((ref file, line)) = dep.location {
                format!("{}:{}", file, line).dimmed().to_string()
            } else if let Some(ref module) = dep.import_from {
                format!("({})", module).dimmed().to_string()
            } else {
                "(external)".dimmed().to_string()
            };
            println!("    {} {}  {}", tag, dep.name.bold(), loc);
        }
        if result.dependencies.len() > show_count {
            println!(
                "    {}",
                format!("... and {} more", result.dependencies.len() - show_count).dimmed()
            );
        }
    }

    println!();

    print_clipboard_status(&result.plain, no_copy);
    println!(
        "  {}",
        format!("Done in {}", format_elapsed(start.elapsed())).dimmed()
    );
    println!();
}

// ── Ctx display ─────────────────────────────────────────────────

pub fn print_ctx_result(
    result: &crate::ctx::AnalysisResult,
    no_copy: bool,
    start: std::time::Instant,
) {
    println!();
    println!(
        "  {}  {} file{}, {} dep{}, {} reference{}",
        "supp".bold().cyan(),
        result.file_count,
        if result.file_count == 1 { "" } else { "s" },
        result.dep_file_count,
        if result.dep_file_count == 1 { "" } else { "s" },
        result.used_by_count,
        if result.used_by_count == 1 { "" } else { "s" },
    );
    println!("  {}", "─".repeat(40).dimmed());
    println!();

    // Brief summary
    println!(
        "  {} lines, {}",
        result.total_lines.to_string().bold(),
        format_size(result.total_bytes).dimmed()
    );
    println!();

    let compression = Some((result.original_bytes, result.total_bytes));
    print_footer(&result.plain, no_copy, start, compression, false);
}

// ── Context display ─────────────────────────────────────────────

pub fn print_context_result(result: &AnalysisResult, no_copy: bool, start: std::time::Instant) {
    println!();
    println!(
        "  {}  {} file{}, {} line{}, {}",
        "supp".bold().cyan(),
        result.file_count,
        if result.file_count == 1 { "" } else { "s" },
        result.total_lines,
        if result.total_lines == 1 { "" } else { "s" },
        format_size(result.total_bytes).dimmed()
    );
    println!("  {}", "─".repeat(40).dimmed());
    println!();

    let compression = Some((result.original_bytes, result.total_bytes));
    print_footer(&result.plain, no_copy, start, compression, false);
}

// ── Pick display ────────────────────────────────────────────────

pub fn print_pick_stats(result: &AnalysisResult, no_copy: bool, start: std::time::Instant) {
    eprintln!();
    eprintln!(
        "  {}  {} file{}, {} line{}, {}",
        "pick".bold().cyan(),
        result.file_count,
        if result.file_count == 1 { "" } else { "s" },
        result.total_lines,
        if result.total_lines == 1 { "" } else { "s" },
        format_size(result.total_bytes).dimmed()
    );
    eprintln!("  {}", "─".repeat(40).dimmed());
    eprintln!();

    let compression = Some((result.original_bytes, result.total_bytes));
    print_footer(&result.plain, no_copy, start, compression, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ── format_size ──────────────────────────────────────────────

    #[test]
    fn format_size_zero_bytes() {
        assert_eq!(format_size(0), "0 B");
    }

    #[test]
    fn format_size_one_byte() {
        assert_eq!(format_size(1), "1 B");
    }

    #[test]
    fn format_size_just_below_kb() {
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn format_size_exact_kb() {
        assert_eq!(format_size(1024), "1.0 KB");
    }

    #[test]
    fn format_size_1_5_kb() {
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn format_size_just_below_mb() {
        assert_eq!(format_size(1_048_575), "1024.0 KB");
    }

    #[test]
    fn format_size_exact_mb() {
        assert_eq!(format_size(1_048_576), "1.0 MB");
    }

    #[test]
    fn format_size_exact_gb() {
        assert_eq!(format_size(1_073_741_824), "1.0 GB");
    }

    // ── format_elapsed ───────────────────────────────────────────

    #[test]
    fn format_elapsed_zero() {
        assert_eq!(format_elapsed(Duration::from_millis(0)), "0ms");
    }

    #[test]
    fn format_elapsed_150ms() {
        assert_eq!(format_elapsed(Duration::from_millis(150)), "150ms");
    }

    #[test]
    fn format_elapsed_999ms() {
        assert_eq!(format_elapsed(Duration::from_millis(999)), "999ms");
    }

    #[test]
    fn format_elapsed_1000ms_switches_to_seconds() {
        assert_eq!(format_elapsed(Duration::from_millis(1000)), "1.00s");
    }

    #[test]
    fn format_elapsed_2500ms() {
        assert_eq!(format_elapsed(Duration::from_millis(2500)), "2.50s");
    }

    // ── format_number ────────────────────────────────────────────

    #[test]
    fn format_number_zero() {
        assert_eq!(format_number(0), "0");
    }

    #[test]
    fn format_number_small() {
        assert_eq!(format_number(42), "42");
    }

    #[test]
    fn format_number_999() {
        assert_eq!(format_number(999), "999");
    }

    #[test]
    fn format_number_1000() {
        assert_eq!(format_number(1000), "1,000");
    }

    #[test]
    fn format_number_12345() {
        assert_eq!(format_number(12345), "12,345");
    }

    #[test]
    fn format_number_millions() {
        assert_eq!(format_number(1_234_567), "1,234,567");
    }

    // ── file_status_indicator ────────────────────────────────────

    #[test]
    fn file_status_indicator_modified() {
        let (plain, _) = file_status_indicator(FileStatus::Modified);
        assert_eq!(plain, "[M]");
    }

    #[test]
    fn file_status_indicator_added() {
        let (plain, _) = file_status_indicator(FileStatus::Added);
        assert_eq!(plain, "[A]");
    }

    #[test]
    fn file_status_indicator_deleted() {
        let (plain, _) = file_status_indicator(FileStatus::Deleted);
        assert_eq!(plain, "[D]");
    }

    #[test]
    fn file_status_indicator_renamed() {
        let (plain, _) = file_status_indicator(FileStatus::Renamed);
        assert_eq!(plain, "[R]");
    }

    #[test]
    fn file_status_indicator_untracked() {
        let (plain, _) = file_status_indicator(FileStatus::Untracked);
        assert_eq!(plain, "[?]");
    }

    // ── token estimation ────────────────────────────────────────

    #[test]
    fn estimate_tokens_code() {
        // 300 bytes of code → 100 tokens at 3.0 bytes/token
        assert_eq!(estimate_tokens(300), 100);
    }

    #[test]
    fn estimate_tokens_zero() {
        assert_eq!(estimate_tokens(0), 0);
    }

    #[test]
    fn estimate_tokens_small() {
        // 7 bytes → 2 tokens (7/3.0 = 2.33, rounds to 2)
        assert_eq!(estimate_tokens(7), 2);
    }
    // ── file_status_indicator colored output ────────────────────

    #[test]
    fn file_status_indicator_colored_not_empty() {
        for status in [
            FileStatus::Modified,
            FileStatus::Added,
            FileStatus::Deleted,
            FileStatus::Renamed,
            FileStatus::Untracked,
        ] {
            let (_, colored) = file_status_indicator(status);
            assert!(!colored.is_empty());
        }
    }

    // ── status_label ────────────────────────────────────────────

    #[test]
    fn status_label_added() {
        let s = status_label(DeltaStatus::Added);
        assert!(s.to_string().contains("added"));
    }

    #[test]
    fn status_label_deleted() {
        let s = status_label(DeltaStatus::Deleted);
        assert!(s.to_string().contains("deleted"));
    }

    #[test]
    fn status_label_modified() {
        let s = status_label(DeltaStatus::Modified);
        assert!(s.to_string().contains("modified"));
    }

    #[test]
    fn status_label_renamed() {
        let s = status_label(DeltaStatus::Renamed);
        assert!(s.to_string().contains("renamed"));
    }

    #[test]
    fn status_label_copied() {
        let s = status_label(DeltaStatus::Copied);
        assert!(s.to_string().contains("copied"));
    }

    #[test]
    fn status_label_untracked() {
        let s = status_label(DeltaStatus::Untracked);
        assert!(s.to_string().contains("added"));
    }

    // ── print_file_tree ─────────────────────────────────────────

    #[test]
    fn print_file_tree_single_file() {
        let files = vec![FileEntry {
            path: "src/main.rs".to_string(),
            old_path: None,
            status: DeltaStatus::Modified,
            additions: 5,
            deletions: 2,
            patch: String::new(),
        }];
        let (name_col, add_w, del_w) = print_file_tree(&files);
        assert!(name_col > 0);
        assert!(add_w > 0);
        assert!(del_w > 0);
    }

    #[test]
    fn print_file_tree_root_level_file() {
        let files = vec![FileEntry {
            path: "README.md".to_string(),
            old_path: None,
            status: DeltaStatus::Added,
            additions: 10,
            deletions: 0,
            patch: String::new(),
        }];
        let (name_col, _, _) = print_file_tree(&files);
        assert!(name_col > 0);
    }

    #[test]
    fn print_file_tree_renamed() {
        let files = vec![FileEntry {
            path: "new_name.rs".to_string(),
            old_path: Some("old_name.rs".to_string()),
            status: DeltaStatus::Renamed,
            additions: 0,
            deletions: 0,
            patch: String::new(),
        }];
        let _ = print_file_tree(&files);
    }

    #[test]
    fn print_file_tree_multiple_dirs() {
        let files = vec![
            FileEntry {
                path: "src/main.rs".to_string(),
                old_path: None,
                status: DeltaStatus::Modified,
                additions: 3,
                deletions: 1,
                patch: String::new(),
            },
            FileEntry {
                path: "tests/test.rs".to_string(),
                old_path: None,
                status: DeltaStatus::Added,
                additions: 10,
                deletions: 0,
                patch: String::new(),
            },
        ];
        let _ = print_file_tree(&files);
    }

    // ── print_summary ───────────────────────────────────────────

    #[test]
    fn print_summary_all_statuses() {
        let files = vec![
            FileEntry {
                path: "a.rs".to_string(),
                old_path: None,
                status: DeltaStatus::Added,
                additions: 10,
                deletions: 0,
                patch: String::new(),
            },
            FileEntry {
                path: "b.rs".to_string(),
                old_path: None,
                status: DeltaStatus::Modified,
                additions: 5,
                deletions: 2,
                patch: String::new(),
            },
            FileEntry {
                path: "c.rs".to_string(),
                old_path: None,
                status: DeltaStatus::Deleted,
                additions: 0,
                deletions: 8,
                patch: String::new(),
            },
            FileEntry {
                path: "d.rs".to_string(),
                old_path: Some("old.rs".to_string()),
                status: DeltaStatus::Renamed,
                additions: 0,
                deletions: 0,
                patch: String::new(),
            },
            FileEntry {
                path: "e.rs".to_string(),
                old_path: None,
                status: DeltaStatus::Untracked,
                additions: 3,
                deletions: 0,
                patch: String::new(),
            },
        ];
        print_summary(&files, 30, 3, 3);
    }

    #[test]
    fn print_summary_single_file() {
        let files = vec![FileEntry {
            path: "a.rs".to_string(),
            old_path: None,
            status: DeltaStatus::Modified,
            additions: 1,
            deletions: 1,
            patch: String::new(),
        }];
        print_summary(&files, 20, 2, 2);
    }

    // ── print_diff_result ───────────────────────────────────────

    #[test]
    fn print_diff_result_empty_files() {
        let result = DiffResult {
            label: "test".to_string(),
            files: vec![],
            text: "".to_string(),
            has_conflicts: false,
            is_branch_comparison: false,
            commit_count: None,
            stale_check: None,
        };
        print_diff_result(result, true, std::time::Instant::now());
    }

    #[test]
    fn print_diff_result_with_conflicts() {
        let result = DiffResult {
            label: "origin/main ... feature".to_string(),
            files: vec![],
            text: "".to_string(),
            has_conflicts: true,
            is_branch_comparison: true,
            commit_count: Some(3),
            stale_check: None,
        };
        print_diff_result(result, true, std::time::Instant::now());
    }

    #[test]
    fn print_diff_result_no_conflicts() {
        let result = DiffResult {
            label: "origin/main ... feature".to_string(),
            files: vec![],
            text: "".to_string(),
            has_conflicts: false,
            is_branch_comparison: true,
            commit_count: Some(1),
            stale_check: None,
        };
        print_diff_result(result, true, std::time::Instant::now());
    }

    #[test]
    fn print_diff_result_with_files() {
        let result = DiffResult {
            label: "test".to_string(),
            files: vec![FileEntry {
                path: "src/main.rs".to_string(),
                old_path: None,
                status: DeltaStatus::Modified,
                additions: 5,
                deletions: 2,
                patch: "+new line\n".to_string(),
            }],
            text: "+new line\n".to_string(),
            has_conflicts: false,
            is_branch_comparison: false,
            commit_count: None,
            stale_check: None,
        };
        print_diff_result(result, true, std::time::Instant::now());
    }

    // ── print_tree_result ───────────────────────────────────────

    #[test]
    fn print_tree_result_basic() {
        let mut status_counts = std::collections::HashMap::new();
        status_counts.insert(FileStatus::Modified, 1);
        let result = TreeResult {
            display: "root/\n└── file.txt [M]\n".to_string(),
            plain: "root/\n└── file.txt\n".to_string(),
            file_count: 1,
            dir_count: 0,
            status_counts,
        };
        print_tree_result(result, ".", true, std::time::Instant::now());
    }

    #[test]
    fn print_tree_result_no_statuses() {
        let result = TreeResult {
            display: "root/\n└── file.txt\n".to_string(),
            plain: "root/\n└── file.txt\n".to_string(),
            file_count: 1,
            dir_count: 1,
            status_counts: std::collections::HashMap::new(),
        };
        print_tree_result(result, "src", true, std::time::Instant::now());
    }

    #[test]
    fn print_tree_result_all_status_types() {
        let mut status_counts = std::collections::HashMap::new();
        status_counts.insert(FileStatus::Modified, 2);
        status_counts.insert(FileStatus::Added, 1);
        status_counts.insert(FileStatus::Untracked, 3);
        status_counts.insert(FileStatus::Renamed, 1);
        let result = TreeResult {
            display: "root/\n".to_string(),
            plain: "root/\n".to_string(),
            file_count: 7,
            dir_count: 2,
            status_counts,
        };
        print_tree_result(result, ".", true, std::time::Instant::now());
    }

    // ── print_footer edge cases ─────────────────────────────────

    #[test]
    fn print_footer_with_tokens() {
        print_footer("test text", true, std::time::Instant::now(), None, false);
    }

    #[test]
    fn print_footer_with_compression() {
        print_footer(
            "test text",
            true,
            std::time::Instant::now(),
            Some((200, 100)),
            false,
        );
    }

    #[test]
    fn print_footer_stderr_mode() {
        print_footer(
            "test text",
            true,
            std::time::Instant::now(),
            Some((200, 100)),
            true,
        );
    }

    #[test]
    fn print_footer_no_compression_when_equal() {
        print_footer(
            "test text",
            true,
            std::time::Instant::now(),
            Some((100, 100)),
            false,
        );
    }

    // ── color_kind_tag ──────────────────────────────────────────

    #[test]
    fn color_kind_tag_all_variants() {
        use crate::symbol::SymbolKind;
        let variants = [
            SymbolKind::Function,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::Trait,
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Method,
            SymbolKind::Type,
            SymbolKind::Const,
            SymbolKind::Macro,
            SymbolKind::File,
        ];
        for kind in variants {
            let tag = color_kind_tag(kind);
            assert!(
                !tag.is_empty(),
                "color_kind_tag returned empty for {:?}",
                kind
            );
        }
    }

    // ── print_clipboard_status ──────────────────────────────────

    #[test]
    fn print_clipboard_status_no_copy() {
        // no_copy=true just prints a "not copied" message
        print_clipboard_status("some text", true);
    }

    #[test]
    fn print_clipboard_status_empty() {
        // empty text triggers early return
        print_clipboard_status("", false);
    }

    // ── print_sym_results ───────────────────────────────────────

    #[test]
    fn print_sym_results_empty() {
        use crate::symbol::SearchResult;
        let result = SearchResult {
            matches: vec![],
            total_symbols: 100,
        };
        print_sym_results(&result, true, std::time::Instant::now());
    }

    #[test]
    fn print_sym_results_with_matches() {
        use crate::symbol::{SearchResult, Symbol, SymbolKind};
        let sym_with_parent = Symbol {
            name: "do_thing".to_string(),
            kind: SymbolKind::Method,
            file: "src/lib.rs".to_string(),
            line: 42,
            signature: "fn do_thing(&self)".to_string(),
            parent: Some("MyStruct".to_string()),
            keywords: vec![],
        };
        let sym_without_parent = Symbol {
            name: "helper".to_string(),
            kind: SymbolKind::Function,
            file: "src/util.rs".to_string(),
            line: 10,
            signature: "fn helper() -> bool".to_string(),
            parent: None,
            keywords: vec![],
        };
        let result = SearchResult {
            matches: vec![(sym_with_parent, 1.0), (sym_without_parent, 0.8)],
            total_symbols: 200,
        };
        print_sym_results(&result, true, std::time::Instant::now());
    }

    // ── print_why_result ────────────────────────────────────────

    fn make_test_symbol() -> crate::symbol::Symbol {
        crate::symbol::Symbol {
            name: "test_fn".to_string(),
            kind: crate::symbol::SymbolKind::Function,
            file: "src/lib.rs".to_string(),
            line: 10,
            signature: "fn test_fn()".to_string(),
            parent: None,
            keywords: vec![],
        }
    }

    #[test]
    fn print_why_result_basic() {
        use crate::why::WhyResult;
        let result = WhyResult {
            symbol: make_test_symbol(),
            full_definition: "fn test_fn() {\n    todo!()\n}".to_string(),
            doc_comment: None,
            call_sites: vec![],
            dependencies: vec![],
            hierarchy: None,
            plain: "test_fn plain text".to_string(),
        };
        print_why_result(&result, true, std::time::Instant::now());
    }

    #[test]
    fn print_why_result_with_all_sections() {
        use crate::why::{CallSite, Dependency, Hierarchy, HierarchyEntry, WhyResult};
        let result = WhyResult {
            symbol: make_test_symbol(),
            full_definition: "fn test_fn() {\n    todo!()\n}".to_string(),
            doc_comment: Some("/// A test function\n/// with docs".to_string()),
            call_sites: vec![CallSite {
                file: "src/main.rs".to_string(),
                line: 20,
                context: "test_fn()".to_string(),
                caller: Some("main".to_string()),
            }],
            dependencies: vec![Dependency {
                name: "HashMap".to_string(),
                kind: Some(crate::symbol::SymbolKind::Struct),
                location: Some(("src/map.rs".to_string(), 5)),
                import_from: None,
            }],
            hierarchy: Some(Hierarchy {
                parents: vec![
                    HierarchyEntry {
                        name: "ParentTrait".to_string(),
                        location: Some(("src/traits.rs".to_string(), 1)),
                        external_module: Some("core".to_string()),
                    },
                    HierarchyEntry {
                        name: "ExternalParent".to_string(),
                        location: None,
                        external_module: Some("serde".to_string()),
                    },
                ],
                children: vec![
                    HierarchyEntry {
                        name: "ChildImpl".to_string(),
                        location: Some(("src/impl.rs".to_string(), 30)),
                        external_module: None,
                    },
                    HierarchyEntry {
                        name: "ExternalChild".to_string(),
                        location: None,
                        external_module: None,
                    },
                ],
            }),
            plain: "why result plain".to_string(),
        };
        print_why_result(&result, true, std::time::Instant::now());
    }

    #[test]
    fn print_why_result_many_call_sites() {
        use crate::why::{CallSite, WhyResult};
        let call_sites: Vec<CallSite> = (0..15)
            .map(|i| CallSite {
                file: format!("src/file_{}.rs", i),
                line: i + 1,
                context: format!("call_{}", i),
                caller: if i % 2 == 0 {
                    Some(format!("caller_{}", i))
                } else {
                    None
                },
            })
            .collect();
        let result = WhyResult {
            symbol: make_test_symbol(),
            full_definition: "fn test_fn() {}".to_string(),
            doc_comment: None,
            call_sites,
            dependencies: vec![],
            hierarchy: None,
            plain: "plain".to_string(),
        };
        print_why_result(&result, true, std::time::Instant::now());
    }

    #[test]
    fn print_why_result_many_deps() {
        use crate::why::{Dependency, WhyResult};
        let dependencies: Vec<Dependency> = (0..20)
            .map(|i| Dependency {
                name: format!("Dep{}", i),
                kind: if i % 3 == 0 {
                    Some(crate::symbol::SymbolKind::Function)
                } else {
                    None
                },
                location: if i % 2 == 0 {
                    Some((format!("src/dep_{}.rs", i), i + 1))
                } else {
                    None
                },
                import_from: if i % 2 != 0 {
                    Some(format!("mod_{}", i))
                } else {
                    None
                },
            })
            .collect();
        let result = WhyResult {
            symbol: make_test_symbol(),
            full_definition: "fn test_fn() {}".to_string(),
            doc_comment: None,
            call_sites: vec![],
            dependencies,
            hierarchy: None,
            plain: "plain".to_string(),
        };
        print_why_result(&result, true, std::time::Instant::now());
    }

    #[test]
    fn print_why_result_long_definition() {
        use crate::why::WhyResult;
        let long_def = (0..30)
            .map(|i| format!("    line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = WhyResult {
            symbol: make_test_symbol(),
            full_definition: long_def,
            doc_comment: None,
            call_sites: vec![],
            dependencies: vec![],
            hierarchy: None,
            plain: "plain".to_string(),
        };
        print_why_result(&result, true, std::time::Instant::now());
    }

    // ── print_ctx_result ────────────────────────────────────────

    #[test]
    fn print_ctx_result_basic() {
        let result = AnalysisResult {
            plain: "ctx plain output".to_string(),
            file_count: 3,
            total_lines: 150,
            total_bytes: 4096,
            original_bytes: 8192,
            dep_file_count: 2,
            used_by_count: 5,
        };
        print_ctx_result(&result, true, std::time::Instant::now());
    }

    // ── print_context_result ────────────────────────────────────

    #[test]
    fn print_context_result_basic() {
        let result = AnalysisResult {
            plain: "context plain output".to_string(),
            file_count: 1,
            total_lines: 50,
            total_bytes: 1024,
            original_bytes: 2048,
            dep_file_count: 0,
            used_by_count: 1,
        };
        print_context_result(&result, true, std::time::Instant::now());
    }

    // ── print_pick_stats ────────────────────────────────────────

    #[test]
    fn print_pick_stats_basic() {
        let result = AnalysisResult {
            plain: "pick plain output".to_string(),
            file_count: 2,
            total_lines: 75,
            total_bytes: 2048,
            original_bytes: 4096,
            dep_file_count: 1,
            used_by_count: 3,
        };
        print_pick_stats(&result, true, std::time::Instant::now());
    }

    // ── print_clipboard_status ──────────────────────────────────

    #[test]
    fn print_clipboard_status_empty_text() {
        // Should return without printing anything
        print_clipboard_status("", true);
        print_clipboard_status("", false);
    }

    // ── format_size boundary ────────────────────────────────────

    #[test]
    fn format_size_large_mb() {
        let result = format_size(500_000_000);
        assert!(result.contains("MB"));
    }

    #[test]
    fn format_size_multi_gb() {
        let result = format_size(5_000_000_000);
        assert!(result.contains("GB"));
    }

    // ── estimate_tokens larger values ───────────────────────────

    #[test]
    fn estimate_tokens_large() {
        let tokens = estimate_tokens(30000);
        assert_eq!(tokens, 10000);
    }

    // ── format_number edge cases ────────────────────────────────

    #[test]
    fn format_number_100() {
        assert_eq!(format_number(100), "100");
    }

    #[test]
    fn format_number_10000() {
        assert_eq!(format_number(10000), "10,000");
    }

    #[test]
    fn format_number_100000000() {
        assert_eq!(format_number(100_000_000), "100,000,000");
    }

    // ── print_diff_result with files ────────────────────────────

    #[test]
    fn print_diff_result_with_files_extended() {
        let result = DiffResult {
            label: "working tree".to_string(),
            files: vec![
                FileEntry {
                    path: "src/main.rs".to_string(),
                    old_path: None,
                    status: DeltaStatus::Modified,
                    additions: 10,
                    deletions: 3,
                    patch: "some patch".to_string(),
                },
                FileEntry {
                    path: "src/lib.rs".to_string(),
                    old_path: None,
                    status: DeltaStatus::Added,
                    additions: 20,
                    deletions: 0,
                    patch: "another patch".to_string(),
                },
            ],
            text: "diff output".to_string(),
            has_conflicts: false,
            is_branch_comparison: false,
            commit_count: Some(1),
            stale_check: None,
        };
        print_diff_result(result, true, std::time::Instant::now());
    }

    // ── print_why_result ────────────────────────────────────────

    #[test]
    fn print_why_result_minimal() {
        use crate::symbol::{Symbol, SymbolKind};
        use crate::why::WhyResult;
        let result = WhyResult {
            symbol: Symbol {
                name: "foo".to_string(),
                kind: SymbolKind::Function,
                file: "test.rs".to_string(),
                line: 1,
                signature: "fn foo()".to_string(),
                parent: None,
                keywords: vec![],
            },
            doc_comment: None,
            full_definition: "fn foo() { 42 }".to_string(),
            call_sites: vec![],
            dependencies: vec![],
            hierarchy: None,
            plain: "plain text".to_string(),
        };
        print_why_result(&result, true, std::time::Instant::now());
    }

    #[test]
    fn print_why_result_with_all_sections_extended() {
        use crate::symbol::{Symbol, SymbolKind};
        use crate::why::{CallSite, Dependency, Hierarchy, HierarchyEntry, WhyResult};
        let result = WhyResult {
            symbol: Symbol {
                name: "Child".to_string(),
                kind: SymbolKind::Class,
                file: "test.py".to_string(),
                line: 1,
                signature: "class Child(Parent):".to_string(),
                parent: None,
                keywords: vec![],
            },
            doc_comment: Some("A child class.".to_string()),
            full_definition: "class Child(Parent):\n    def method(self):\n        pass"
                .to_string(),
            call_sites: vec![CallSite {
                file: "main.py".to_string(),
                line: 10,
                context: "obj = Child()".to_string(),
                caller: Some("create_obj".to_string()),
            }],
            dependencies: vec![Dependency {
                name: "Parent".to_string(),
                kind: Some(SymbolKind::Class),
                location: Some(("base.py".to_string(), 1)),
                import_from: None,
            }],
            hierarchy: Some(Hierarchy {
                parents: vec![
                    HierarchyEntry {
                        name: "Parent".to_string(),
                        location: Some(("base.py".to_string(), 1)),
                        external_module: None,
                    },
                    HierarchyEntry {
                        name: "ExternalBase".to_string(),
                        location: None,
                        external_module: Some("external_lib".to_string()),
                    },
                    HierarchyEntry {
                        name: "Unknown".to_string(),
                        location: None,
                        external_module: None,
                    },
                ],
                children: vec![
                    HierarchyEntry {
                        name: "GrandChild".to_string(),
                        location: Some(("gc.py".to_string(), 1)),
                        external_module: None,
                    },
                    HierarchyEntry {
                        name: "Orphan".to_string(),
                        location: None,
                        external_module: None,
                    },
                ],
            }),
            plain: "plain text".to_string(),
        };
        print_why_result(&result, true, std::time::Instant::now());
    }

    #[test]
    fn print_why_result_with_parent_symbol() {
        use crate::symbol::{Symbol, SymbolKind};
        use crate::why::WhyResult;
        let result = WhyResult {
            symbol: Symbol {
                name: "method".to_string(),
                kind: SymbolKind::Method,
                file: "test.rs".to_string(),
                line: 5,
                signature: "fn method(&self)".to_string(),
                parent: Some("MyStruct".to_string()),
                keywords: vec![],
            },
            doc_comment: None,
            full_definition: "fn method(&self) { }".to_string(),
            call_sites: vec![],
            dependencies: vec![],
            hierarchy: None,
            plain: "text".to_string(),
        };
        print_why_result(&result, true, std::time::Instant::now());
    }

    #[test]
    fn print_why_result_long_definition_truncated() {
        use crate::symbol::{Symbol, SymbolKind};
        use crate::why::WhyResult;
        let long_def = (0..50)
            .map(|i| format!("    line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = WhyResult {
            symbol: Symbol {
                name: "big_fn".to_string(),
                kind: SymbolKind::Function,
                file: "test.rs".to_string(),
                line: 1,
                signature: "fn big_fn()".to_string(),
                parent: None,
                keywords: vec![],
            },
            doc_comment: None,
            full_definition: long_def,
            call_sites: vec![],
            dependencies: vec![],
            hierarchy: None,
            plain: "text".to_string(),
        };
        print_why_result(&result, true, std::time::Instant::now());
    }

    // ── print_tree_result ───────────────────────────────────────

    #[test]
    fn print_tree_result_empty() {
        let result = TreeResult {
            display: String::new(),
            plain: String::new(),
            file_count: 0,
            dir_count: 0,
            status_counts: std::collections::HashMap::new(),
        };
        print_tree_result(result, ".", true, std::time::Instant::now());
    }

    #[test]
    fn print_tree_result_singular() {
        let result = TreeResult {
            display: "test/\n└── file.txt\n".to_string(),
            plain: "test/\n└── file.txt\n".to_string(),
            file_count: 1,
            dir_count: 1,
            status_counts: std::collections::HashMap::new(),
        };
        print_tree_result(result, "test", true, std::time::Instant::now());
    }

    #[test]
    fn print_tree_result_with_statuses() {
        let mut status_counts = std::collections::HashMap::new();
        status_counts.insert(FileStatus::Modified, 2);
        status_counts.insert(FileStatus::Added, 1);
        status_counts.insert(FileStatus::Untracked, 1);
        status_counts.insert(FileStatus::Renamed, 1);
        let result = TreeResult {
            display: "test/\n├── a.rs\n└── b.rs\n".to_string(),
            plain: "test/\n├── a.rs\n└── b.rs\n".to_string(),
            file_count: 5,
            dir_count: 2,
            status_counts,
        };
        print_tree_result(result, "test", true, std::time::Instant::now());
    }

    // ── print_summary edge cases ────────────────────────────────

    #[test]
    fn print_summary_narrow_col() {
        let files = vec![FileEntry {
            path: "x.rs".to_string(),
            old_path: None,
            status: DeltaStatus::Modified,
            additions: 1,
            deletions: 1,
            patch: String::new(),
        }];
        // Very small global_max_name_col to test the padding edge case
        print_summary(&files, 5, 2, 2);
    }
}
