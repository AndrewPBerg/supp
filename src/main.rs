mod cli;
mod git;

use std::collections::BTreeMap;

use clap::Parser;
use colored::Colorize;
use git2::Delta;

use cli::Cli;
use cli::Commands;
use git::{DiffOptions, FileEntry, get_diff};

#[cfg(target_os = "linux")]
fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
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
fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    let mut cb = arboard::Clipboard::new()?;
    cb.set_text(text)?;
    Ok(())
}

fn format_size(bytes: usize) -> String {
    match bytes {
        b if b < 1_024 => format!("{} B", b),
        b if b < 1_048_576 => format!("{:.1} KB", b as f64 / 1_024.0),
        b if b < 1_073_741_824 => format!("{:.1} MB", b as f64 / 1_048_576.0),
        b => format!("{:.1} GB", b as f64 / 1_073_741_824.0),
    }
}

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

// Build and print a tree of file paths with their statuses.
fn print_file_tree(files: &[FileEntry]) -> (usize, usize, usize) {
    // Group into: dir_path -> Vec<(filename, entry)>
    // Top-level files go under the "" key.
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

    // Compute max digit widths for +/- columns across all files (pad before coloring).
    let max_add_w = files
        .iter()
        .map(|e| e.additions.to_string().len() + 1) // +1 for the '+' sign
        .max()
        .unwrap_or(2);
    let max_del_w = files
        .iter()
        .map(|e| e.deletions.to_string().len() + 1) // +1 for the '-' sign
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
            // Pad plain strings before applying color so ANSI codes don't skew widths.
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

    // "  {total} file{s}" left portion
    let left_visible_w = 2 + total_str.len() + 5 + suffix.len();
    // Status label column starts at global_max_name_col, label is 9 chars wide
    // Right-align the detail within the 9-char status column
    let status_col = global_max_name_col;
    let pad_to_status = if status_col > left_visible_w {
        status_col - left_visible_w
    } else {
        1
    };
    // Right-align detail within the status label width (9 chars)
    let detail_pad = if 9 > detail_plain.len() { 9 - detail_plain.len() } else { 0 };
    let detail_aligned = format!("{}{}", " ".repeat(detail_pad), detail);

    // +/- columns follow after status + 2 char gap
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

fn format_elapsed(elapsed: std::time::Duration) -> String {
    let ms = elapsed.as_secs_f64() * 1000.0;
    if ms < 1000.0 {
        format!("{:.0}ms", ms)
    } else {
        format!("{:.2}s", elapsed.as_secs_f64())
    }
}

fn main() -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    let cli = Cli::parse();

    if cli.no_color {
        colored::control::set_override(false);
    }

    match cli.command {
        Commands::Diff {
            path,
            cached,
            untracked,
            local,
            branch,
            all,
            self_branch,
            context_lines,
            filter,
        } => {
            let no_copy = cli.no_copy;
            let repo_path = path.as_deref().unwrap_or(".");
            let opts = DiffOptions {
                cached,
                untracked,
                local,
                branch,
                all,
                self_branch,
                context_lines,
                filter,
                regex: cli.regex,
            };
            let result = get_diff(repo_path, opts)?;

            // Header
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
                    copy_to_clipboard(&result.text)?;
                    println!(
                        "  {} {} {}",
                        "✓".green().bold(),
                        "Copied to clipboard".green(),
                        format!("({})", format_size(result.text.len())).dimmed(),
                    );
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
            println!("  {}", format!("Done in {}", format_elapsed(start.elapsed())).dimmed());
            println!();
        }
        Commands::Tree { size, .. } => {
            println!("tree! size: {:?}", size);
            println!("  {}", format!("Done in {}", format_elapsed(start.elapsed())).dimmed());
        }
    }
    Ok(())
}
