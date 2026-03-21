use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;

use anyhow::{anyhow, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeltaStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    Untracked,
}

pub struct DiffOptions {
    pub untracked: bool,
    pub tracked: bool,
    pub staged: bool,
    pub local: bool,
    pub all: bool,
    pub branch: Option<String>,
    pub context_lines: Option<u32>,
    pub max_untracked_size: u64,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            untracked: false,
            tracked: false,
            staged: false,
            local: false,
            branch: None,
            all: false,
            context_lines: None,
            max_untracked_size: 10 * 1024 * 1024,
        }
    }
}

pub struct FileEntry {
    pub path: String,
    pub old_path: Option<String>,
    pub status: DeltaStatus,
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


// ── Git CLI helpers ─────────────────────────────────────────────────

fn run_git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| anyhow!("failed to run git: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("git {} failed: {}", args.first().unwrap_or(&""), stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn try_run_git(dir: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Discover a git repository from `path`. Returns `Ok(None)` if not inside a repo.
fn discover_repo(path: &str) -> Result<Option<(gix::Repository, PathBuf)>> {
    let p = Path::new(path);
    let dir = if p.is_file() {
        p.parent().unwrap_or(p)
    } else {
        p
    };
    match gix::discover(dir) {
        Ok(repo) => match repo.workdir() {
            Some(wd) => {
                let wd = wd.to_path_buf();
                Ok(Some((repo, wd)))
            }
            None => Ok(None), // bare repo
        },
        Err(_) => Ok(None),
    }
}

fn open_repo(workdir: &Path) -> Result<gix::Repository> {
    gix::discover(workdir)
        .map_err(|e| anyhow!("failed to open git repo at {}: {}", workdir.display(), e))
}

// ── Status map (for tree display) ───────────────────────────────────

/// Returns a map of repo-relative paths → FileStatus for every dirty file.
/// Returns `Ok(None)` when `path` is not inside a git repository.
pub fn get_status_map(path: &str) -> Result<Option<(HashMap<String, FileStatus>, String)>> {
    let Some((repo, workdir)) = discover_repo(path)? else {
        return Ok(None);
    };
    let iter = repo
        .status(gix::progress::Discard)
        .map_err(|e| anyhow!("status setup failed: {}", e))?
        .into_iter(Vec::new())
        .map_err(|e| anyhow!("status iteration failed: {}", e))?;

    let mut map = HashMap::new();

    for item in iter {
        let item = item.map_err(|e| anyhow!("status item error: {}", e))?;
        match item {
            gix::status::Item::TreeIndex(change) => {
                let (path_str, fs) = match &change {
                    gix::diff::index::Change::Addition { location, .. } => {
                        (location.to_string(), FileStatus::Added)
                    }
                    gix::diff::index::Change::Deletion { location, .. } => {
                        (location.to_string(), FileStatus::Deleted)
                    }
                    gix::diff::index::Change::Modification { location, .. } => {
                        (location.to_string(), FileStatus::Modified)
                    }
                    gix::diff::index::Change::Rewrite {
                        location,
                        ..
                    } => (location.to_string(), FileStatus::Renamed),
                };
                // Staged takes priority: TreeIndex items are emitted before IndexWorktree
                map.insert(path_str, fs);
            }
            gix::status::Item::IndexWorktree(iw) => {
                use gix::status::index_worktree::Item as IW;
                match iw {
                    IW::Modification {
                        rela_path, status, ..
                    } => {
                        let path_str = rela_path.to_string();
                        // Don't overwrite staged status (staged has higher priority)
                        if let std::collections::hash_map::Entry::Vacant(e) = map.entry(path_str) {
                            use gix_status::index_as_worktree::{Change, EntryStatus};
                            let fs = match status {
                                EntryStatus::Change(change) => match change {
                                    Change::Removed => FileStatus::Deleted,
                                    Change::Modification { .. }
                                    | Change::Type { .. } => FileStatus::Modified,
                                    Change::SubmoduleModification(_) => FileStatus::Modified,
                                },
                                EntryStatus::IntentToAdd => FileStatus::Added,
                                EntryStatus::Conflict { .. } => FileStatus::Modified,
                                EntryStatus::NeedsUpdate(_) => continue,
                            };
                            e.insert(fs);
                        }
                    }
                    IW::DirectoryContents { entry, .. } => {
                        use gix::dir::entry::Status;
                        if matches!(entry.status, Status::Untracked) {
                            let path_str = entry.rela_path.to_string();
                            map.entry(path_str).or_insert(FileStatus::Untracked);
                        }
                    }
                    IW::Rewrite {
                        dirwalk_entry,
                        ..
                    } => {
                        let path_str = dirwalk_entry.rela_path.to_string();
                        map.entry(path_str).or_insert(FileStatus::Renamed);
                    }
                }
            }
        }
    }

    // Compute the repo-relative prefix for the requested path
    let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| Path::new(path).to_path_buf());
    let canon_workdir = std::fs::canonicalize(&workdir).unwrap_or(workdir);
    let prefix = abs_path
        .strip_prefix(&canon_workdir)
        .unwrap_or(Path::new(""))
        .to_string_lossy()
        .into_owned();

    Ok(Some((map, prefix)))
}

// ── Diff parsing helpers ────────────────────────────────────────────

fn parse_name_status(output: &str) -> Vec<(DeltaStatus, String, Option<String>)> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let status_str = parts.next()?;
            let first_path = parts.next()?;

