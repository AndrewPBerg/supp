use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Result, bail};
use colored::Colorize;
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::terminal;
use ignore::WalkBuilder;
use regex::Regex;

const MAX_HISTORY: usize = 20;

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
    args.push(format!(
        r#"case {{}} in "[hist "*)  echo {{}} | sed 's/\[hist [0-9]*\] //' | tr ', ' '\n' ;; *) head -{preview_lines} {{}} ;; esac"#
    ));
    args
}

/// Spawn fzf, return selected paths.
fn spawn_fzf(input: &str, args: &[String]) -> Result<Vec<String>> {
    let mut child = match Command::new("fzf")
        .args(args)
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

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes())?;
    }

    let output = child.wait_with_output()?;

    match output.status.code() {
        Some(0) => {}
        Some(130) | Some(1) => return Ok(Vec::new()),
        Some(code) => bail!("fzf exited with code {}", code),
        None => bail!("fzf was terminated by a signal"),
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

const HIST_PREFIX: &str = "[hist ";

/// Format a history entry as a single fzf-selectable line.
fn format_history_line(index: usize, files: &[String]) -> String {
    format!("{}{}] {}", HIST_PREFIX, index + 1, files.join(", "))
}

/// Parse a selected fzf line — if it's a history entry, return the expanded file list.
fn parse_history_line(line: &str) -> Option<Vec<String>> {
    if !line.starts_with(HIST_PREFIX) {
        return None;
    }
    let rest = line.strip_prefix(HIST_PREFIX)?;
    let (_, files_part) = rest.split_once("] ")?;
    Some(files_part.split(", ").map(String::from).collect())
}

/// Spawn fzf with the collected file list, return selected paths.
pub fn run_fzf(root: &str, multi: bool, regex: Option<&str>, preview_lines: usize) -> Result<Vec<String>> {
    run_fzf_with_history(root, multi, regex, preview_lines, &[])
}

/// Spawn fzf with history entries at the top, then regular files below.
pub fn run_fzf_with_history(
    root: &str,
    multi: bool,
    regex: Option<&str>,
    preview_lines: usize,
    history: &[Vec<String>],
) -> Result<Vec<String>> {
    let files = collect_files(root, regex)?;
    if files.is_empty() && history.is_empty() {
        bail!("no files found under '{}'", root);
    }

    let args = build_fzf_args(multi, preview_lines);

    // Build input: history entries (newest first) then files
    let mut lines = Vec::new();
    for (i, entry) in history.iter().enumerate().rev() {
        lines.push(format_history_line(i, entry));
    }
    lines.extend(files);
    let input = lines.join("\n");

    let raw_selected = spawn_fzf(&input, &args)?;

    // Expand any history lines back to their file lists
    let mut result = Vec::new();
    for line in raw_selected {
        if let Some(expanded) = parse_history_line(&line) {
            for f in expanded {
                if !result.contains(&f) {
                    result.push(f);
                }
            }
        } else {
            if !result.contains(&line) {
                result.push(line);
            }
        }
    }

    Ok(result)
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

// ── Pick history ────────────────────────────────────────────────

fn history_path(root: &Path) -> PathBuf {
    if let Ok(repo) = gix::discover(root) {
        let git_dir = repo.git_dir().to_path_buf();
        return git_dir.join("supp").join("pick-history");
    }
    let mut hasher = 0u64;
    for b in root.to_string_lossy().bytes() {
        hasher = hasher.wrapping_mul(31).wrapping_add(b as u64);
    }
    PathBuf::from(format!("/tmp/supp-pick-{:x}", hasher))
}

/// Load history entries (newest last). Silently skips malformed lines.
fn load_history(path: &Path) -> Vec<Vec<String>> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Vec<String>>(&line).ok())
        .collect()
}

