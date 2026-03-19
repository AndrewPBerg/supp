use std::collections::HashMap;
use std::sync::mpsc;

use anyhow::{anyhow, Result};
use git2::{Delta, DiffFormat, Repository};

const MAX_UNTRACKED_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

pub struct DiffOptions {
    pub cached: bool,
    pub untracked: bool,
    pub local: bool,
    pub branch: Option<String>,
    pub all: bool,
    pub self_branch: bool,
    pub context_lines: Option<u32>,
    pub filter: Option<String>,
    pub regex: Option<String>,
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
    pub has_conflicts: bool,
    pub is_branch_comparison: bool,
    pub stale_check: Option<mpsc::Receiver<bool>>,
}

/// Build a `git2::DiffOptions` with the given context lines setting.
fn make_diff_options(context_lines: Option<u32>) -> git2::DiffOptions {
    let mut opts = git2::DiffOptions::new();
    if let Some(lines) = context_lines {
        opts.context_lines(lines);
    }
    opts
}

/// Build `FetchOptions` with SSH / credential-helper callbacks.
fn make_fetch_options(repo: &Repository) -> Result<git2::FetchOptions<'_>> {
    let config = repo.config()?;
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |url, username, allowed| {
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
    Ok(fetch_opts)
}

/// Collect untracked files from the working directory.
fn collect_untracked_files(repo: &Repository) -> Result<(Vec<FileEntry>, String)> {
    let statuses = repo.statuses(None)?;
    let workdir = repo.workdir().ok_or_else(|| anyhow!("bare repository"))?;
    let estimated_cap = statuses.len() * 256;
    let mut files = Vec::new();
    let mut text = String::with_capacity(estimated_cap.max(4096));

    for entry in statuses.iter() {
        if entry.status().contains(git2::Status::WT_NEW) {
            if let Some(path) = entry.path() {
                let full_path = workdir.join(path);
                let mut additions = 0usize;

                // OOM guard: skip files larger than 10 MB
                let too_large = std::fs::metadata(&full_path)
                    .map(|m| m.len() > MAX_UNTRACKED_FILE_SIZE)
                    .unwrap_or(false);

                if !too_large {
                    if let Ok(content) = std::fs::read_to_string(&full_path) {
                        text.push_str(&format!("--- /dev/null\n+++ b/{}\n", path));
                        for line in content.lines() {
                            text.push_str(&format!("+{}\n", line));
                            additions += 1;
                        }
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
    Ok((files, text))
}

/// Collects the patch text and per-file line counts in a single pass, then
/// merges them with the delta metadata to produce `FileEntry` list.
fn collect_diff_data(diff: &git2::Diff) -> (Vec<FileEntry>, String) {
    let capacity = diff
        .stats()
        .map(|s| (s.insertions() + s.deletions()) * 80)
        .unwrap_or(4096);
    let mut text = String::with_capacity(capacity);
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

/// Apply glob filter to file entries, removing non-matching paths.
fn apply_filter(files: Vec<FileEntry>, filter: &str) -> Vec<FileEntry> {
    let glob = ignore::gitignore::GitignoreBuilder::new("")
        .add_line(None, filter)
        .ok()
        .and_then(|b| b.build().ok());

    match glob {
        Some(matcher) => files
            .into_iter()
            .filter(|f| {
                matcher
                    .matched_path_or_any_parents(&f.path, false)
                    .is_ignore()
            })
            .collect(),
        None => files,
    }
}

/// Apply regex filter to file entries, keeping paths that match.
fn apply_regex_filter(files: Vec<FileEntry>, pattern: &str) -> Result<Vec<FileEntry>> {
    let re = regex::Regex::new(pattern)
        .map_err(|e| anyhow!("invalid regex '{}': {}", pattern, e))?;
    Ok(files.into_iter().filter(|f| re.is_match(&f.path)).collect())
}

pub fn get_diff(repo_path: &str, opts: DiffOptions) -> Result<DiffResult> {
    let repo = Repository::discover(repo_path)?;
    let mut diff_opts = make_diff_options(opts.context_lines);
    let filter = opts.filter.clone();
    let regex = opts.regex.clone();

    let mut result = get_diff_inner(&repo, repo_path, opts, &mut diff_opts)?;

    if let Some(ref pattern) = filter {
        result.files = apply_filter(result.files, pattern);
    }
    if let Some(ref pattern) = regex {
        result.files = apply_regex_filter(result.files, pattern)?;
    }

    Ok(result)
}

fn get_diff_inner(
    repo: &Repository,
    _repo_path: &str,
    opts: DiffOptions,
    diff_opts: &mut git2::DiffOptions,
) -> Result<DiffResult> {
    if opts.untracked {
        let (files, text) = collect_untracked_files(repo)?;
        return Ok(DiffResult {
            label: "Untracked files".into(),
            files,
            text,
            has_conflicts: false,
            is_branch_comparison: false,
            stale_check: None,
        });
    }

    if opts.cached {
        let head = repo.head()?;
        let head_commit = head.peel_to_commit()?;
        let head_tree = head_commit.tree()?;
        let diff = repo.diff_tree_to_index(Some(&head_tree), None, Some(diff_opts))?;
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
            has_conflicts: false,
            is_branch_comparison: false,
            stale_check: None,
        });
    }

    if opts.local {
        let diff = repo.diff_index_to_workdir(None, Some(diff_opts))?;
        let (files, text) = collect_diff_data(&diff);
        return Ok(DiffResult {
            label: "Local changes (unstaged)".into(),
            files,
            text,
            has_conflicts: false,
            is_branch_comparison: false,
            stale_check: None,
        });
    }

    if opts.all {
        let (untracked_files, mut text) = collect_untracked_files(repo)?;

        // Collect staged (cached) changes
        let head = repo.head()?;
        let head_commit = head.peel_to_commit()?;
        let head_tree = head_commit.tree()?;
        let staged_diff = repo.diff_tree_to_index(Some(&head_tree), None, Some(diff_opts))?;
        let (staged_files, staged_text) = collect_diff_data(&staged_diff);

        // Collect unstaged changes
        let local_diff = repo.diff_index_to_workdir(None, Some(diff_opts))?;
        let (local_files, local_text) = collect_diff_data(&local_diff);

        // Merge staged and unstaged: combine by path, summing line counts
        let mut merged: HashMap<String, FileEntry> = HashMap::new();
        for f in staged_files.into_iter().chain(local_files.into_iter()) {
            let entry = merged.entry(f.path.clone()).or_insert(FileEntry {
                path: f.path.clone(),
                old_path: f.old_path.clone(),
                status: f.status,
                additions: 0,
                deletions: 0,
            });
            entry.additions += f.additions;
            entry.deletions += f.deletions;
            if matches!(f.status, Delta::Modified | Delta::Deleted | Delta::Renamed) {
                entry.status = f.status;
            }
        }
        // Append untracked paths not already covered
        for f in &untracked_files {
            merged.entry(f.path.clone()).or_insert(FileEntry {
                path: f.path.clone(),
                old_path: None,
                status: Delta::Untracked,
                additions: f.additions,
                deletions: 0,
            });
        }

        text.push_str(&staged_text);
        text.push_str(&local_text);

        let mut all_files: Vec<FileEntry> = merged.into_values().collect();
        all_files.sort_by(|a, b| a.path.cmp(&b.path));

        return Ok(DiffResult {
            label: "All local changes".into(),
            files: all_files,
            text,
            has_conflicts: false,
            is_branch_comparison: false,
            stale_check: None,
        });
    }

    if opts.self_branch {
        let current_branch = {
            let head = repo.head()?;
            head.shorthand()
                .ok_or_else(|| anyhow!("Detached HEAD — use -b <branch> to specify a target"))?
                .to_string()
        };

        let refname = format!("refs/remotes/origin/{}", current_branch);
        let pre_oid = repo.find_reference(&refname).ok().and_then(|r| r.target());

        let (tx, rx) = mpsc::channel();
        let repo_path_owned = repo.path().to_path_buf();
        let branch_for_fetch = current_branch.clone();
        std::thread::spawn(move || {
            let result = (|| -> Option<bool> {
                let repo = Repository::open(&repo_path_owned).ok()?;
                let mut fetch_opts = make_fetch_options(&repo).ok()?;
                let mut remote = repo.find_remote("origin").ok()?;
                remote.fetch(&[&branch_for_fetch], Some(&mut fetch_opts), None).ok()?;
                let post_oid = repo.find_reference(&format!("refs/remotes/origin/{}", branch_for_fetch))
                    .ok()?.target();
                Some(pre_oid != post_oid)
            })();
            let _ = tx.send(result.unwrap_or(false));
        });

        let base_ref = repo.find_reference(&refname)?;
        let base_commit = base_ref.peel_to_commit()?;
        let base_tree = base_commit.tree()?;

        let head = repo.head()?;
        let local_commit = head.peel_to_commit()?;
        let local_tree = local_commit.tree()?;

        let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&local_tree), Some(diff_opts))?;
        let (files, text) = collect_diff_data(&diff);

        let has_conflicts = repo
            .merge_commits(&base_commit, &local_commit, None)
            .map(|idx| idx.has_conflicts())
            .unwrap_or(false);

        return Ok(DiffResult {
            label: format!("origin/{} ... {}", current_branch, current_branch),
            files,
            text,
            has_conflicts,
            is_branch_comparison: true,
            stale_check: Some(rx),
        });
    }

    // Remote diff (default): compare current branch against the default branch
    let current_branch = {
        let head = repo.head()?;
        head.shorthand()
            .ok_or_else(|| anyhow!("Detached HEAD — use -b <branch> to specify a target"))?
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
                    for candidate in &["main", "master", "develop", "trunk", "dev"] {
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

    let refname = format!("refs/remotes/origin/{}", base_branch);
    let pre_oid = repo.find_reference(&refname).ok().and_then(|r| r.target());

    let (tx, rx) = mpsc::channel();
    let repo_path_owned = repo.path().to_path_buf();
    let branch_for_fetch = base_branch.clone();
    std::thread::spawn(move || {
        let result = (|| -> Option<bool> {
            let repo = Repository::open(&repo_path_owned).ok()?;
            let mut fetch_opts = make_fetch_options(&repo).ok()?;
            let mut remote = repo.find_remote("origin").ok()?;
            remote.fetch(&[&branch_for_fetch], Some(&mut fetch_opts), None).ok()?;
            let post_oid = repo.find_reference(&format!("refs/remotes/origin/{}", branch_for_fetch))
                .ok()?.target();
            Some(pre_oid != post_oid)
        })();
        let _ = tx.send(result.unwrap_or(false));
    });

    let base_ref = repo.find_reference(&refname)?;
    let base_commit = base_ref.peel_to_commit()?;
    let base_tree = base_commit.tree()?;

    let head = repo.head()?;
    let local_commit = head.peel_to_commit()?;
    let local_tree = local_commit.tree()?;

    let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&local_tree), Some(diff_opts))?;
    let (files, text) = collect_diff_data(&diff);

    let has_conflicts = repo
        .merge_commits(&base_commit, &local_commit, None)
        .map(|idx| idx.has_conflicts())
        .unwrap_or(false);

    Ok(DiffResult {
        label: format!("{} ... {}", base_branch, current_branch),
        files,
        text,
        has_conflicts,
        is_branch_comparison: true,
        stale_check: Some(rx),
    })
}
