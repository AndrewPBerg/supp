mod cli;
mod git;

use clap::Parser;

use cli::Cli;
use cli::Commands;
use git::get_diff;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Diff => get_diff(".")?,    // do diff stufss
        Commands::Tree => println!("tree!"), // do tree tings
    }
    Ok(())
}
