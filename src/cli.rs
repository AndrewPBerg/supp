use clap::{Parser, Subcommand};

#[derive(Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Diff {
        /// Path or registered repo name (defaults to '.')
        path: Option<String>,

        /// Only diff staged (cached) files
        #[arg(short = 'c', long)]
        cached: bool,

        /// Only diff untracked files
        #[arg(short = 'u', long)]
        untracked: bool,

        /// Only diff local (unstaged) changes
        #[arg(short = 'l', long)]
        local: bool,

        /// Branch to target for remote comparison (default: current branch)
        #[arg(short = 'b', long)]
        branch: Option<String>,
    },
    Tree {
        #[arg(long)]
        size: Option<String>,
    },
}
