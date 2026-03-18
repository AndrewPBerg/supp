use clap::{Parser, Subcommand};

#[derive(Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Diff {
        #[arg(long)]
        repo: Option<String>,
    },
    Tree {
        #[arg(long)]
        size: Option<String>,
    },
}
