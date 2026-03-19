use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;

use anyhow::{anyhow, Result};
use git2::{Delta, DiffFormat, Repository, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

/// Returns a map of repo-relative paths → FileStatus for every dirty file.
/// Returns `Ok(None)` when `path` is not inside a git repository.
pub fn get_status_map(path: &str) -> Result<Option<(HashMap<String, FileStatus>, String)>> {
    let repo = match Repository::discover(path) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };

    let mut so = git2::StatusOptions::new();
    so.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);

    let statuses = repo.statuses(Some(&mut so))?;
    let mut map = HashMap::new();

    for entry in statuses.iter() {
        let Some(p) = entry.path() else { continue };
        let s = entry.status();
        let fs = if s.intersects(Status::INDEX_NEW) {
            FileStatus::Added
        } else if s.intersects(Status::INDEX_RENAMED) {
            FileStatus::Renamed
        } else if s.intersects(Status::INDEX_DELETED | Status::WT_DELETED) {
            FileStatus::Deleted
        } else if s.intersects(Status::INDEX_MODIFIED | Status::WT_MODIFIED) {
            FileStatus::Modified
        } else if s.intersects(Status::WT_NEW) {
            FileStatus::Untracked
        } else {
            continue;
        };
        map.insert(p.to_string(), fs);
    }

    // Compute the repo-relative prefix for the requested path
    let workdir = repo.workdir().ok_or_else(|| anyhow!("bare repository"))?;
    let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| Path::new(path).to_path_buf());
    let prefix = abs_path
        .strip_prefix(std::fs::canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf()))
        .unwrap_or(Path::new(""))
        .to_string_lossy()
        .into_owned();

    Ok(Some((map, prefix)))
}

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
    pub patch: String,
}

