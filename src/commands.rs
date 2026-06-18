use serde::Serialize;

use crate::project;

#[derive(Debug, Clone, Serialize)]
pub struct CommandEntry {
    pub command: String,
    pub source: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandsResult {
    pub commands: Vec<CommandEntry>,
}

pub fn analyze(root: &str) -> anyhow::Result<CommandsResult> {
    let project = project::analyze(root)?;
    let mut commands = Vec::new();

    for command in project.common_commands {
        commands.push(CommandEntry {
            purpose: infer_purpose(&command).to_string(),
            source: infer_source(&command).to_string(),
            command,
        });
    }

    commands.sort_by(|a, b| {
        purpose_rank(&a.purpose)
            .cmp(&purpose_rank(&b.purpose))
            .then_with(|| a.command.cmp(&b.command))
    });
    commands.dedup_by(|a, b| a.command == b.command);

    Ok(CommandsResult { commands })
}

fn infer_source(command: &str) -> &'static str {
    if command.starts_with("uv run poe ") {
        "pyproject.toml [tool.poe.tasks]"
    } else if command.starts_with("uv run ") || command == "pytest" {
        "pyproject.toml / pytest"
    } else if command.starts_with("cargo ") {
        "Cargo.toml"
    } else if command.starts_with("go ") {
        "go.mod"
    } else if command.starts_with("npm run")
        || command.starts_with("pnpm ")
        || command.starts_with("yarn ")
        || command.starts_with("bun ")
    {
        "package.json scripts"
    } else {
        "project config"
    }
}

fn infer_purpose(command: &str) -> &'static str {
    let c = command.to_lowercase();
    if c.contains("clippy") || c.contains("lint") {
        "lint"
    } else if c.contains("typecheck") || c.contains("check") {
        "check"
    } else if c.contains("test") || c.contains("pytest") {
        "test"
    } else if c.contains("bench") {
        "benchmark"
    } else if c.contains("eval") {
        "eval"
    } else {
        "other"
    }
}

fn purpose_rank(purpose: &str) -> usize {
    match purpose {
        "test" => 0,
        "check" => 1,
        "lint" => 2,
        "eval" => 3,
        "benchmark" => 4,
        _ => 9,
    }
}
