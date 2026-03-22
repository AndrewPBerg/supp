mod cli;
mod compress;
mod config;
mod ctx;
mod git;

mod pick;
mod self_update;
mod styles;
mod symbol;
mod tree;
mod why;

use std::io::IsTerminal;

use clap::Parser;
use colored::Colorize;

use cli::Cli;
use cli::Commands;
use config::Config;
use git::{DiffOptions, get_diff};

fn main() -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    let cli = Cli::parse();
    let config = Config::load();

    if cli.version_flag {
        self_update::print_version();
        return Ok(());
    }

    let json = cli.resolve_json(&config);
    let piped = !std::io::stdout().is_terminal();

    if cli.resolve_no_color(&config) || json || piped {
        colored::control::set_override(false);
    }

    let no_copy = json || piped || cli.resolve_no_copy(&config);
    let max_untracked_size = config.limits.max_untracked_file_size_mb * 1024 * 1024;
    let max_files = config.limits.max_files;
    let max_total_bytes = config.limits.max_total_mb * 1024 * 1024;

    // Commands that don't need token counting — handle early without spawning the thread
    match cli.command {
        Some(Commands::Completions { shell }) => {
            Cli::generate_completions(shell);
            return Ok(());
        }
        Some(Commands::Sym { ref query }) => {
            let result = symbol::search(".", query)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                styles::print_sym_results(&result, no_copy, start);
            }
            return Ok(());
        }
        Some(Commands::Why { ref query }) => {
            let result = why::explain(".", query)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                styles::print_why_result(&result, no_copy, start);
            }
            return Ok(());
        }

        Some(Commands::Version) => {
            self_update::print_version();
            return Ok(());
        }
        Some(Commands::Update) => {
            self_update::self_update()?;
            return Ok(());
        }
        Some(Commands::Uninstall) => {
            self_update::uninstall()?;
            return Ok(());
        }
        Some(Commands::CleanCache { ref path }) => {
            let root = path.as_deref().unwrap_or(".");
            symbol::clean_cache(root)?;
            eprintln!(
                "Symbol cache deleted for {}",
                std::fs::canonicalize(root)?.display()
            );
            return Ok(());
        }
        _ => {}
    }

    match cli.command {
        Some(Commands::Diff {
            path,
            untracked,
            tracked,
            staged,
            local,
            all,
            branch,
            context_lines,
        }) => {
            let repo_path = path.as_deref().unwrap_or(".");
            let opts = DiffOptions {
                untracked,
                tracked,
                staged,
                local,
                all,
                branch,
                context_lines: context_lines.or(Some(config.diff.context_lines)),
                max_untracked_size,
            };
            let result = get_diff(repo_path, opts, cli.regex.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                styles::print_diff_result(result, no_copy, start);
            }
        }
        Some(Commands::Pick { ref path, single }) => {
            let root = path.as_deref().unwrap_or(".");
            let mut picked_mode = None;
            let selected = if single {
                let s =
                    pick::run_fzf(root, false, cli.regex.as_deref(), config.pick.preview_lines)?;
                if s.is_empty() {
                    return Ok(());
                }
                s
            } else {
                let (f, mode_override) = pick::interactive_pick_loop(
                    root,
                    cli.regex.as_deref(),
                    config.pick.preview_lines,
                )?;
                if f.is_empty() {
                    return Ok(());
                }
                picked_mode = mode_override;
                f
            };
            let pick_start = std::time::Instant::now();
            let depth = cli.resolve_depth(&config);
            let mode = picked_mode.unwrap_or_else(|| cli.resolve_mode(&config));
            let result = ctx::analyze(
                ".",
                &selected,
                depth,
                cli.regex.as_deref(),
                mode,
                max_files,
                max_total_bytes,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("{}", selected.join(" "));
                styles::print_pick_stats(&result, no_copy, pick_start);
            }
            return Ok(());
        }
        Some(Commands::Tree {
            path,
            depth,
            no_git,
        }) => {
            let root = path.as_deref().unwrap_or(".");

            let statuses = if no_git {
                None
            } else {
                git::get_status_map(root)?
            };

            let status_ref = statuses
                .as_ref()
                .map(|(map, prefix)| (map, prefix.as_str()));
            let result = tree::build_tree(root, depth, cli.regex.as_deref(), status_ref)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                styles::print_tree_result(result, root, no_copy, start);
            }
        }
        None => {
            let mode = cli.resolve_mode(&config);
            // Expand any "p" tokens via fzf before processing
            let has_p = cli.paths.iter().any(|p| p == "p");
            let paths = if has_p {
                let expanded = pick::expand_p_tokens(
                    &cli.paths,
                    cli.regex.as_deref(),
                    config.pick.preview_lines,
                )?;
                if !expanded.is_empty() {
                    // Print resolved command so user can copy/rerun without fzf
                    eprintln!("{}", format!("supp {}", expanded.join(" ")).dimmed());
                }
                expanded
            } else {
                cli.paths.clone()
            };

            let paths = if paths.is_empty() {
                if has_p {
                    return Ok(());
                }
                vec![".".to_string()]
            } else {
                paths
            };

            if paths.len() == 1 && std::path::Path::new(&paths[0]).is_file() {
                let depth = cli.resolve_depth(&config);
                let result = ctx::analyze(
                    ".",
                    &paths,
                    depth,
                    cli.regex.as_deref(),
                    mode,
                    max_files,
                    max_total_bytes,
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    styles::print_ctx_result(&result, no_copy, start);
                }
            } else {
                let depth = cli.resolve_depth(&config);
                let result = ctx::analyze(
                    ".",
                    &paths,
                    depth,
                    cli.regex.as_deref(),
                    mode,
                    max_files,
                    max_total_bytes,
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    styles::print_context_result(&result, no_copy, start);
                }
            }
        }
        Some(
            Commands::Completions { .. }
            | Commands::Sym { .. }
            | Commands::Why { .. }
            | Commands::Version
            | Commands::Update
            | Commands::Uninstall
            | Commands::CleanCache { .. },
        ) => {
            unreachable!()
        }
    }
    Ok(())
}
