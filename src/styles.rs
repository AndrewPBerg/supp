use std::collections::BTreeMap;

use colored::Colorize;
use git2::Delta;

use crate::git::{DiffResult, FileEntry, FileStatus};
use crate::tree::TreeResult;

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
            child.wait()?;
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
        FileStatus::Modified  => ("[M]", "[M]".yellow().to_string()),
        FileStatus::Added     => ("[A]", "[A]".green().to_string()),
        FileStatus::Deleted   => ("[D]", "[D]".red().to_string()),
        FileStatus::Renamed   => ("[R]", "[R]".cyan().to_string()),
        FileStatus::Untracked => ("[?]", "[?]".dimmed().to_string()),
    }
}

// ── Diff display ───────────────────────────────────────────────────

fn status_label(delta: Delta) -> colored::ColoredString {
    match delta {
        Delta::Added | Delta::Untracked => " added   ".green(),
        Delta::Deleted => " deleted ".red(),
        Delta::Modified => " modified".yellow(),
        Delta::Renamed => " renamed ".cyan(),
        Delta::Copied => " copied  ".cyan(),
        _ => " changed ".white(),
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

fn print_summary(files: &[FileEntry], global_max_name_col: usize, max_add_w: usize, max_del_w: usize) {
    let added = files
        .iter()
        .filter(|f| matches!(f.status, Delta::Added | Delta::Untracked))
        .count();
    let modified = files.iter().filter(|f| f.status == Delta::Modified).count();
    let deleted = files.iter().filter(|f| f.status == Delta::Deleted).count();
    let renamed = files.iter().filter(|f| f.status == Delta::Renamed).count();

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
    let detail_pad = if 9 > detail_plain.len() { 9 - detail_plain.len() } else { 0 };
    let detail_aligned = format!("{}{}", " ".repeat(detail_pad), detail);

    let add_str = format!(
        "{:>width$}",
        format!("+{}", total_adds),
        width = max_add_w
    );
    let del_str = format!(
        "{:>width$}",
        format!("-{}", total_dels),
        width = max_del_w
    );

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

pub fn print_diff_result(result: DiffResult, no_copy: bool, start: std::time::Instant, token_handle: std::thread::JoinHandle<Option<usize>>) {
    println!();
    println!("  {}  {}", "supp diff".bold().cyan(), result.label.dimmed());
    println!("  {}", "─".repeat(40).dimmed());

    if result.is_branch_comparison {
        if result.has_conflicts {
            println!("  {}", "✗ Merge conflicts detected".red().bold());
        } else {
            println!("  {}", "✓ No merge conflicts".green());
        }
    }

    if result.files.is_empty() {
        println!("  {}", "No changes found.".dimmed());
    } else {
        println!();
        let (name_col, add_w, del_w) = print_file_tree(&result.files);
        print_summary(&result.files, name_col, add_w, del_w);
        println!();

        if no_copy {
            println!(
                "  {} {}",
                "–".dimmed(),
                format!("({}, not copied)", format_size(result.text.len())).dimmed(),
            );
        } else {
            match copy_to_clipboard(&result.text) {
                Ok(()) => {
                    println!(
                        "  {} {} {}",
                        "✓".green().bold(),
                        "Copied to clipboard".green(),
                        format!("({})", format_size(result.text.len())).dimmed(),
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
    if let Some(rx) = result.stale_check
        && let Ok(true) = rx.recv_timeout(std::time::Duration::from_millis(300))
    {
        println!(
            "  {} {}",
            "⚠".yellow().bold(),
            format!("{} has new commits — re-run for latest", result.label.split(" ... ").next().unwrap_or(&result.label)).yellow()
        );
    }
    if let Some(count) = token_handle.join().ok().flatten() {
        println!(
            "  {} {}",
            "≈".dimmed(),
            format!("~{} tokens (cl100k est.)", format_number(count)).dimmed(),
        );
    }
    println!("  {}", format!("Done in {}", format_elapsed(start.elapsed())).dimmed());
    println!();
}

// ── Tree display ───────────────────────────────────────────────────

pub fn print_tree_result(result: TreeResult, root: &str, no_copy: bool, start: std::time::Instant, token_handle: std::thread::JoinHandle<Option<usize>>) {
    println!();
    println!("  {}  {}", "supp tree".bold().cyan(), root.dimmed());
    println!("  {}", "─".repeat(40).dimmed());
    println!();

    for line in result.display.lines() {
        println!("  {}", line);
    }

    let dir_s = if result.dir_count == 1 { "directory" } else { "directories" };
    let file_s = if result.file_count == 1 { "file" } else { "files" };

    // Build status summary parts
    let mut status_parts: Vec<String> = Vec::new();
    let modified = result.status_counts.get(&FileStatus::Modified).copied().unwrap_or(0);
    let added = result.status_counts.get(&FileStatus::Added).copied().unwrap_or(0);
    let untracked = result.status_counts.get(&FileStatus::Untracked).copied().unwrap_or(0);
    let renamed = result.status_counts.get(&FileStatus::Renamed).copied().unwrap_or(0);

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

    if no_copy {
        println!(
            "  {} {}",
            "–".dimmed(),
            format!("({}, not copied)", format_size(result.plain.len())).dimmed(),
        );
    } else {
        match copy_to_clipboard(&result.plain) {
            Ok(()) => {
                println!(
                    "  {} {} {}",
                    "✓".green().bold(),
                    "Copied to clipboard".green(),
                    format!("({})", format_size(result.plain.len())).dimmed(),
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
    if let Some(count) = token_handle.join().ok().flatten() {
        println!(
            "  {} {}",
            "≈".dimmed(),
            format!("~{} tokens (cl100k est.)", format_number(count)).dimmed(),
        );
    }
    println!("  {}", format!("Done in {}", format_elapsed(start.elapsed())).dimmed());
    println!();
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
}
