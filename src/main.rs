mod cli;
mod compress;
mod context;
mod git;
mod pick;
mod styles;
mod tree;

use clap::Parser;

use cli::Cli;
use cli::Commands;
use git::{DiffOptions, get_diff};

fn main() -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    let cli = Cli::parse();

    if cli.no_color {
        colored::control::set_override(false);
    }

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
                context_lines,
                filter,
                regex: cli.regex,
            };
            let result = get_diff(repo_path, opts)?;
            let _ = text_tx.send(result.text.clone());
            styles::print_diff_result(result, cli.no_copy, start, token_handle);
        }
        Some(Commands::Completions { shell }) => {
            Cli::generate_completions(shell);
            return Ok(());
        }
        Some(Commands::Pick { ref path, single }) => {
            let root = path.as_deref().unwrap_or(".");
            let selected = pick::run_fzf(root, !single, cli.regex.as_deref())?;
            if selected.is_empty() {
                return Ok(());
            }
            let pick_start = std::time::Instant::now();
            let result = context::generate_context(&selected, cli.depth, cli.regex.as_deref(), cli.resolve_mode())?;
            let _ = text_tx.send(result.plain.clone());
            println!("{}", selected.join(" "));
            styles::print_pick_stats(result, cli.no_copy, pick_start, token_handle);
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
            let _ = text_tx.send(result.plain.clone());
            styles::print_tree_result(result, root, cli.no_copy, start, token_handle);
        }
        None => {
            if cli.paths.is_empty() {
                anyhow::bail!("no paths provided. Usage: supp <paths...> or supp <subcommand>");
            }
            let result = context::generate_context(&cli.paths, cli.depth, cli.regex.as_deref(), cli.resolve_mode())?;
            let _ = text_tx.send(result.plain.clone());
            styles::print_context_result(result, cli.no_copy, start, token_handle);
        }
    }
    Ok(())
}
