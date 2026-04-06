use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(subcommand_negates_reqs = true)]
pub struct Cli {
    /// Print version and check for updates
    #[arg(long = "version")]
    pub version_flag: bool,
    /// Skip copying to clipboard and just show the stats
    #[arg(short = 'n', long = "no-copy", aliases = &["no"], global = true)]
    pub no_copy: bool,

    /// Disable colored output
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,

    /// Output as JSON (machine-readable)
    #[arg(short = 'j', long, global = true)]
    pub json: bool,

    /// Regex pattern to filter file paths (e.g. "src/.*\.rs$")
    #[arg(short = 'r', long = "regex", global = true)]
    pub regex: Option<String>,

    /// Paths for context generation (files and/or directories)
    pub paths: Vec<String>,

    /// Reduce noise: strip comments and collapse blank lines
    #[arg(long, global = true)]
    pub slim: bool,

    /// Outline mode: extract only signatures, types, and API surface
    #[arg(short = 'm', long, global = true, conflicts_with = "slim")]
    pub map: bool,

    /// Filter map output by symbol importance (0.0–1.0 percentile cutoff)
    #[arg(long = "map-threshold", global = true, value_name = "PERCENTILE")]
    pub map_threshold: Option<f64>,

    /// Tree depth in context header (default: 2)
    #[arg(short = 'd', long = "depth")]
    pub depth: Option<usize>,

    /// Performance mode override: full, balanced, lite
    #[arg(short = 'p', long = "perf", global = true)]
    pub perf: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Git diff with file tree, line counts, and full patch context
    Diff {
        /// Path or registered repo name (defaults to '.')
        path: Option<String>,

        /// Untracked files only
        #[arg(short = 'u', long)]
        untracked: bool,

        /// Unstaged changes to tracked files
        #[arg(short = 't', long)]
        tracked: bool,

        /// Staged changes only
        #[arg(short = 's', long)]
        staged: bool,

        /// All local changes vs self branch remote
        #[arg(short = 'l', long)]
        local: bool,

        /// All branch changes vs remote default main (default behavior)
        #[arg(short = 'a', long)]
        all: bool,

        /// Branch to compare to (used with -a)
        #[arg(short = 'b', long)]
        branch: Option<String>,

        /// Number of context lines in unified diff output
        #[arg(short = 'U', long = "unified")]
        context_lines: Option<u32>,
    },
    /// Project layout with git status markers (modified, added, untracked)
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
    /// Find functions, types, and constants by name across the codebase
    #[command(alias = "s")]
    Sym {
        /// Search query (free-form text, split into tokens)
        query: Vec<String>,
    },
    /// Explain a symbol: definition, docs, who calls it, and what it depends on
    #[command(alias = "w")]
    Why {
        /// Symbol name to look up (exact or fuzzy match)
        query: Vec<String>,
    },

    /// Interactively pick files with fzf for context generation
    #[command(alias = "p")]
    Pick {
        /// Root directory to search (defaults to ".")
        path: Option<String>,
        /// Select only a single file (skips confirmation and accumulation)
        #[arg(short = '1', long)]
        single: bool,
    },
    /// Show version and check for updates
    #[command(alias = "v")]
    Version,
    /// Update supp to the latest release
    Update,
    /// Remove supp from your system
    Uninstall,
    /// Delete the symbol cache for a project
    #[command(name = "clean-cache")]
    CleanCache {
        /// Project root (defaults to ".")
        path: Option<String>,
    },
    /// Set or check the global performance mode
    Perf {
        /// Mode to set (full, balanced, lite). Omit to check current mode.
        mode: Option<String>,
    },
}

impl Cli {
    pub fn resolve_depth(&self, config: &crate::config::Config) -> usize {
        self.depth.unwrap_or(config.global.depth)
    }

    pub fn resolve_no_copy(&self, config: &crate::config::Config) -> bool {
        self.no_copy || config.global.no_copy
    }

    pub fn resolve_no_color(&self, config: &crate::config::Config) -> bool {
        self.no_color || config.global.no_color
    }

    pub fn resolve_json(&self, config: &crate::config::Config) -> bool {
        self.json || config.global.json
    }

    pub fn resolve_mode(&self, config: &crate::config::Config) -> crate::compress::Mode {
        if self.map {
            crate::compress::Mode::Map
        } else if self.slim {
            crate::compress::Mode::Slim
        } else {
            match config.global.mode.as_str() {
                "slim" => crate::compress::Mode::Slim,
                "map" => crate::compress::Mode::Map,
                _ => crate::compress::Mode::Full,
            }
        }
    }