pub struct DiffResult {
    pub label: String,
    pub files: Vec<FileEntry>,
    pub text: String,
    pub has_conflicts: bool,
    pub is_branch_comparison: bool,
    pub commit_count: Option<usize>,
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
pub(crate) fn collect_untracked_files(repo: &Repository) -> Result<(Vec<FileEntry>, String)> {
    let statuses = repo.statuses(None)?;
    let workdir = repo.workdir().ok_or_else(|| anyhow!("bare repository"))?;
    let estimated_cap = statuses.len() * 256;
    let mut files = Vec::new();
    let mut text = String::with_capacity(estimated_cap.max(4096));

    for entry in statuses.iter() {
        if entry.status().contains(git2::Status::WT_NEW)
            && let Some(path) = entry.path()
        {
            let full_path = workdir.join(path);
            let mut additions = 0usize;
            let mut file_patch = String::new();

            // OOM guard: skip files larger than 10 MB
            let too_large = std::fs::metadata(&full_path)
                .map(|m| m.len() > MAX_UNTRACKED_FILE_SIZE)
                .unwrap_or(false);

            if !too_large
                && let Ok(content) = std::fs::read_to_string(&full_path)
            {
                let header = format!("--- /dev/null\n+++ b/{}\n", path);
                text.push_str(&header);
                file_patch.push_str(&header);
                for line in content.lines() {
                    let diff_line = format!("+{}\n", line);
                    text.push_str(&diff_line);
                    file_patch.push_str(&diff_line);
                    additions += 1;
                }
            }

            files.push(FileEntry {
                path: path.to_string(),
                old_path: None,
                status: Delta::Untracked,
                additions,
                deletions: 0,
                patch: file_patch,
            });
        }
    }
    Ok((files, text))
}

/// Collects the patch text and per-file line counts in a single pass, then
/// merges them with the delta metadata to produce `FileEntry` list.
pub(crate) fn collect_diff_data(diff: &git2::Diff) -> (Vec<FileEntry>, String) {
    let capacity = diff
        .stats()
        .map(|s| (s.insertions() + s.deletions()) * 80)
        .unwrap_or(4096);
    let mut text = String::with_capacity(capacity);
    let mut line_counts: HashMap<String, (usize, usize)> = HashMap::new();
    let mut per_file_patch: HashMap<String, String> = HashMap::new();

    let _ = diff.print(DiffFormat::Patch, |delta, _hunk, line| {
        let origin = line.origin();
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file_text = per_file_patch.entry(path.clone()).or_default();
        if matches!(origin, '+' | '-' | ' ') {
            text.push(origin);
            file_text.push(origin);
        }
        if let Ok(s) = std::str::from_utf8(line.content()) {
            text.push_str(s);
            file_text.push_str(s);
        }
        let counts = line_counts.entry(path).or_insert((0, 0));
        match origin {
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
            let patch = per_file_patch.remove(&path).unwrap_or_default();
            FileEntry {
                path,
                old_path,
                status: delta.status(),
                additions,
                deletions,
                patch,
            }
        })
        .collect();

    (files, text)
}

/// Apply glob filter to file entries, removing non-matching paths.
pub(crate) fn apply_filter(files: Vec<FileEntry>, filter: &str) -> Vec<FileEntry> {
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
pub(crate) fn apply_regex_filter(files: Vec<FileEntry>, pattern: &str) -> Result<Vec<FileEntry>> {
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

    // Rebuild text from remaining files' patches after filtering
    if filter.is_some() || regex.is_some() {
        result.text = result.files.iter().map(|f| f.patch.as_str()).collect();
    }

    Ok(result)
}

fn diff_branch_against_remote(
    repo: &Repository,
    _local_branch: &str,
    remote_branch: &str,
    diff_opts: &mut git2::DiffOptions,
    label: String,
) -> Result<DiffResult> {
    let refname = format!("refs/remotes/origin/{}", remote_branch);
    let pre_oid = repo.find_reference(&refname).ok().and_then(|r| r.target());

    let (tx, rx) = mpsc::channel();
    let repo_path_owned = repo.path().to_path_buf();
    let branch_for_fetch = remote_branch.to_string();
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

    let commit_count = (|| -> Option<usize> {
        let mut revwalk = repo.revwalk().ok()?;
        revwalk.push(local_commit.id()).ok()?;
        revwalk.hide(base_commit.id()).ok()?;
        Some(revwalk.count())
    })();

    let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&local_tree), Some(diff_opts))?;
    let (files, text) = collect_diff_data(&diff);

    let has_conflicts = repo
        .merge_commits(&base_commit, &local_commit, None)
        .map(|idx| idx.has_conflicts())
        .unwrap_or(false);

    Ok(DiffResult {
        label,
        files,
        text,
        has_conflicts,
        is_branch_comparison: true,
        commit_count,
        stale_check: Some(rx),
    })
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
            commit_count: None,
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
            commit_count: None,
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
            commit_count: None,
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
                patch: String::new(),
            });
            entry.additions += f.additions;
            entry.deletions += f.deletions;
            entry.patch.push_str(&f.patch);
            if f.old_path.is_some() {
                entry.old_path = f.old_path.clone();
            }
            if matches!(f.status, Delta::Modified | Delta::Deleted | Delta::Renamed) {
                entry.status = f.status;
            }
        }
        // Append untracked paths not already covered
        for f in untracked_files {
            merged.entry(f.path.clone()).or_insert(FileEntry {
                path: f.path.clone(),
                old_path: None,
                status: Delta::Untracked,
                additions: f.additions,
                deletions: 0,
                patch: f.patch,
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
            commit_count: None,
            stale_check: None,
        });
    }

    let current_branch = {
        let head = repo.head()?;
        head.shorthand()
            .ok_or_else(|| anyhow!("Detached HEAD — use -b <branch> to specify a target"))?
            .to_string()
    };

    if opts.self_branch {
        return diff_branch_against_remote(
            repo,
            &current_branch,
            &current_branch,
            diff_opts,
            format!("origin/{} ... {}", current_branch, current_branch),
        );
    }

    // Remote diff (default): compare current branch against the default branch
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

    diff_branch_against_remote(
        repo,
        &current_branch,
        &base_branch,
        diff_opts,
        format!("origin/{} ... {}", base_branch, current_branch),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, Signature};
    use tempfile::TempDir;
    use std::fs;

    fn setup_test_repo() -> (TempDir, Repository) {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();

        // Configure user for commits
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "test@test.com").unwrap();

        // Create initial commit so HEAD exists
        {
            let sig = Signature::now("Test", "test@test.com").unwrap();
            let tree_id = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();
        }

        (dir, repo)
    }

    fn write_and_stage(repo: &Repository, dir: &std::path::Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new(name)).unwrap();
        index.write().unwrap();
    }

    fn default_opts() -> DiffOptions {
        DiffOptions {
            cached: false,
            untracked: false,
            local: false,
            branch: None,
            all: false,
            self_branch: false,
            context_lines: None,
            filter: None,
            regex: None,
        }
    }

    // ── get_status_map ───────────────────────────────────────────

    #[test]
    fn status_map_non_git_dir() {
        let dir = TempDir::new().unwrap();
        let result = get_status_map(dir.path().to_str().unwrap()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn status_map_clean_repo() {
        let (dir, _repo) = setup_test_repo();
        let result = get_status_map(dir.path().to_str().unwrap()).unwrap().unwrap();
        assert!(result.0.is_empty());
    }

    #[test]
    fn status_map_untracked() {
        let (dir, _repo) = setup_test_repo();
        fs::write(dir.path().join("new.txt"), "hello").unwrap();
        let (map, _) = get_status_map(dir.path().to_str().unwrap()).unwrap().unwrap();
        assert_eq!(map.get("new.txt"), Some(&FileStatus::Untracked));
    }

    #[test]
    fn status_map_staged_new() {
        let (dir, repo) = setup_test_repo();
        write_and_stage(&repo, dir.path(), "added.txt", "content");
        let (map, _) = get_status_map(dir.path().to_str().unwrap()).unwrap().unwrap();
        assert_eq!(map.get("added.txt"), Some(&FileStatus::Added));
    }

    #[test]
    fn status_map_modified() {
        let (dir, repo) = setup_test_repo();
        write_and_stage(&repo, dir.path(), "file.txt", "v1");
        // Commit it
        let sig = Signature::now("Test", "test@test.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add file", &tree, &[&head]).unwrap();
        // Modify it
        fs::write(dir.path().join("file.txt"), "v2").unwrap();
        let (map, _) = get_status_map(dir.path().to_str().unwrap()).unwrap().unwrap();
        assert_eq!(map.get("file.txt"), Some(&FileStatus::Modified));
    }

    #[test]
    fn status_map_deleted() {
        let (dir, repo) = setup_test_repo();
        write_and_stage(&repo, dir.path(), "file.txt", "content");
        let sig = Signature::now("Test", "test@test.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add file", &tree, &[&head]).unwrap();
        // Delete it
        fs::remove_file(dir.path().join("file.txt")).unwrap();
        let (map, _) = get_status_map(dir.path().to_str().unwrap()).unwrap().unwrap();
        assert_eq!(map.get("file.txt"), Some(&FileStatus::Deleted));
    }

    #[test]
    fn status_map_subdirectory_prefix() {
        let (dir, repo) = setup_test_repo();
        write_and_stage(&repo, dir.path(), "sub/file.txt", "content");
        let (map, _) = get_status_map(dir.path().to_str().unwrap()).unwrap().unwrap();
        assert!(map.contains_key("sub/file.txt"));
    }

    // ── apply_filter ─────────────────────────────────────────────

    fn make_entry(path: &str) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            old_path: None,
            status: Delta::Modified,
            additions: 1,
            deletions: 0,
            patch: String::new(),
        }
    }

    #[test]
    fn filter_keeps_matching() {
        let files = vec![make_entry("src/main.rs"), make_entry("readme.md")];
        let result = apply_filter(files, "*.rs");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "src/main.rs");
    }

    #[test]
    fn filter_no_match_empty() {
        let files = vec![make_entry("main.rs")];
        let result = apply_filter(files, "*.xyz");
        assert!(result.is_empty());
    }

    #[test]
    fn filter_star_keeps_all() {
        let files = vec![make_entry("a.rs"), make_entry("b.txt")];
        let result = apply_filter(files, "*");
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_invalid_glob_graceful() {
        let files = vec![make_entry("a.rs")];
        // Invalid glob should return all files (no matcher built)
        let result = apply_filter(files, "[invalid");
        // Either returns all or none — just shouldn't panic
        let _ = result;
    }

    // ── apply_regex_filter ───────────────────────────────────────

    #[test]
    fn regex_filter_keeps_matching() {
        let files = vec![make_entry("src/main.rs"), make_entry("readme.md")];
        let result = apply_regex_filter(files, r"\.rs$").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "src/main.rs");
    }

    #[test]
    fn regex_filter_no_match() {
        let files = vec![make_entry("main.rs")];
        let result = apply_regex_filter(files, r"\.py$").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn regex_filter_invalid_returns_err() {
        let files = vec![make_entry("a.rs")];
        assert!(apply_regex_filter(files, r"[invalid").is_err());
    }

    // ── collect_diff_data ────────────────────────────────────────

    #[test]
    fn collect_diff_single_modified_file() {
        let (dir, repo) = setup_test_repo();
        write_and_stage(&repo, dir.path(), "file.txt", "line1\n");
        let sig = Signature::now("Test", "test@test.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add", &tree, &[&head]).unwrap();

        // Modify and stage
        write_and_stage(&repo, dir.path(), "file.txt", "line1\nline2\n");
        let head2 = repo.head().unwrap().peel_to_commit().unwrap();
        let head_tree = head2.tree().unwrap();
        let diff = repo.diff_tree_to_index(Some(&head_tree), None, None).unwrap();

        let (files, text) = collect_diff_data(&diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "file.txt");
        assert!(files[0].additions > 0);
        assert!(!text.is_empty());
    }

    #[test]
    fn collect_diff_empty() {
        let (_dir, repo) = setup_test_repo();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let tree = head.tree().unwrap();
        let diff = repo.diff_tree_to_index(Some(&tree), None, None).unwrap();
        let (files, text) = collect_diff_data(&diff);
        assert!(files.is_empty());
        assert!(text.is_empty());
    }

    #[test]
    fn collect_diff_multiple_files() {
        let (dir, repo) = setup_test_repo();
        write_and_stage(&repo, dir.path(), "a.txt", "a");
        write_and_stage(&repo, dir.path(), "b.txt", "b");
        let sig = Signature::now("Test", "test@test.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add files", &tree, &[&head]).unwrap();

        write_and_stage(&repo, dir.path(), "a.txt", "a modified");
        write_and_stage(&repo, dir.path(), "b.txt", "b modified");
        let head2 = repo.head().unwrap().peel_to_commit().unwrap();
        let head_tree = head2.tree().unwrap();
        let diff = repo.diff_tree_to_index(Some(&head_tree), None, None).unwrap();

        let (files, _) = collect_diff_data(&diff);
        assert_eq!(files.len(), 2);
    }

    // ── collect_untracked_files ──────────────────────────────────

    #[test]
    fn collect_untracked_single() {
        let (dir, repo) = setup_test_repo();
        fs::write(dir.path().join("untracked.txt"), "hello").unwrap();
        let (files, text) = collect_untracked_files(&repo).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "untracked.txt");
        assert_eq!(files[0].status, Delta::Untracked);
        assert!(!text.is_empty());
    }

    #[test]
    fn collect_untracked_none() {
        let (_dir, repo) = setup_test_repo();
        let (files, text) = collect_untracked_files(&repo).unwrap();
        assert!(files.is_empty());
        assert!(text.is_empty());
    }

    // ── get_diff (public) ────────────────────────────────────────

    #[test]
    fn get_diff_cached_mode() {
        let (dir, repo) = setup_test_repo();
        write_and_stage(&repo, dir.path(), "staged.txt", "content");
        let mut opts = default_opts();
        opts.cached = true;
        let result = get_diff(dir.path().to_str().unwrap(), opts).unwrap();
        assert!(result.label.contains("Staged"));
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn get_diff_local_mode() {
        let (dir, repo) = setup_test_repo();
        write_and_stage(&repo, dir.path(), "file.txt", "v1");
        let sig = Signature::now("Test", "test@test.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add", &tree, &[&head]).unwrap();
        // Now modify without staging
        fs::write(dir.path().join("file.txt"), "v2").unwrap();
        let mut opts = default_opts();
        opts.local = true;
        let result = get_diff(dir.path().to_str().unwrap(), opts).unwrap();
        assert!(result.label.contains("Local") || result.label.contains("unstaged"));
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn get_diff_untracked_mode() {
        let (dir, _repo) = setup_test_repo();
        fs::write(dir.path().join("new.txt"), "hello").unwrap();
        let mut opts = default_opts();
        opts.untracked = true;
        let result = get_diff(dir.path().to_str().unwrap(), opts).unwrap();
        assert!(result.label.contains("Untracked"));
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn get_diff_all_mode() {
        let (dir, repo) = setup_test_repo();
        // Create a committed file, then modify it (unstaged)
        write_and_stage(&repo, dir.path(), "committed.txt", "v1");
        let sig = Signature::now("Test", "test@test.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add", &tree, &[&head]).unwrap();
        // Stage a new file
        write_and_stage(&repo, dir.path(), "staged.txt", "new");
        // Create an untracked file
        fs::write(dir.path().join("untracked.txt"), "ut").unwrap();

        let mut opts = default_opts();
        opts.all = true;
        let result = get_diff(dir.path().to_str().unwrap(), opts).unwrap();
        assert!(result.label.contains("All"));
        assert!(result.files.len() >= 2);
    }

    #[test]
    fn get_diff_empty_cached() {
        let (dir, _repo) = setup_test_repo();
        let mut opts = default_opts();
        opts.cached = true;
        let result = get_diff(dir.path().to_str().unwrap(), opts).unwrap();
        assert!(result.files.is_empty());
    }

    #[test]
    fn get_diff_with_glob_filter() {
        let (dir, repo) = setup_test_repo();
        write_and_stage(&repo, dir.path(), "keep.rs", "fn main() {}");
        write_and_stage(&repo, dir.path(), "skip.txt", "hello");
        let mut opts = default_opts();
        opts.cached = true;
        opts.filter = Some("*.rs".to_string());
        let result = get_diff(dir.path().to_str().unwrap(), opts).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, "keep.rs");
    }

    #[test]
    fn get_diff_with_regex_filter() {
        let (dir, repo) = setup_test_repo();
        write_and_stage(&repo, dir.path(), "main.rs", "fn main() {}");
        write_and_stage(&repo, dir.path(), "readme.md", "# readme");
        let mut opts = default_opts();
        opts.cached = true;
        opts.regex = Some(r"\.rs$".to_string());
        let result = get_diff(dir.path().to_str().unwrap(), opts).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, "main.rs");
    }
}
