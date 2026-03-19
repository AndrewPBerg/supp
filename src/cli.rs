use clap::{Parser, Subcommand};

#[derive(Parser)]
pub struct Cli {
    /// Skip copying to clipboard and just show the stats
    #[arg(short = 'n', long = "no-copy", aliases = &["no"], global = true)]
    pub no_copy: bool,

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

        /// Combine all local changes (untracked + staged + unstaged)
        #[arg(short = 'a', long)]
        all: bool,

        /// Compare current branch against its own remote (origin/<branch>)
        #[arg(short = 's', long)]
        self_branch: bool,
    },
    Tree {
        #[arg(long)]
        size: Option<String>,
    },
}
