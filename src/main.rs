mod cli;
mod git;

use clap::Parser;

use cli::Cli;
use cli::Commands;
use git::get_diff;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Diff { repo, .. } => get_diff(&repo.as_deref().unwrap_or("."))?, // do diff stufss
        Commands::Tree { size, .. } => println!("tree! size: {:?}", size),         // do tree tings
    }
    Ok(())
}
