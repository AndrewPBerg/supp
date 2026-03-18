use anyhow::Result;
use git2::Repository;

pub fn get_diff(repo_path: &str) -> Result<()> {
    // 1. open repo
    Repository::open(repo_path)?;
    // 2. get diff
    //
    // 3. return it
    Ok(())
}