/// Append `selection` to history, skipping if it duplicates the last entry. Bounds to MAX_HISTORY.
fn save_history(path: &Path, history: &mut Vec<Vec<String>>, selection: &[String]) {
    if selection.is_empty() {
        return;
    }
    // Deduplicate: remove any existing identical entry
    history.retain(|entry| entry != selection);
    history.push(selection.to_vec());
    // Bound
    if history.len() > MAX_HISTORY {
        *history = history.split_off(history.len() - MAX_HISTORY);
    }
    // Write
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut out = Vec::new();
    for entry in history.iter() {
        if let Ok(json) = serde_json::to_string(entry) {
            out.push(json);
        }
    }
    let _ = std::fs::write(path, out.join("\n") + "\n");
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

/// Interactive pick flow. Opens fzf with history entries at the top, then enters
/// an accumulation loop (pick more / execute / cancel). History is saved on execute.
pub fn interactive_pick_loop(
    root: &str,
    regex: Option<&str>,
    preview_lines: usize,
) -> Result<Vec<String>> {
    let hist_path = history_path(Path::new("."));
    let mut history = load_history(&hist_path);

    // First fzf session — includes history
    let mut accumulated = run_fzf_with_history(root, true, regex, preview_lines, &history)?;
    if accumulated.is_empty() {
        return Ok(Vec::new());
    }

    let mut need_redraw = true;

    loop {
        if need_redraw {
            eprintln!();
            eprintln!("{}", "  Accumulated files:".bold());
            for (i, f) in accumulated.iter().enumerate() {
                eprintln!("    {} {}", format!("{}.", i + 1).dimmed(), f.cyan());
            }
            eprintln!();
            eprint!("{}", "  p: pick more | enter: execute | esc: cancel ".dimmed());
            io::stderr().flush()?;
            need_redraw = false;
        }

        let key = read_single_key()?;

        match key {
            KeyCode::Char('p' | 'P') => {
                eprintln!();
                let more = run_fzf_with_history(root, true, regex, preview_lines, &history)?;
                if !more.is_empty() {
                    merge_unique(&mut accumulated, more);
                }
                need_redraw = true;
            }
            KeyCode::Enter => {
                eprintln!();
                save_history(&hist_path, &mut history, &accumulated);
                return Ok(accumulated);
            }
            KeyCode::Esc => {
                eprintln!();
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
        assert!(args.iter().any(|a| a.contains("head -50")));
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

    #[test]
    fn history_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pick-history");

        let sel1 = vec!["a.rs".to_string(), "b.rs".to_string()];
        let sel2 = vec!["c.rs".to_string()];

        let mut history = Vec::new();
        save_history(&path, &mut history, &sel1);
        save_history(&path, &mut history, &sel2);

        let loaded = load_history(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0], sel1);
        assert_eq!(loaded[1], sel2);
    }

    #[test]
    fn history_deduplicates() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pick-history");

        let sel = vec!["a.rs".to_string()];
        let mut history = Vec::new();
        save_history(&path, &mut history, &sel);
        save_history(&path, &mut history, &sel);

        let loaded = load_history(&path);
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn history_bounds_to_max() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pick-history");

        let mut history = Vec::new();
        for i in 0..25 {
            save_history(&path, &mut history, &[format!("file{i}.rs")]);
        }

        let loaded = load_history(&path);
        assert_eq!(loaded.len(), MAX_HISTORY);
        // Most recent should be last
        assert_eq!(loaded.last().unwrap(), &vec!["file24.rs".to_string()]);
    }

    #[test]
    fn history_skips_empty_selection() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pick-history");

        let mut history = Vec::new();
        save_history(&path, &mut history, &[]);

        assert!(!path.exists());
    }

    #[test]
    fn history_skips_malformed_lines() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pick-history");
        fs::write(&path, "[\"a.rs\"]\nnot json\n[\"b.rs\"]\n").unwrap();

        let loaded = load_history(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0], vec!["a.rs".to_string()]);
        assert_eq!(loaded[1], vec!["b.rs".to_string()]);
    }

    #[test]
    fn history_line_roundtrip() {
        let files = vec!["a.rs".to_string(), "b.rs".to_string()];
        let line = format_history_line(0, &files);
        assert_eq!(line, "[hist 1] a.rs, b.rs");
        let parsed = parse_history_line(&line).unwrap();
        assert_eq!(parsed, files);
    }

    #[test]
    fn parse_history_line_not_history() {
        assert!(parse_history_line("./src/main.rs").is_none());
        assert!(parse_history_line("").is_none());
    }
}