            if status_str.starts_with('R') {
                let new_path = parts.next()?;
                Some((DeltaStatus::Renamed, new_path.to_string(), Some(first_path.to_string())))
            } else if status_str.starts_with('C') {
                let new_path = parts.next()?;
                Some((DeltaStatus::Copied, new_path.to_string(), Some(first_path.to_string())))
            } else {
                let status = match status_str {
                    "A" => DeltaStatus::Added,
                    "D" => DeltaStatus::Deleted,
                    "M" => DeltaStatus::Modified,
                    _ => return None,
                };
                Some((status, first_path.to_string(), None))
            }
        })
        .collect()
}

/// Split a unified diff into per-file chunks, returning (path, patch_text, additions, deletions).
fn split_diff_per_file(full_diff: &str) -> HashMap<String, (String, usize, usize)> {
    let mut result: HashMap<String, (String, usize, usize)> = HashMap::new();

    if full_diff.is_empty() {
        return result;
    }

    // Find all "diff --git" boundaries
    let mut boundaries: Vec<usize> = Vec::new();
    for (i, _) in full_diff.match_indices("diff --git ") {
        // Must be at start of string or preceded by newline
        if i == 0 || full_diff.as_bytes().get(i.wrapping_sub(1)) == Some(&b'\n') {
            boundaries.push(i);
        }
    }

    if boundaries.is_empty() {
        return result;
    }

    for (idx, &start) in boundaries.iter().enumerate() {
        let end = boundaries.get(idx + 1).copied().unwrap_or(full_diff.len());
        let chunk = &full_diff[start..end];

        // Extract path from +++ line, falling back to --- line
        let path = chunk
            .lines()
            .find_map(|l| l.strip_prefix("+++ b/").map(String::from))
            .or_else(|| {
                chunk
                    .lines()
                    .find_map(|l| l.strip_prefix("--- a/").map(String::from))
            });

        let Some(path) = path else { continue };

        let mut additions = 0usize;
        let mut deletions = 0usize;
        for line in chunk.lines() {
            if line.starts_with('+') && !line.starts_with("+++") {
                additions += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                deletions += 1;
            }
        }

        result.insert(path, (chunk.to_string(), additions, deletions));
    }

    result
}

/// Run `git diff` with the given args and return structured file entries + full text.
fn run_diff(repo_dir: &Path, args: &[&str]) -> Result<(Vec<FileEntry>, String)> {
    // Get name-status
    let mut ns_args = vec!["diff", "--name-status"];
    ns_args.extend_from_slice(args);
    let name_status_out = run_git(repo_dir, &ns_args)?;

    // Get full patch (force a/b prefixes to avoid mnemonic prefix config)
    let mut patch_args = vec!["diff", "--no-color", "--src-prefix=a/", "--dst-prefix=b/"];
    patch_args.extend_from_slice(args);
    let full_patch = run_git(repo_dir, &patch_args)?;

    let statuses = parse_name_status(&name_status_out);
    let mut patch_map = split_diff_per_file(&full_patch);

    let files: Vec<FileEntry> = statuses
        .into_iter()
        .map(|(status, path, old_path)| {
            let (patch, additions, deletions) =
                patch_map.remove(&path).unwrap_or_default();
            FileEntry {
                path,
                old_path,
                status,
                additions,
                deletions,
                patch,
            }
        })
        .collect();

    Ok((files, full_patch))
}

