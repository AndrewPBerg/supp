use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(subcommand_negates_reqs = true)]
pub struct Cli {
    /// Skip copying to clipboard and just show the stats
    #[arg(short = 'n', long = "no-copy", aliases = &["no"], global = true)]
    pub no_copy: bool,

    /// Disable colored output
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,

    /// Regex pattern to filter file paths (e.g. "src/.*\.rs$")
    #[arg(short = 'r', long = "regex", global = true)]
    pub regex: Option<String>,

    /// Paths for context generation (files and/or directories)
    pub paths: Vec<String>,

    /// Strip comments and collapse blank lines
    #[arg(short = 'S', long, global = true)]
    pub slim: bool,

    /// Extract only signatures and definitions (codemap)
    #[arg(short = 'M', long, global = true, conflicts_with = "slim")]
    pub map: bool,

    /// Tree depth in context header (default: 2)
    #[arg(short = 'd', long = "depth", default_value = "2")]
    pub depth: usize,

    #[command(subcommand)]
    pub command: Option<Commands>,
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

        /// Number of context lines in unified diff output
        #[arg(short = 'U', long = "unified")]
        context_lines: Option<u32>,

        /// Glob pattern to filter files (e.g. "*.rs")
        #[arg(short = 'f', long = "filter")]
        filter: Option<String>,

    },
    Tree {
        /// Directory to display (defaults to ".")
        path: Option<String>,
        /// Maximum depth to display
        #[arg(short = 'd', long)]
        depth: Option<usize>,
        /// Disable git status indicators
        #[arg(long = "no-git")]
        no_git: bool,
    },
    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
    /// Interactively pick files with fzf for context generation
    #[command(alias = "p")]
    Pick {
        /// Root directory to search (defaults to ".")
        path: Option<String>,
        /// Select only a single file (no --multi)
        #[arg(short = 's', long)]
        single: bool,
    },
}

impl Cli {
    pub fn resolve_mode(&self) -> crate::compress::Mode {
        if self.map { crate::compress::Mode::Map } else if self.slim { crate::compress::Mode::Slim } else { crate::compress::Mode::Full }
    }

    pub fn generate_completions(shell: clap_complete::Shell) {
        let mut cmd = Cli::command();
        clap_complete::generate(shell, &mut cmd, "supp", &mut std::io::stdout());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    // ── Subcommand basics ────────────────────────────────────────

    #[test]
    fn diff_subcommand() {
        let cli = parse(&["supp", "diff"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Diff { .. })));
    }

