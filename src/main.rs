mod cli;
mod compress;
mod config;
mod context;
mod ctx;
mod git;
mod mcp;
mod pick;
mod styles;
mod symbol;
mod tree;
mod why;

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

    if cli.resolve_no_color(&config) {
        colored::control::set_override(false);
    }

    let no_copy = cli.resolve_no_copy(&config);
    let max_untracked_size = config.limits.max_untracked_file_size_mb * 1024 * 1024;

    // Commands that don't need token counting — handle early without spawning the thread
    match cli.command {
        Some(Commands::Completions { shell }) => {
            Cli::generate_completions(shell);
            return Ok(());
        }
        Some(Commands::Sym { ref query }) => {
            let result = symbol::search(".", query)?;
            styles::print_sym_results(&result, no_copy, start);
            return Ok(());
        }
        Some(Commands::Why { ref query }) => {
            let result = why::explain(".", query)?;
            styles::print_why_result(&result, no_copy, start);
            return Ok(());
        }
        Some(Commands::Mcp) => {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(mcp::run())?;
            return Ok(());
        }
        _ => {}
    }

    // Spawn token-counting thread only for commands that use it
    let (text_tx, text_rx) = std::sync::mpsc::channel::<String>();
    let token_handle = std::thread::spawn(move || {
        let bpe = tiktoken_rs::cl100k_base().ok()?;
        let text: String = text_rx.recv().ok()?;
        Some(bpe.encode_with_special_tokens(&text).len())
    });

    match cli.command {
        Some(Commands::Diff {
            path,
            cached,
            untracked,
            local,
            branch,
            all,
            self_branch,
            context_lines,
            filter,
        }) => {
            let repo_path = path.as_deref().unwrap_or(".");
            let opts = DiffOptions {
                cached,
                untracked,
                local,
                branch,
                all,
                self_branch,
                context_lines: context_lines.or(Some(config.diff.context_lines)),
                filter,
                max_untracked_size,
            };
            let result = get_diff(repo_path, opts, cli.regex.as_deref())?;
            styles::print_diff_result(result, no_copy, start, text_tx, token_handle);
        }
        Some(Commands::Pick { ref path, single }) => {
            let root = path.as_deref().unwrap_or(".");
            if single {
                // Single mode: no confirmation, straight to analysis
                let selected = pick::run_fzf(root, false, cli.regex.as_deref(), config.pick.preview_lines)?;
                if selected.is_empty() {
                    return Ok(());
                }
                let pick_start = std::time::Instant::now();
                let depth = cli.resolve_depth(&config);
                let mode = cli.resolve_mode(&config);
                let result = context::generate_context(&selected, depth, cli.regex.as_deref(), mode)?;
                println!("{}", selected.join(" "));
                styles::print_pick_stats(result, no_copy, pick_start, text_tx, token_handle);
            } else {
                // Multi mode: confirm → accumulate → execute
                let confirmed = pick::pick_with_confirm(root, cli.regex.as_deref(), config.pick.preview_lines)?;
                if confirmed.is_empty() {
                    return Ok(());
                }
                let final_files = pick::interactive_pick_loop(root, cli.regex.as_deref(), config.pick.preview_lines, confirmed)?;
                if final_files.is_empty() {
                    return Ok(());
                }
                let pick_start = std::time::Instant::now();
                let depth = cli.resolve_depth(&config);
                let mode = cli.resolve_mode(&config);
                let result = context::generate_context(&final_files, depth, cli.regex.as_deref(), mode)?;
                println!("{}", final_files.join(" "));
                styles::print_pick_stats(result, no_copy, pick_start, text_tx, token_handle);
            }
            return Ok(());
        }
        Some(Commands::Tree { path, depth, no_git }) => {
            let root = path.as_deref().unwrap_or(".");

            let statuses = if no_git {
                None
            } else {
                git::get_status_map(root)?
            };

            let status_ref = statuses.as_ref().map(|(map, prefix)| (map, prefix.as_str()));
            let result = tree::build_tree(root, depth, cli.regex.as_deref(), status_ref)?;
            styles::print_tree_result(result, root, no_copy, start, text_tx, token_handle);
        }
        None => {
            let mode = cli.resolve_mode(&config);
            // Expand any "p" tokens via fzf before processing
            let has_p = cli.paths.iter().any(|p| p == "p");
            let paths = if has_p {
                let expanded = pick::expand_p_tokens(&cli.paths, cli.regex.as_deref(), config.pick.preview_lines)?;
                if !expanded.is_empty() {
                    // Print resolved command so user can copy/rerun without fzf
                    eprintln!("{}", format!("supp {}", expanded.join(" ")).dimmed());
                }
                expanded
            } else {
                cli.paths.clone()
            };

            if paths.is_empty() {
                if has_p {
                    return Ok(());
                }
                let selected = pick::run_fzf(".", false, cli.regex.as_deref(), config.pick.preview_lines)?;
                if selected.is_empty() {
                    return Ok(());
                }
                let file = selected.into_iter().next().unwrap();
                let depth = cli.resolve_depth(&config);
                let result = ctx::analyze(".", &[file], depth, cli.regex.as_deref(), mode)?;
                styles::print_ctx_result(&result, no_copy, start, text_tx, token_handle);
            } else if paths.len() == 1 && std::path::Path::new(&paths[0]).is_file() {
                let depth = cli.resolve_depth(&config);
                let result = ctx::analyze(".", &paths, depth, cli.regex.as_deref(), mode)?;
                styles::print_ctx_result(&result, no_copy, start, text_tx, token_handle);
            } else {
                let depth = cli.resolve_depth(&config);
                let result = context::generate_context(&paths, depth, cli.regex.as_deref(), mode)?;
                styles::print_context_result(result, no_copy, start, text_tx, token_handle);
            }
        }
        Some(Commands::Completions { .. } | Commands::Sym { .. } | Commands::Why { .. } | Commands::Mcp) => {
            unreachable!()
        }
    }
    Ok(())
}
