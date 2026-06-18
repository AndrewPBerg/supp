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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn infer_source_and_purpose_cover_supported_stacks() {
        assert_eq!(
            infer_source("uv run poe lint"),
            "pyproject.toml [tool.poe.tasks]"
        );
        assert_eq!(infer_source("uv run pytest"), "pyproject.toml / pytest");
        assert_eq!(infer_source("cargo test"), "Cargo.toml");
        assert_eq!(infer_source("go test ./..."), "go.mod");
        assert_eq!(infer_source("npm run test"), "package.json scripts");
        assert_eq!(infer_source("custom"), "project config");

        assert_eq!(infer_purpose("cargo clippy"), "lint");
        assert_eq!(infer_purpose("npm run typecheck"), "check");
        assert_eq!(infer_purpose("pytest"), "test");
        assert_eq!(infer_purpose("cargo bench"), "benchmark");
        assert_eq!(infer_purpose("run eval"), "eval");
        assert_eq!(infer_purpose("serve"), "other");
    }

    #[test]
    fn analyze_lists_sorted_deduped_commands() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let result = analyze(dir.path().to_str().unwrap()).unwrap();
        let commands: Vec<_> = result.commands.iter().map(|c| c.command.as_str()).collect();
        assert_eq!(commands, vec!["cargo test", "cargo clippy"]);
        assert_eq!(result.commands[0].purpose, "test");
        assert_eq!(result.commands[1].purpose, "lint");
    }

    #[test]
    fn purpose_rank_orders_known_purposes_before_other() {
        assert!(purpose_rank("test") < purpose_rank("lint"));
        assert!(purpose_rank("benchmark") < purpose_rank("other"));
    }
}