    #[test]
    fn tree_subcommand() {
        let cli = parse(&["supp", "tree"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Tree { .. })));
    }

    #[test]
    fn no_subcommand_no_paths_succeeds_empty() {
        let cli = parse(&["supp"]).unwrap();
        assert!(cli.command.is_none());
        assert!(cli.paths.is_empty());
    }

    #[test]
    fn context_positional_paths() {
        let cli = parse(&["supp", "src/main.rs", "src/cli.rs"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.paths, vec!["src/main.rs", "src/cli.rs"]);
    }

    #[test]
    fn context_depth_flag() {
        let cli = parse(&["supp", "-d", "3", "src/"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.depth, 3);
        assert_eq!(cli.paths, vec!["src/"]);
    }

    #[test]
    fn context_default_depth() {
        let cli = parse(&["supp", "src/"]).unwrap();
        assert_eq!(cli.depth, 2);
    }

    // ── Global flags ─────────────────────────────────────────────

    #[test]
    fn no_copy_short() {
        let cli = parse(&["supp", "-n", "diff"]).unwrap();
        assert!(cli.no_copy);
    }

    #[test]
    fn no_copy_long() {
        let cli = parse(&["supp", "--no-copy", "diff"]).unwrap();
        assert!(cli.no_copy);
    }

    #[test]
    fn no_color_flag() {
        let cli = parse(&["supp", "--no-color", "diff"]).unwrap();
        assert!(cli.no_color);
    }

    #[test]
    fn regex_before_subcommand() {
        let cli = parse(&["supp", "-r", "src/.*\\.rs$", "diff"]).unwrap();
        assert_eq!(cli.regex.as_deref(), Some("src/.*\\.rs$"));
    }

    // ── Diff flags ───────────────────────────────────────────────

    #[test]
    fn diff_cached() {
        let cli = parse(&["supp", "diff", "-c"]).unwrap();
        match cli.command {
            Some(Commands::Diff { cached, .. }) => assert!(cached),
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn diff_untracked() {
        let cli = parse(&["supp", "diff", "-u"]).unwrap();
        match cli.command {
            Some(Commands::Diff { untracked, .. }) => assert!(untracked),
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn diff_local() {
        let cli = parse(&["supp", "diff", "-l"]).unwrap();
        match cli.command {
            Some(Commands::Diff { local, .. }) => assert!(local),
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn diff_all() {
        let cli = parse(&["supp", "diff", "-a"]).unwrap();
        match cli.command {
            Some(Commands::Diff { all, .. }) => assert!(all),
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn diff_self_branch() {
        let cli = parse(&["supp", "diff", "-s"]).unwrap();
        match cli.command {
            Some(Commands::Diff { self_branch, .. }) => assert!(self_branch),
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn diff_branch() {
        let cli = parse(&["supp", "diff", "-b", "develop"]).unwrap();
        match cli.command {
            Some(Commands::Diff { branch, .. }) => assert_eq!(branch.as_deref(), Some("develop")),
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn diff_context_lines() {
        let cli = parse(&["supp", "diff", "-U", "5"]).unwrap();
        match cli.command {
            Some(Commands::Diff { context_lines, .. }) => assert_eq!(context_lines, Some(5)),
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn diff_filter() {
        let cli = parse(&["supp", "diff", "-f", "*.rs"]).unwrap();
        match cli.command {
            Some(Commands::Diff { filter, .. }) => assert_eq!(filter.as_deref(), Some("*.rs")),
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn diff_positional_path() {
        let cli = parse(&["supp", "diff", "/tmp/repo"]).unwrap();
        match cli.command {
            Some(Commands::Diff { path, .. }) => assert_eq!(path.as_deref(), Some("/tmp/repo")),
            _ => panic!("expected diff"),
        }
    }

    // ── Tree flags ───────────────────────────────────────────────

    #[test]
    fn tree_depth() {
        let cli = parse(&["supp", "tree", "-d", "3"]).unwrap();
        match cli.command {
            Some(Commands::Tree { depth, .. }) => assert_eq!(depth, Some(3)),
            _ => panic!("expected tree"),
        }
    }

    #[test]
    fn tree_no_git() {
        let cli = parse(&["supp", "tree", "--no-git"]).unwrap();
        match cli.command {
            Some(Commands::Tree { no_git, .. }) => assert!(no_git),
            _ => panic!("expected tree"),
        }
    }

    #[test]
    fn tree_positional_path() {
        let cli = parse(&["supp", "tree", "/tmp/dir"]).unwrap();
        match cli.command {
            Some(Commands::Tree { path, .. }) => assert_eq!(path.as_deref(), Some("/tmp/dir")),
            _ => panic!("expected tree"),
        }
    }

    // ── Completions ─────────────────────────────────────────────

    #[test]
    fn completions_subcommand() {
        let cli = parse(&["supp", "completions", "bash"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Completions { .. })));
    }

    // ── Pick flags ──────────────────────────────────────────────

    #[test]
    fn pick_subcommand() {
        let cli = parse(&["supp", "pick"]).unwrap();
        match cli.command {
            Some(Commands::Pick { path, single }) => {
                assert!(path.is_none());
                assert!(!single);
            }
            _ => panic!("expected pick"),
        }
    }

    #[test]
    fn pick_single() {
        let cli = parse(&["supp", "pick", "--single"]).unwrap();
        match cli.command {
            Some(Commands::Pick { single, .. }) => assert!(single),
            _ => panic!("expected pick"),
        }
    }

    #[test]
    fn pick_with_path() {
        let cli = parse(&["supp", "pick", "/tmp/dir"]).unwrap();
        match cli.command {
            Some(Commands::Pick { path, .. }) => assert_eq!(path.as_deref(), Some("/tmp/dir")),
            _ => panic!("expected pick"),
        }
    }

    // ── Compression flags ────────────────────────────────────────

    #[test]
    fn slim_flag() {
        let cli = parse(&["supp", "--slim", "src/"]).unwrap();
        assert!(cli.slim);
        assert!(!cli.map);
    }

    #[test]
    fn map_flag() {
        let cli = parse(&["supp", "--map", "src/"]).unwrap();
        assert!(cli.map);
        assert!(!cli.slim);
    }

    #[test]
    fn slim_position_independent() {
        let cli = parse(&["supp", "src/", "--slim"]).unwrap();
        assert!(cli.slim);
    }

    #[test]
    fn map_position_independent() {
        let cli = parse(&["supp", "src/", "--map"]).unwrap();
        assert!(cli.map);
    }

    #[test]
    fn slim_and_map_conflict() {
        let result = parse(&["supp", "--slim", "--map", "src/"]);
        assert!(result.is_err());
    }

    // ── Combined flags ───────────────────────────────────────────

    #[test]
    fn combined_global_and_diff_flags() {
        let cli = parse(&["supp", "diff", "-n", "--no-color", "-c", "-f", "*.rs"]).unwrap();
        assert!(cli.no_copy);
        assert!(cli.no_color);
        match cli.command {
            Some(Commands::Diff { cached, filter, .. }) => {
                assert!(cached);
                assert_eq!(filter.as_deref(), Some("*.rs"));
            }
            _ => panic!("expected diff"),
        }
    }
}
