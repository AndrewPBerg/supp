use clap::{Parser, Subcommand};

#[derive(Parser)]
pub struct Cli {
    #[arg(long)]
    pub repo: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Diff,
    Tree,
}
