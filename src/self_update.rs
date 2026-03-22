use std::io::Write;
use std::process::Command;

use anyhow::{Result, bail};
use colored::Colorize;

const REPO: &str = "AndrewPBerg/supp";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

fn fetch_latest_version() -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body: serde_json::Value = ureq::get(&url)
        .header("User-Agent", "supp")
        .call()?
        .into_body()
        .read_json()?;
    let tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing tag_name in release response"))?;
    Ok(tag.strip_prefix('v').unwrap_or(tag).to_string())
}

pub fn print_version() {
    println!("supp {}", format!("v{CURRENT_VERSION}").bold());

    match fetch_latest_version() {
        Ok(latest) if latest != CURRENT_VERSION => {
            println!(
                "{}",
                format!("Update available: v{CURRENT_VERSION} → v{latest} (run `supp update`)")
                    .yellow()
            );
        }
        _ => {}
    }
}

pub fn self_update() -> Result<()> {
    print!("Checking for updates... ");
    std::io::stdout().flush()?;

    let latest = fetch_latest_version()?;

    if latest == CURRENT_VERSION {
        println!("{}", "already up to date.".green());
        return Ok(());
    }

    println!("v{CURRENT_VERSION} → v{latest}");

    // Install to the same directory as the current binary
    let current_exe = std::env::current_exe()?;
    let install_dir = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine install directory"))?;

    println!("Downloading and running install script...");

    let status = if cfg!(windows) {
        let script_url = format!("https://raw.githubusercontent.com/{REPO}/main/install.ps1");
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &format!(
                    "&([scriptblock]::Create((irm '{url}'))) -Version '{latest}' -InstallDir '{dir}'",
                    url = script_url,
                    latest = latest,
                    dir = install_dir.display(),
                ),
            ])
            .status()?
    } else {
        let script_url = format!("https://raw.githubusercontent.com/{REPO}/main/install.sh");
        Command::new("bash")
            .args([
                "-c",
                &format!(
                    "VERSION={latest} INSTALL_DIR={dir} bash <(curl -fsSL {url})",
                    latest = latest,
                    dir = install_dir.display(),
                    url = script_url,
                ),
            ])
            .status()?
    };

    if !status.success() {
        bail!("Update failed — install script exited with error");
    }

    println!("{}", format!("supp updated to v{latest}").green().bold());
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let exe = std::env::current_exe()?;
    print!(
        "Remove supp at {}? [y/N] ",
        exe.display().to_string().bold()
    );
    std::io::stdout().flush()?;

    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;

    if !answer.trim().eq_ignore_ascii_case("y") {
        println!("Cancelled.");
        return Ok(());
    }

    // Try direct removal first, fall back to sudo
    if std::fs::remove_file(&exe).is_err() {
        let status = Command::new("sudo")
            .args(["rm", exe.to_str().unwrap()])
            .status()?;
        if !status.success() {
            bail!("Failed to remove binary");
        }
    }

    println!("{}", "supp has been removed.".green());
    Ok(())
}
