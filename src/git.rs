use anyhow::Result;
use git2::{Diff, Repository};

pub fn get_diff(repo_path: &str) -> Result<()> {
    // 1. open repo
    let repo = Repository::open(repo_path)?;
    // 2. get diff
    let diff: Diff = repo.diff_index_to_workdir(None, None)?;
    // 3. print it
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        // print the line content here
        // line.content() gives you the raw bytes
        if let Ok(s) = std::str::from_utf8(line.content()) {
            print!("{}", s);
        }
        true // return true to continue iteration
    })?;
    Ok(())
}