    pub fn resolve_map_threshold(&self) -> Option<f64> {
        self.map_threshold.map(|t| t.clamp(0.0, 1.0))
    }

    pub fn resolve_perf(&self, _config: &crate::config::Config) -> crate::config::PerfMode {
        // CLI flag (-p/--perf) > SUPP_PERF env var > persisted file > default (full)
        let raw = self
            .perf
            .clone()
            .or_else(|| std::env::var("SUPP_PERF").ok());
        match raw {
            Some(r) => r.parse().unwrap_or(crate::config::PerfMode::Full),
            None => crate::config::load_perf_mode(),
        }
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
        assert_eq!(cli.depth, Some(3));
        assert_eq!(cli.paths, vec!["src/"]);
    }

    #[test]
    fn context_default_depth() {
        let cli = parse(&["supp", "src/"]).unwrap();
        assert_eq!(cli.depth, None);
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
    fn diff_untracked() {
        let cli = parse(&["supp", "diff", "-u"]).unwrap();
        match cli.command {
            Some(Commands::Diff { untracked, .. }) => assert!(untracked),
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn diff_tracked() {
        let cli = parse(&["supp", "diff", "-t"]).unwrap();
        match cli.command {
            Some(Commands::Diff { tracked, .. }) => assert!(tracked),
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn diff_staged() {
        let cli = parse(&["supp", "diff", "-s"]).unwrap();
        match cli.command {
            Some(Commands::Diff { staged, .. }) => assert!(staged),
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

    // ── Version / Update / Uninstall ────────────────────────────

    #[test]
    fn version_subcommand() {
        let cli = parse(&["supp", "version"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Version)));
    }

    #[test]
    fn version_alias() {
        let cli = parse(&["supp", "v"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Version)));
    }

    #[test]
    fn update_subcommand() {
        let cli = parse(&["supp", "update"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Update)));
    }

    #[test]
    fn uninstall_subcommand() {
        let cli = parse(&["supp", "uninstall"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Uninstall)));
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
        let cli = parse(&["supp", "diff", "-n", "--no-color", "-s", "-r", r"\.rs$"]).unwrap();
        assert!(cli.no_copy);
        assert!(cli.no_color);
        assert_eq!(cli.regex.as_deref(), Some(r"\.rs$"));
        match cli.command {
            Some(Commands::Diff { staged, .. }) => {
                assert!(staged);
            }
            _ => panic!("expected diff"),
        }
    }

    // ── resolve_mode ────────────────────────────────────────────

    #[test]
    fn resolve_mode_default_is_full() {
        let cli = parse(&["supp", "src/"]).unwrap();
        let config = crate::config::Config::default();
        assert_eq!(cli.resolve_mode(&config), crate::compress::Mode::Full);
    }

    #[test]
    fn resolve_mode_slim() {
        let cli = parse(&["supp", "--slim", "src/"]).unwrap();
        let config = crate::config::Config::default();
        assert_eq!(cli.resolve_mode(&config), crate::compress::Mode::Slim);
    }

    #[test]
    fn resolve_mode_map() {
        let cli = parse(&["supp", "--map", "src/"]).unwrap();
        let config = crate::config::Config::default();
        assert_eq!(cli.resolve_mode(&config), crate::compress::Mode::Map);
    }

    // ── pick alias ──────────────────────────────────────────────

    #[test]
    fn pick_alias_p() {
        let cli = parse(&["supp", "p"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Pick { .. })));
    }

    // ── resolve_no_copy ────────────────────────────────────────

    #[test]
    fn resolve_no_copy_from_flag() {
        let cli = parse(&["supp", "-n", "src/"]).unwrap();
        let config = crate::config::Config::default();
        assert!(cli.resolve_no_copy(&config));
    }

    #[test]
    fn resolve_no_copy_from_config() {
        let cli = parse(&["supp", "src/"]).unwrap();
        let mut config = crate::config::Config::default();
        config.global.no_copy = true;
        assert!(cli.resolve_no_copy(&config));
    }

    #[test]
    fn resolve_no_copy_default() {
        let cli = parse(&["supp", "src/"]).unwrap();
        let config = crate::config::Config::default();
        assert!(!cli.resolve_no_copy(&config));
    }

    // ── resolve_no_color ───────────────────────────────────────

    #[test]
    fn resolve_no_color_from_flag() {
        let cli = parse(&["supp", "--no-color", "src/"]).unwrap();
        let config = crate::config::Config::default();
        assert!(cli.resolve_no_color(&config));
    }

    #[test]
    fn resolve_no_color_from_config() {
        let cli = parse(&["supp", "src/"]).unwrap();
        let mut config = crate::config::Config::default();
        config.global.no_color = true;
        assert!(cli.resolve_no_color(&config));
    }

    // ── resolve_depth ──────────────────────────────────────────

    #[test]
    fn resolve_depth_from_flag() {
        let cli = parse(&["supp", "-d", "5", "src/"]).unwrap();
        let config = crate::config::Config::default();
        assert_eq!(cli.resolve_depth(&config), 5);
    }

    #[test]
    fn resolve_depth_from_config() {
        let cli = parse(&["supp", "src/"]).unwrap();
        let config = crate::config::Config::default();
        assert_eq!(cli.resolve_depth(&config), 2); // default depth
    }

    // ── clean-cache subcommand ─────────────────────────────────

    #[test]
    fn clean_cache_subcommand() {
        let cli = parse(&["supp", "clean-cache"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::CleanCache { .. })));
    }

    #[test]
    fn clean_cache_with_path() {
        let cli = parse(&["supp", "clean-cache", "/tmp/repo"]).unwrap();
        match cli.command {
            Some(Commands::CleanCache { path }) => assert_eq!(path.as_deref(), Some("/tmp/repo")),
            _ => panic!("expected clean-cache"),
        }
    }

    // ── resolve_json ──────────────────────────────────────────────

    #[test]
    fn resolve_json_from_flag() {
        let cli = parse(&["supp", "-j", "src/"]).unwrap();
        let config = crate::config::Config::default();
        assert!(cli.resolve_json(&config));
    }

    #[test]
    fn resolve_json_from_config() {
        let cli = parse(&["supp", "src/"]).unwrap();
        let mut config = crate::config::Config::default();
        config.global.json = true;
        assert!(cli.resolve_json(&config));
    }

    #[test]
    fn resolve_json_default_false() {
        let cli = parse(&["supp", "src/"]).unwrap();
        let config = crate::config::Config::default();
        assert!(!cli.resolve_json(&config));
    }

    // ── resolve_mode from config ──────────────────────────────────

    #[test]
    fn resolve_mode_slim_from_config() {
        let cli = parse(&["supp", "src/"]).unwrap();
        let mut config = crate::config::Config::default();
        config.global.mode = "slim".to_string();
        assert_eq!(cli.resolve_mode(&config), crate::compress::Mode::Slim);
    }

    #[test]
    fn resolve_mode_map_from_config() {
        let cli = parse(&["supp", "src/"]).unwrap();
        let mut config = crate::config::Config::default();
        config.global.mode = "map".to_string();
        assert_eq!(cli.resolve_mode(&config), crate::compress::Mode::Map);
    }

    #[test]
    fn resolve_mode_unknown_config_defaults_to_full() {
        let cli = parse(&["supp", "src/"]).unwrap();
        let mut config = crate::config::Config::default();
        config.global.mode = "unknown".to_string();
        assert_eq!(cli.resolve_mode(&config), crate::compress::Mode::Full);
    }

    // ── sym and why subcommands ──────────────────────────────────

    #[test]
    fn sym_subcommand() {
        let cli = parse(&["supp", "sym", "hello"]).unwrap();
        match cli.command {
            Some(Commands::Sym { query }) => assert_eq!(query, vec!["hello"]),
            _ => panic!("expected sym"),
        }
    }

    #[test]
    fn sym_alias_s() {
        let cli = parse(&["supp", "s", "test"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Sym { .. })));
    }

    #[test]
    fn why_subcommand() {
        let cli = parse(&["supp", "why", "my_func"]).unwrap();
        match cli.command {
            Some(Commands::Why { query }) => assert_eq!(query, vec!["my_func"]),
            _ => panic!("expected why"),
        }
    }

    #[test]
    fn why_alias_w() {
        let cli = parse(&["supp", "w", "test"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Why { .. })));
    }

    #[test]
    fn version_flag() {
        let cli = parse(&["supp", "--version"]).unwrap();
        assert!(cli.version_flag);
    }
}
