use anyhow::{anyhow, Result};
use git2::{DiffFormat, Repository};

pub struct DiffOptions {
    pub cached: bool,
    pub untracked: bool,
    pub local: bool,
    pub branch: Option<String>,
}

pub fn get_diff(repo_path: &str, opts: DiffOptions) -> Result<String> {
    let repo = Repository::open(repo_path)?;
    let mut output = String::new();

    if opts.untracked {
        let statuses = repo.statuses(None)?;
        for entry in statuses.iter() {
            if entry.status().contains(git2::Status::WT_NEW) {
                if let Some(path) = entry.path() {
                    let full_path = std::path::Path::new(repo_path).join(path);
                    if let Ok(content) = std::fs::read_to_string(&full_path) {
                        output.push_str(&format!("--- /dev/null\n+++ b/{}\n", path));
                        for line in content.lines() {
                            output.push_str(&format!("+{}\n", line));
                        }
                    }
                }
            }
        }
    } else if opts.cached {
        let head = repo.head()?;
        let head_commit = head.peel_to_commit()?;
        let head_tree = head_commit.tree()?;
        let diff = repo.diff_tree_to_index(Some(&head_tree), None, None)?;
        diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
            if let Ok(s) = std::str::from_utf8(line.content()) {
                output.push_str(s);
            }
            true
        })?;
    } else if opts.local {
        let diff = repo.diff_index_to_workdir(None, None)?;
        diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
            if let Ok(s) = std::str::from_utf8(line.content()) {
                output.push_str(s);
            }
            true
        })?;
    } else {
        let branch = match opts.branch {
            Some(b) => b,
            None => {
                let head = repo.head()?;
                head.shorthand()
                    .ok_or_else(|| anyhow!("Could not determine current branch"))?
                    .to_string()
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
            remote.fetch(&[branch.as_str()], Some(&mut fetch_opts), None)?;
        }

        let remote_ref =
            repo.find_reference(&format!("refs/remotes/origin/{}", branch))?;
        let remote_commit = remote_ref.peel_to_commit()?;
        let remote_tree = remote_commit.tree()?;

        let head = repo.head()?;
        let local_commit = head.peel_to_commit()?;
        let local_tree = local_commit.tree()?;

        let diff = repo.diff_tree_to_tree(Some(&remote_tree), Some(&local_tree), None)?;
        diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
            if let Ok(s) = std::str::from_utf8(line.content()) {
                output.push_str(s);
            }
            true
        })?;
    }

    Ok(output)
}
