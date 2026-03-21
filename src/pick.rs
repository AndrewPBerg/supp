use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Result, bail};
use colored::Colorize;
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::terminal;
use ignore::WalkBuilder;
use regex::Regex;

/// Collect all files under `root`, respecting .gitignore, with optional regex filter.
pub fn collect_files(root: &str, regex: Option<&str>) -> Result<Vec<String>> {
    let re = regex.map(Regex::new).transpose()?;
    let mut files = Vec::new();

    let walker = WalkBuilder::new(root)
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
            files.push(rel);
        }
    }

    Ok(files)
}

/// Build the fzf argument list. Extracted for testability.
fn build_fzf_args(multi: bool, preview_lines: usize) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if multi {
        args.extend([
            "--multi".into(),
            "--bind".into(),
            "space:toggle".into(),
            "--bind".into(),
            "enter:toggle".into(),
            "--bind".into(),
            "tab:accept".into(),
            "--bind".into(),
            "esc:abort".into(),
            "--header".into(),
            "space/enter: select | tab: confirm | esc: cancel".into(),
        ]);
    }
    args.push("--preview".into());
    args.push(format!("head -{preview_lines} {{}}"));
    args
}

/// Spawn fzf with the collected file list, return selected paths.
pub fn run_fzf(root: &str, multi: bool, regex: Option<&str>, preview_lines: usize) -> Result<Vec<String>> {
    let files = collect_files(root, regex)?;
    if files.is_empty() {
        bail!("no files found under '{}'", root);
    }

    let args = build_fzf_args(multi, preview_lines);

    let mut child = match Command::new("fzf")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "fzf is not installed. Install it to use `supp pick`:\n  \
                 Arch: pacman -S fzf\n  \
                 Ubuntu/Debian: apt install fzf\n  \
                 macOS: brew install fzf\n  \
                 https://github.com/junegunn/fzf#installation"
            );
        }
        Err(e) => return Err(e.into()),
    };

    // Write file list to fzf's stdin
    if let Some(mut stdin) = child.stdin.take() {
        let input = files.join("\n");
        stdin.write_all(input.as_bytes())?;
    }

    let output = child.wait_with_output()?;

    match output.status.code() {
        Some(0) => {}
        Some(130) | Some(1) => {
            // User pressed Esc/Ctrl-C or no match — clean exit
            return Ok(Vec::new());
        }
        Some(code) => {
            bail!("fzf exited with code {}", code);
        }
        None => {
            bail!("fzf was terminated by a signal");
        }
    }

    let selected: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();

    Ok(selected)
}

// ── Terminal key input ───────────────────────────────────────────

/// Read a single key press using crossterm's raw mode.
/// Returns the KeyCode. Handles raw mode enable/disable with cleanup on drop.
fn read_single_key() -> Result<KeyCode> {
    terminal::enable_raw_mode()?;
    let result = (|| {
        loop {
            if event::poll(Duration::from_secs(60))?
                && let Event::Key(KeyEvent { code, .. }) = event::read()? {
                    return Ok(code);
                }
        }
    })();
    terminal::disable_raw_mode()?;
    result
}

// ── Accumulation UI ─────────────────────────────────────────────

/// Merge new files into accumulated list, deduplicating.
pub fn merge_unique(accumulated: &mut Vec<String>, new: Vec<String>) {
    for f in new {
        if !accumulated.contains(&f) {
            accumulated.push(f);
        }
    }
}

/// Interactive accumulation loop. Shows accumulated files and lets user pick more, execute, or cancel.
/// After each fzf session, selected files are merged into the accumulator and the user can
/// pick more (p), execute (enter), or cancel (esc).
pub fn interactive_pick_loop(
    root: &str,
    regex: Option<&str>,
    preview_lines: usize,
    initial: Vec<String>,
) -> Result<Vec<String>> {
    let mut accumulated = initial;

    loop {
        eprintln!();
        eprintln!("{}", "  Accumulated files:".bold());
        for (i, f) in accumulated.iter().enumerate() {
            eprintln!("    {} {}", format!("{}.", i + 1).dimmed(), f.cyan());
        }
        eprintln!();
        eprint!("{}", "  p: pick more | enter: execute | esc: cancel ".dimmed());
        io::stderr().flush()?;

        let key = read_single_key()?;
        eprintln!();

        match key {
            KeyCode::Char('p' | 'P') => {
                let more = run_fzf(root, true, regex, preview_lines)?;
                if !more.is_empty() {
                    merge_unique(&mut accumulated, more);
                }
            }
            KeyCode::Enter => {
                return Ok(accumulated);
            }
            KeyCode::Esc => {
                return Ok(Vec::new());
            }
            _ => {}
        }
    }
}

/// Expand `"p"` tokens in a list of paths by launching fzf for each one.
/// Non-`"p"` tokens are passed through unchanged.
pub fn expand_p_tokens(
    paths: &[String],
    regex: Option<&str>,
    preview_lines: usize,
) -> Result<Vec<String>> {
    let mut result = Vec::new();
    for path in paths {
        if path == "p" {
            let picked = run_fzf(".", true, regex, preview_lines)?;
            result.extend(picked);
        } else {
            result.push(path.clone());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup(files: &[&str]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for name in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, "content").unwrap();
        }
        dir
    }

    #[test]
    fn collect_files_finds_all() {
        let dir = setup(&["a.rs", "b.rs", "sub/c.rs"]);
        let files = collect_files(dir.path().to_str().unwrap(), None).unwrap();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn collect_files_regex_filter() {
        let dir = setup(&["main.rs", "lib.rs", "readme.md"]);
        let files = collect_files(dir.path().to_str().unwrap(), Some(r"\.rs$")).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f.ends_with(".rs")));
    }

    #[test]
    fn collect_files_empty_dir() {
        let dir = TempDir::new().unwrap();
        let files = collect_files(dir.path().to_str().unwrap(), None).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn collect_files_invalid_regex() {
        let dir = TempDir::new().unwrap();
        let result = collect_files(dir.path().to_str().unwrap(), Some("[invalid"));
        assert!(result.is_err());
    }

    #[test]
    fn fzf_args_single_mode() {
        let args = build_fzf_args(false, 50);
        assert!(!args.contains(&"--multi".to_string()));
        assert!(args.contains(&"--preview".to_string()));
        assert!(args.contains(&"head -50 {}".to_string()));
    }

    #[test]
    fn fzf_args_multi_mode() {
        let args = build_fzf_args(true, 30);
        assert!(args.contains(&"--multi".to_string()));
        assert!(args.contains(&"space:toggle".to_string()));
        assert!(args.contains(&"enter:toggle".to_string()));
        assert!(args.contains(&"tab:accept".to_string()));
        assert!(args.contains(&"esc:abort".to_string()));
        assert!(args.contains(&"space/enter: select | tab: confirm | esc: cancel".to_string()));
    }

    #[test]
    fn merge_unique_deduplicates() {
        let mut acc = vec!["a.rs".into(), "b.rs".into()];
        merge_unique(&mut acc, vec!["b.rs".into(), "c.rs".into()]);
        assert_eq!(acc, vec!["a.rs", "b.rs", "c.rs"]);
    }

    #[test]
    fn merge_unique_empty() {
        let mut acc = vec!["a.rs".into()];
        merge_unique(&mut acc, vec![]);
        assert_eq!(acc, vec!["a.rs"]);
    }

    #[test]
    fn merge_unique_into_empty() {
        let mut acc: Vec<String> = vec![];
        merge_unique(&mut acc, vec!["a.rs".into()]);
        assert_eq!(acc, vec!["a.rs"]);
    }
}
