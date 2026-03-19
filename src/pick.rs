use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Result, bail};
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

/// Spawn fzf with the collected file list, return selected paths.
pub fn run_fzf(root: &str, multi: bool, regex: Option<&str>) -> Result<Vec<String>> {
    let files = collect_files(root, regex)?;
    if files.is_empty() {
        bail!("no files found under '{}'", root);
    }

    let mut args: Vec<&str> = Vec::new();
    if multi {
        args.extend_from_slice(&[
            "--multi",
            "--bind",
            "enter:toggle",
            "--bind",
            "double-click:toggle",
            "--bind",
            "tab:accept",
            "--bind",
            "esc:deselect-all",
            "--header",
            "enter: toggle | tab: confirm | esc: clear selection",
        ]);
    }
    args.extend_from_slice(&["--preview", "head -100 {}"]);

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
        Some(130) => {
            // User pressed Esc/Ctrl-C — clean exit
            return Ok(Vec::new());
        }
        Some(1) => {
            bail!("no matches found in fzf");
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
}
