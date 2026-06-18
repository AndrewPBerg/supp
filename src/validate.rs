use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::project;
use crate::tests_for;

#[derive(Debug, Serialize, Clone)]
pub struct ValidateResult {
    pub target: String,
    pub commands: Vec<String>,
    pub ran: bool,
    pub exit_code: Option<i32>,
    pub output: Option<String>,
    pub warnings: Vec<String>,
}

pub fn plan(
    root: &str,
    target: Option<&str>,
    changed: bool,
    fast: bool,
    run: bool,
) -> anyhow::Result<ValidateResult> {
    let project_info = project::analyze(root)?;
    let root_path = project::discover_root(root)?;
    let mut commands = BTreeSet::new();
    let label = if changed {
        "--changed".to_string()
    } else {
        target.unwrap_or(".").to_string()
    };
    let mut warnings = Vec::new();

    if changed {
        let tests = tests_for::analyze(root, None, true, 20)?;
        for cmd in tests.focused_commands {
            commands.insert(cmd);
        }
        if commands.is_empty() {
            add_project_defaults(&mut commands, &project_info, fast);
        }
    } else if let Some(t) = target {
        let normalized = project::normalize_target(&root_path, t);
        let tests = tests_for::analyze(root, Some(t), false, 20)?;
        for cmd in tests.focused_commands {
            commands.insert(cmd);
        }
        if commands.is_empty() {
            add_for_path(&mut commands, &normalized, &project_info);
        }
    } else {
        add_project_defaults(&mut commands, &project_info, fast);
    }

    if commands.is_empty() {
        warnings.push(
            "Could not infer a validation command; inspect project config manually.".to_string(),
        );
    }

    let commands_vec: Vec<String> = commands.into_iter().collect();
    let mut result = ValidateResult {
        target: label,
        commands: commands_vec.clone(),
        ran: false,
        exit_code: None,
        output: None,
        warnings,
    };

    if run && let Some(cmd) = commands_vec.first() {
        let output = std::process::Command::new("sh")
            .arg("-lc")
            .arg(cmd)
            .current_dir(&root_path)
            .output()?;
        result.ran = true;
        result.exit_code = output.status.code();
        let mut text = String::new();
        text.push_str(&String::from_utf8_lossy(&output.stdout));
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        result.output = Some(text);
    }

    Ok(result)
}

fn add_project_defaults(commands: &mut BTreeSet<String>, p: &project::ProjectResult, fast: bool) {
    if p.package_managers.iter().any(|m| m == "cargo") {
        let _ = fast;
        commands.insert("cargo test".to_string());
        return;
    }
    if p.package_managers.iter().any(|m| m == "go") {
        commands.insert("go test ./...".to_string());
        return;
    }
    if p.test_runners.iter().any(|r| r == "pytest") {
        let prefix = if p.package_managers.iter().any(|m| m == "uv") {
            "uv run "
        } else {
            ""
        };
        commands.insert(if fast {
            format!("{prefix}pytest -m unit")
        } else {
            format!("{prefix}pytest")
        });
        return;
    }
    if let Some(cmd) = p.common_commands.iter().find(|c| c.contains("test")) {
        commands.insert(cmd.clone());
    }
}

fn add_for_path(commands: &mut BTreeSet<String>, target: &str, p: &project::ProjectResult) {
    let dir = Path::new(target)
        .parent()
        .and_then(|d| d.to_str())
        .unwrap_or(".");
    if target.ends_with(".rs") {
        commands.insert("cargo test".to_string());
    } else if target.ends_with(".go") {
        commands.insert(format!(
            "go test ./{}",
            if dir == "." { "..." } else { dir }
        ));
    } else if target.ends_with(".py") {
        let prefix = if p.package_managers.iter().any(|m| m == "uv") {
            "uv run "
        } else {
            ""
        };
        commands.insert(format!("{prefix}pytest {target}"));
    }
}