// ── Untracked files ─────────────────────────────────────────────────

/// Collect untracked files from the working directory.
pub(crate) fn collect_untracked_files(repo_dir: &Path, max_untracked_size: u64) -> Result<(Vec<FileEntry>, String)> {
    let repo = open_repo(repo_dir)?;
    let iter = repo
        .status(gix::progress::Discard)
        .map_err(|e| anyhow!("status setup failed: {}", e))?
        .into_iter(Vec::new())
        .map_err(|e| anyhow!("status iteration failed: {}", e))?;

    let mut untracked_paths = Vec::new();
    for item in iter {
        let item = item.map_err(|e| anyhow!("status item error: {}", e))?;
        if let gix::status::Item::IndexWorktree(
            gix::status::index_worktree::Item::DirectoryContents { entry, .. },
        ) = item
            && matches!(entry.status, gix::dir::entry::Status::Untracked) {
                untracked_paths.push(entry.rela_path.to_string());
            }
    }

    let mut files = Vec::new();
    let mut text = String::with_capacity(4096);

    for rel_path in &untracked_paths {
        let full_path = repo_dir.join(rel_path);
        let mut additions = 0usize;
        let mut file_patch = String::new();

        // OOM guard: skip files larger than the configured limit
        let too_large = std::fs::metadata(&full_path)
            .map(|m| m.len() > max_untracked_size)
            .unwrap_or(false);

        if !too_large
            && let Ok(content) = std::fs::read_to_string(&full_path)
        {
            let header = format!("--- /dev/null\n+++ b/{}\n", rel_path);
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
            path: rel_path.to_string(),
            old_path: None,
            status: DeltaStatus::Untracked,
            additions,
            deletions: 0,
            patch: file_patch,
        });
    }
    Ok((files, text))
}

// ── Filters ─────────────────────────────────────────────────────────

/// Apply regex filter to file entries, keeping paths that match.
pub(crate) fn apply_regex_filter(files: Vec<FileEntry>, pattern: &str) -> Result<Vec<FileEntry>> {
    let re = regex::Regex::new(pattern)
        .map_err(|e| anyhow!("invalid regex '{}': {}", pattern, e))?;
    Ok(files.into_iter().filter(|f| re.is_match(&f.path)).collect())
}

// ── Public diff entry point ─────────────────────────────────────────

pub fn get_diff(repo_path: &str, opts: DiffOptions, regex: Option<&str>) -> Result<DiffResult> {
    let (_, repo_dir) = discover_repo(repo_path)?
        .ok_or_else(|| anyhow!("not a git repository: {}", repo_path))?;

    let mut result = get_diff_inner(&repo_dir, opts)?;

    if let Some(pattern) = regex {
        result.files = apply_regex_filter(result.files, pattern)?;
        result.text = result.files.iter().map(|f| f.patch.as_str()).collect();
    }

    Ok(result)
}

// ── Branch comparison with background fetch ─────────────────────────

