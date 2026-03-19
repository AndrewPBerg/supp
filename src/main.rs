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
fn print_file_tree(files: &[FileEntry]) {
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

        let file_prefix = if dir.is_empty() {
            ""
        } else if is_last_dir {
            "    "
        } else {
            "│   "
        };

        // Align filenames within this group to the longest name.
        let max_name_w = entries
            .iter()
            .map(|e| {
                std::path::Path::new(&e.path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&e.path)
                    .len()
            })
            .max()
            .unwrap_or(0);

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
                "{}{}{}{}{}{}",
                file_prefix.dimmed(),
                branch_char.dimmed(),
                format!("{:<width$}", filename, width = max_name_w),
                status_label(entry.status),
                line_delta,
                rename_hint,
            );
        }
    }
}

fn print_summary(files: &[FileEntry]) {
    let added = files
        .iter()
        .filter(|f| matches!(f.status, Delta::Added | Delta::Untracked))
        .count();
    let modified = files.iter().filter(|f| f.status == Delta::Modified).count();
    let deleted = files.iter().filter(|f| f.status == Delta::Deleted).count();
    let renamed = files.iter().filter(|f| f.status == Delta::Renamed).count();

    let total = files.len();
    let mut parts: Vec<String> = Vec::new();
    if added > 0 {
        parts.push(format!("{} added", added).green().to_string());
    }
    if modified > 0 {
        parts.push(format!("{} modified", modified).yellow().to_string());
    }
    if deleted > 0 {
        parts.push(format!("{} deleted", deleted).red().to_string());
    }
    if renamed > 0 {
        parts.push(format!("{} renamed", renamed).cyan().to_string());
    }

    let total_adds: usize = files.iter().map(|f| f.additions).sum();
    let total_dels: usize = files.iter().map(|f| f.deletions).sum();
    let detail = if parts.is_empty() {
        String::new()
    } else {
        format!("  ({})", parts.join(", "))
    };
    println!(
        "\n  {} file{}{}   {} {}",
        total.to_string().bold(),
        if total == 1 { "" } else { "s" },
        detail,
        format!("+{}", total_adds).green().bold(),
        format!("-{}", total_dels).red().bold(),
    );
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Diff {
            path,
            cached,
            untracked,
            local,
            branch,
            all,
            self_branch,
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
            };
            let result = get_diff(repo_path, opts)?;

            // Header
            println!();
            println!("  {}  {}", "supp diff".bold().cyan(), result.label.dimmed());
            println!("  {}", "─".repeat(40).dimmed());

            if result.files.is_empty() {
                println!("  {}", "No changes found.".dimmed());
            } else {
                println!();
                print_file_tree(&result.files);
                print_summary(&result.files);
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
            println!();
        }
        Commands::Tree { size, .. } => println!("tree! size: {:?}", size),
    }
    Ok(())
}
