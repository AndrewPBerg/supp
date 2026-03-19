use std::collections::HashMap;

use anyhow::{anyhow, Result};
use git2::{Delta, DiffFormat, Repository};

pub struct DiffOptions {
    pub cached: bool,
    pub untracked: bool,
    pub local: bool,
    pub branch: Option<String>,
}

pub struct FileEntry {
    pub path: String,
    pub old_path: Option<String>,
    pub status: Delta,
    pub additions: usize,
    pub deletions: usize,
}

pub struct DiffResult {
    pub label: String,
    pub files: Vec<FileEntry>,
    pub text: String,
}

/// Collects the patch text and per-file line counts in a single pass, then
/// merges them with the delta metadata to produce `FileEntry` list.
fn collect_diff_data(diff: &git2::Diff) -> (Vec<FileEntry>, String) {
    let mut text = String::new();
    let mut line_counts: HashMap<String, (usize, usize)> = HashMap::new();

    let _ = diff.print(DiffFormat::Patch, |delta, _hunk, line| {
        if let Ok(s) = std::str::from_utf8(line.content()) {
            text.push_str(s);
        }
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let counts = line_counts.entry(path).or_insert((0, 0));
        match line.origin() {
            '+' => counts.0 += 1,
            '-' => counts.1 += 1,
            _ => {}
        }
        true
    });

    let files = diff
        .deltas()
        .map(|delta| {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let old_path = if delta.status() == Delta::Renamed {
                delta
                    .old_file()
                    .path()
                    .map(|p| p.to_string_lossy().into_owned())
            } else {
                None
            };
            let (additions, deletions) = line_counts.get(&path).copied().unwrap_or((0, 0));
            FileEntry {
                path,
                old_path,
                status: delta.status(),
                additions,
                deletions,
            }
        })
        .collect();

    (files, text)
}

pub fn get_diff(repo_path: &str, opts: DiffOptions) -> Result<DiffResult> {
    let repo = Repository::open(repo_path)?;

    if opts.untracked {
        let statuses = repo.statuses(None)?;
        let mut files = Vec::new();
        let mut text = String::new();
        for entry in statuses.iter() {
            if entry.status().contains(git2::Status::WT_NEW) {
                if let Some(path) = entry.path() {
                    let full_path = std::path::Path::new(repo_path).join(path);
                    let mut additions = 0usize;
                    if let Ok(content) = std::fs::read_to_string(&full_path) {
                        text.push_str(&format!("--- /dev/null\n+++ b/{}\n", path));
                        for line in content.lines() {
                            text.push_str(&format!("+{}\n", line));
                            additions += 1;
                        }
                    }
                    files.push(FileEntry {
                        path: path.to_string(),
                        old_path: None,
                        status: Delta::Untracked,
                        additions,
                        deletions: 0,
                    });
                }
            }
        }
        return Ok(DiffResult {
            label: "Untracked files".into(),
            files,
            text,
        });
    }

    if opts.cached {
        let head = repo.head()?;
        let head_commit = head.peel_to_commit()?;
        let head_tree = head_commit.tree()?;
        let diff = repo.diff_tree_to_index(Some(&head_tree), None, None)?;
        let (files, text) = collect_diff_data(&diff);
        let branch = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(|s| s.to_string()))
            .unwrap_or_else(|| "HEAD".into());
        return Ok(DiffResult {
            label: format!("Staged  HEAD ... index  ({})", branch),
            files,
            text,
        });
    }

    if opts.local {
        let diff = repo.diff_index_to_workdir(None, None)?;
        let (files, text) = collect_diff_data(&diff);
        return Ok(DiffResult {
            label: "Local changes (unstaged)".into(),
            files,
            text,
        });
    }

    // Remote diff (default): compare current branch against the default branch (main/master)
    let current_branch = {
        let head = repo.head()?;
        head.shorthand()
            .ok_or_else(|| anyhow!("Could not determine current branch"))?
            .to_string()
    };

    let base_branch = match opts.branch {
        Some(b) => b,
        None => {
            repo.find_reference("refs/remotes/origin/HEAD")
                .ok()
                .and_then(|r| r.symbolic_target().map(|s| s.to_string()))
                .and_then(|target| {
                    target.strip_prefix("refs/remotes/origin/").map(|s| s.to_string())
                })
                .or_else(|| {
                    for candidate in &["main", "master"] {
                        let refname = format!("refs/remotes/origin/{}", candidate);
                        if repo.find_reference(&refname).is_ok() {
                            return Some(candidate.to_string());
                        }
                    }
                    None
                })
                .ok_or_else(|| anyhow!("Could not determine default branch; use -b to specify one"))?
        }
    };

    {
        let config = repo.config()?;
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(|url, username, allowed| {
            if allowed.contains(git2::CredentialType::SSH_KEY) {
                git2::Cred::ssh_key_from_agent(username.unwrap_or("git"))
            } else if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                git2::Cred::credential_helper(&config, url, username)
            } else {
                git2::Cred::default()
            }
        });
        let mut fetch_opts = git2::FetchOptions::new();
        fetch_opts.remote_callbacks(callbacks);
        let mut remote = repo.find_remote("origin")?;
        remote.fetch(&[base_branch.as_str()], Some(&mut fetch_opts), None)?;
    }

    let base_ref = repo.find_reference(&format!("refs/remotes/origin/{}", base_branch))?;
    let base_commit = base_ref.peel_to_commit()?;
    let base_tree = base_commit.tree()?;

    let head = repo.head()?;
    let local_commit = head.peel_to_commit()?;
    let local_tree = local_commit.tree()?;

    let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&local_tree), None)?;
    let (files, text) = collect_diff_data(&diff);

    Ok(DiffResult {
        label: format!("{} ... {}", base_branch, current_branch),
        files,
        text,
    })
}
