mod cli;
mod git;

use clap::Parser;

use cli::Cli;
use cli::Commands;
use git::{DiffOptions, get_diff};

#[cfg(target_os = "linux")]
fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let tools: &[(&str, &[&str])] = &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])];
    for (tool, args) in tools {
        if let Ok(mut child) = Command::new(tool).args(*args).stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(text.as_bytes())?;
            }
            return Ok(());
        }
    }
    Err(anyhow::anyhow!(
        "no clipboard tool found — install wl-clipboard (Wayland) or xclip (X11)"
    ))
}

#[cfg(not(target_os = "linux"))]
fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    let mut cb = arboard::Clipboard::new()?;
    cb.set_text(text)?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Diff {
            path,
            cached,
            untracked,
            local,
            branch,
        } => {
            let repo_path = path.as_deref().unwrap_or(".");
            let opts = DiffOptions {
                cached,
                untracked,
                local,
                branch,
            };
            let diff_text = get_diff(repo_path, opts)?;
            if diff_text.is_empty() {
                println!("No diff found.");
            } else {
                println!("Diff copied to clipboard ({} bytes)", diff_text.len());
                copy_to_clipboard(&diff_text)?;
            }
        }
        Commands::Tree { size, .. } => println!("tree! size: {:?}", size),
    }
    Ok(())
}