fn diff_branch_against_remote(
    repo_dir: &Path,
    _local_branch: &str,
    remote_branch: &str,
    context_lines: Option<u32>,
    label: String,
) -> Result<DiffResult> {
    let refname = format!("refs/remotes/origin/{}", remote_branch);

    let repo = open_repo(repo_dir)?;

    // Resolve the remote ref OID before fetch (using gix)
    let pre_oid = repo
        .try_find_reference(&refname)
        .ok()
        .flatten()
        .and_then(|r| r.into_fully_peeled_id().ok())
        .map(|id| id.to_hex().to_string());

    // Background fetch — gix::Repository is !Send, so the post-fetch check stays as CLI
    let (tx, rx) = mpsc::channel();
    let repo_dir_owned = repo_dir.to_path_buf();
    let branch_for_fetch = remote_branch.to_string();
    std::thread::spawn(move || {
        let result = (|| -> Option<bool> {
            let output = Command::new("git")
                .args(["fetch", "origin", &branch_for_fetch])
                .current_dir(&repo_dir_owned)
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let post_oid = try_run_git(
                &repo_dir_owned,
                &["rev-parse", &format!("refs/remotes/origin/{}", branch_for_fetch)],
            );
            Some(pre_oid != post_oid)
        })();
        let _ = tx.send(result.unwrap_or(false));
    });

    // Resolve base (remote) and local OIDs using gix
    let base_oid = repo
        .find_reference(&refname)
        .map_err(|e| anyhow!("failed to resolve {}: {}", refname, e))?
        .into_fully_peeled_id()
        .map_err(|e| anyhow!("failed to peel {}: {}", refname, e))?
        .to_hex()
        .to_string();
    let local_oid = repo
        .head_commit()
        .map_err(|e| anyhow!("failed to resolve HEAD: {}", e))?
        .id()
        .to_hex()
        .to_string();

    // Count commits between base and local
    let commit_count = try_run_git(
        repo_dir,
        &["rev-list", "--count", &format!("{}..{}", base_oid, local_oid)],
    )
    .and_then(|s| s.parse::<usize>().ok());

    // Run diff between the two commits
    let mut diff_args: Vec<&str> = Vec::new();
    let ctx_str;
    if let Some(ctx) = context_lines {
        ctx_str = format!("-U{}", ctx);
        diff_args.push(&ctx_str);
    }
    diff_args.push(&base_oid);
    diff_args.push(&local_oid);
    let (files, text) = run_diff(repo_dir, &diff_args)?;

    // Merge conflict detection via merge-tree
    let has_conflicts = Command::new("git")
        .args(["merge-tree", "--write-tree", &base_oid, &local_oid])
        .current_dir(repo_dir)
        .output()
        .map(|o| !o.status.success())
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

// ── Inner diff logic ────────────────────────────────────────────────

fn get_diff_inner(repo_dir: &Path, opts: DiffOptions) -> Result<DiffResult> {
    if opts.untracked {
        let (files, text) = collect_untracked_files(repo_dir, opts.max_untracked_size)?;
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

    let ctx_arg: String;
    let base_diff_args: Vec<&str> = if let Some(ctx) = opts.context_lines {
        ctx_arg = format!("-U{}", ctx);
        vec![&ctx_arg]
    } else {
        vec![]
    };

    if opts.staged {
        let mut args = vec!["--cached"];
        args.extend_from_slice(&base_diff_args);
        let (files, text) = run_diff(repo_dir, &args)?;
        let branch = open_repo(repo_dir)
            .ok()
            .and_then(|r| {
                r.head_ref()
                    .ok()
                    .flatten()
                    .map(|r| r.name().shorten().to_string())
            })
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

    if opts.tracked {
        let (files, text) = run_diff(repo_dir, &base_diff_args)?;
        return Ok(DiffResult {
            label: "Tracked changes (unstaged)".into(),
            files,
            text,
            has_conflicts: false,
            is_branch_comparison: false,
            commit_count: None,
            stale_check: None,
        });
    }

    if opts.local {
        // Gather all local changes (untracked + staged + unstaged)
        // then compare against self branch remote
        let repo = open_repo(repo_dir)?;
        let current_branch = repo
            .head_ref()
            .map_err(|e| anyhow!("failed to read HEAD: {}", e))?
            .ok_or_else(|| anyhow!("Detached HEAD — use -b <branch> to specify a target"))?
            .name()
            .shorten()
            .to_string();

        return diff_branch_against_remote(
            repo_dir,
            &current_branch,
            &current_branch,
            opts.context_lines,
            format!("origin/{} ... {}", current_branch, current_branch),
        );
    }

    if opts.all {
        let (untracked_files, mut text) = collect_untracked_files(repo_dir, opts.max_untracked_size)?;

        // Staged changes
        let mut cached_args = vec!["--cached"];
        cached_args.extend_from_slice(&base_diff_args);
        let (staged_files, staged_text) = run_diff(repo_dir, &cached_args)?;

        // Unstaged changes
        let (local_files, local_text) = run_diff(repo_dir, &base_diff_args)?;

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
            if matches!(
                f.status,
                DeltaStatus::Modified | DeltaStatus::Deleted | DeltaStatus::Renamed
            ) {
                entry.status = f.status;
            }
        }
        // Append untracked paths not already covered
        for f in untracked_files {
            merged.entry(f.path.clone()).or_insert(FileEntry {
                path: f.path.clone(),
                old_path: None,
                status: DeltaStatus::Untracked,
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

    // Resolve current branch
    let repo = open_repo(repo_dir)?;
    let current_branch = repo
        .head_ref()
        .map_err(|e| anyhow!("failed to read HEAD: {}", e))?
        .ok_or_else(|| anyhow!("Detached HEAD — use -b <branch> to specify a target"))?
        .name()
        .shorten()
        .to_string();

    // Remote diff (default / -a): compare current branch against the default branch
    let base_branch = match opts.branch {
        Some(b) => b,
        None => {
            // Try origin/HEAD symbolic ref via gix
            repo.try_find_reference("refs/remotes/origin/HEAD")
                .ok()
                .flatten()
                .and_then(|r| {
                    use gix::refs::TargetRef;
                    match r.target() {
                        TargetRef::Symbolic(name) => {
                            let full = name.as_bstr().to_string();
                            full.strip_prefix("refs/remotes/origin/")
                                .map(|s| s.to_string())
                        }
                        TargetRef::Object(_) => None,
                    }
                })
                .or_else(|| {
                    for candidate in &["main", "master", "develop", "trunk", "dev"] {
                        let refname = format!("refs/remotes/origin/{}", candidate);
                        if repo.try_find_reference(&refname).ok().flatten().is_some() {
                            return Some(candidate.to_string());
                        }
                    }
                    None
                })
                .ok_or_else(|| {
                    anyhow!("Could not determine default branch; use -b to specify one")
                })?
        }
    };

    diff_branch_against_remote(
        repo_dir,
        &current_branch,
        &base_branch,
        opts.context_lines,
        format!("origin/{} ... {}", base_branch, current_branch),
    )
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        run_git(dir.path(), &["init"]).unwrap();
        run_git(dir.path(), &["config", "user.name", "Test"]).unwrap();
        run_git(dir.path(), &["config", "user.email", "test@test.com"]).unwrap();
        // Create initial empty commit so HEAD exists
        run_git(
            dir.path(),
            &["commit", "--allow-empty", "-m", "initial"],
        )
        .unwrap();
        dir
    }

    fn write_and_stage(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        run_git(dir, &["add", name]).unwrap();
    }

    fn commit(dir: &Path, msg: &str) {
        run_git(dir, &["commit", "-m", msg]).unwrap();
    }

    fn default_opts() -> DiffOptions {
        DiffOptions::default()
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
        let dir = setup_test_repo();
        let result = get_status_map(dir.path().to_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(result.0.is_empty());
    }

    #[test]
    fn status_map_untracked() {
        let dir = setup_test_repo();
        fs::write(dir.path().join("new.txt"), "hello").unwrap();
        let (map, _) = get_status_map(dir.path().to_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(map.get("new.txt"), Some(&FileStatus::Untracked));
    }

    #[test]
    fn status_map_staged_new() {
        let dir = setup_test_repo();
        write_and_stage(dir.path(), "added.txt", "content");
        let (map, _) = get_status_map(dir.path().to_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(map.get("added.txt"), Some(&FileStatus::Added));
    }

    #[test]
    fn status_map_modified() {
        let dir = setup_test_repo();
        write_and_stage(dir.path(), "file.txt", "v1");
        commit(dir.path(), "add file");
        // Modify it
        fs::write(dir.path().join("file.txt"), "v2").unwrap();
        let (map, _) = get_status_map(dir.path().to_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(map.get("file.txt"), Some(&FileStatus::Modified));
    }

    #[test]
    fn status_map_deleted() {
        let dir = setup_test_repo();
        write_and_stage(dir.path(), "file.txt", "content");
        commit(dir.path(), "add file");
        // Delete it
        fs::remove_file(dir.path().join("file.txt")).unwrap();
        let (map, _) = get_status_map(dir.path().to_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(map.get("file.txt"), Some(&FileStatus::Deleted));
    }

    #[test]
    fn status_map_subdirectory_prefix() {
        let dir = setup_test_repo();
        write_and_stage(dir.path(), "sub/file.txt", "content");
        let (map, _) = get_status_map(dir.path().to_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(map.contains_key("sub/file.txt"));
    }

    // ── apply_filter ─────────────────────────────────────────────

    fn make_entry(path: &str) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            old_path: None,
            status: DeltaStatus::Modified,
            additions: 1,
            deletions: 0,
            patch: String::new(),
        }
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

    // ── run_diff ─────────────────────────────────────────────────

    #[test]
    fn diff_single_modified_file() {
        let dir = setup_test_repo();
        write_and_stage(dir.path(), "file.txt", "line1\n");
        commit(dir.path(), "add");

        // Modify and stage
        write_and_stage(dir.path(), "file.txt", "line1\nline2\n");
        let (files, text) = run_diff(dir.path(), &["--cached"]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "file.txt");
        assert!(files[0].additions > 0);
        assert!(!text.is_empty());
    }

    #[test]
    fn diff_empty() {
        let dir = setup_test_repo();
        let (files, text) = run_diff(dir.path(), &["--cached"]).unwrap();
        assert!(files.is_empty());
        assert!(text.is_empty());
    }

    #[test]
    fn diff_multiple_files() {
        let dir = setup_test_repo();
        write_and_stage(dir.path(), "a.txt", "a");
        write_and_stage(dir.path(), "b.txt", "b");
        commit(dir.path(), "add files");

        write_and_stage(dir.path(), "a.txt", "a modified");
        write_and_stage(dir.path(), "b.txt", "b modified");
        let (files, _) = run_diff(dir.path(), &["--cached"]).unwrap();
        assert_eq!(files.len(), 2);
    }

    // ── collect_untracked_files ──────────────────────────────────

    #[test]
    fn collect_untracked_single() {
        let dir = setup_test_repo();
        fs::write(dir.path().join("untracked.txt"), "hello").unwrap();
        let (files, text) = collect_untracked_files(dir.path(), 10 * 1024 * 1024).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "untracked.txt");
        assert_eq!(files[0].status, DeltaStatus::Untracked);
        assert!(!text.is_empty());
    }

    #[test]
    fn collect_untracked_none() {
        let dir = setup_test_repo();
        let (files, text) = collect_untracked_files(dir.path(), 10 * 1024 * 1024).unwrap();
        assert!(files.is_empty());
        assert!(text.is_empty());
    }

    // ── get_diff (public) ────────────────────────────────────────

    #[test]
    fn get_diff_cached_mode() {
        let dir = setup_test_repo();
        write_and_stage(dir.path(), "staged.txt", "content");
        let mut opts = default_opts();
        opts.staged = true;
        let result = get_diff(dir.path().to_str().unwrap(), opts, None).unwrap();
        assert!(result.label.contains("Staged"));
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn get_diff_tracked_mode() {
        let dir = setup_test_repo();
        write_and_stage(dir.path(), "file.txt", "v1");
        commit(dir.path(), "add");
        // Now modify without staging
        fs::write(dir.path().join("file.txt"), "v2").unwrap();
        let mut opts = default_opts();
        opts.tracked = true;
        let result = get_diff(dir.path().to_str().unwrap(), opts, None).unwrap();
        assert!(result.label.contains("Tracked") || result.label.contains("unstaged"));
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn get_diff_untracked_mode() {
        let dir = setup_test_repo();
        fs::write(dir.path().join("new.txt"), "hello").unwrap();
        let mut opts = default_opts();
        opts.untracked = true;
        let result = get_diff(dir.path().to_str().unwrap(), opts, None).unwrap();
        assert!(result.label.contains("Untracked"));
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn get_diff_all_mode() {
        let dir = setup_test_repo();
        // Create a committed file, then modify it (unstaged)
        write_and_stage(dir.path(), "committed.txt", "v1");
        commit(dir.path(), "add");
        // Stage a new file
        write_and_stage(dir.path(), "staged.txt", "new");
        // Create an untracked file
        fs::write(dir.path().join("untracked.txt"), "ut").unwrap();

        let mut opts = default_opts();
        opts.all = true;
        let result = get_diff(dir.path().to_str().unwrap(), opts, None).unwrap();
        assert!(result.label.contains("All"));
        assert!(result.files.len() >= 2);
    }

    #[test]
    fn get_diff_empty_cached() {
        let dir = setup_test_repo();
        let mut opts = default_opts();
        opts.staged = true;
        let result = get_diff(dir.path().to_str().unwrap(), opts, None).unwrap();
        assert!(result.files.is_empty());
    }

    #[test]
    fn get_diff_with_regex_filter() {
        let dir = setup_test_repo();
        write_and_stage(dir.path(), "main.rs", "fn main() {}");
        write_and_stage(dir.path(), "readme.md", "# readme");
        let mut opts = default_opts();
        opts.staged = true;
        let result = get_diff(dir.path().to_str().unwrap(), opts, Some(r"\.rs$")).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, "main.rs");
    }
}
