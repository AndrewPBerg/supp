use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use regex::Regex;
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct ProjectResult {
    pub root: String,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub package_managers: Vec<String>,
    pub test_runners: Vec<String>,
    pub test_paths: Vec<String>,
    pub common_commands: Vec<String>,
    pub entrypoints: Vec<String>,
    pub instruction_files: Vec<String>,
}

pub fn analyze(root: &str) -> anyhow::Result<ProjectResult> {
    let root_path = std::fs::canonicalize(root)?;
    let files = collect_files(&root_path);
    let file_set: BTreeSet<String> = files.iter().cloned().collect();

    let mut languages = BTreeSet::new();
    let mut frameworks = BTreeSet::new();
    let mut package_managers = BTreeSet::new();
    let mut test_runners = BTreeSet::new();
    let mut test_paths = BTreeSet::new();
    let mut common_commands = BTreeSet::new();
    let mut entrypoints = BTreeSet::new();
    let mut instruction_files = Vec::new();

    for f in &files {
        match Path::new(f)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
        {
            "rs" => {
                languages.insert("Rust".to_string());
            }
            "go" => {
                languages.insert("Go".to_string());
            }
            "py" => {
                languages.insert("Python".to_string());
            }
            "ts" | "tsx" => {
                languages.insert("TypeScript".to_string());
            }
            "js" | "jsx" => {
                languages.insert("JavaScript".to_string());
            }
            "java" => {
                languages.insert("Java".to_string());
            }
            "c" | "h" => {
                languages.insert("C".to_string());
            }
            "cpp" | "cc" | "cxx" | "hpp" => {
                languages.insert("C++".to_string());
            }
            _ => {}
        }

        let name = Path::new(f)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if matches!(name, "AGENTS.md" | "CLAUDE.md" | "README.md") {
            instruction_files.push(f.clone());
        }
    }

    if file_set.contains("Cargo.toml") {
        package_managers.insert("cargo".to_string());
        test_runners.insert("cargo test".to_string());
        common_commands.insert("cargo test".to_string());
        common_commands.insert("cargo clippy".to_string());
        if file_set.contains("src/main.rs") {
            entrypoints.insert("src/main.rs".to_string());
        }
        if file_set.contains("src/lib.rs") {
            entrypoints.insert("src/lib.rs".to_string());
        }
    }

    if file_set.contains("go.mod") {
        package_managers.insert("go".to_string());
        test_runners.insert("go test".to_string());
        common_commands.insert("go test ./...".to_string());
        for f in &files {
            if f.ends_with("main.go") {
                entrypoints.insert(f.clone());
            }
        }
    }

    if file_set.contains("pyproject.toml") {
        package_managers.insert(
            if file_set.contains("uv.lock") {
                "uv"
            } else {
                "python"
            }
            .to_string(),
        );
        let pyproject = read_rel(&root_path, "pyproject.toml").unwrap_or_default();
        if pyproject.contains("pytest") {
            test_runners.insert("pytest".to_string());
            common_commands.insert(
                if file_set.contains("uv.lock") {
                    "uv run pytest"
                } else {
                    "pytest"
                }
                .to_string(),
            );
        }
        if pyproject.contains("pytest-django") || pyproject.contains("DJANGO_SETTINGS_MODULE") {
            frameworks.insert("Django".to_string());
        }
        for p in parse_pytest_testpaths(&pyproject) {
            test_paths.insert(p);
        }
        for task in parse_poe_tasks(&pyproject).into_iter().take(12) {
            common_commands.insert(format!("uv run poe {task}"));
        }
    }

    if file_set.contains("package.json") {
        let package_json = read_rel(&root_path, "package.json").unwrap_or_default();
        if file_set.contains("pnpm-lock.yaml") {
            package_managers.insert("pnpm".to_string());
        } else if file_set.contains("yarn.lock") {
            package_managers.insert("yarn".to_string());
        } else if file_set.contains("bun.lockb") {
            package_managers.insert("bun".to_string());
        } else {
            package_managers.insert("npm".to_string());
        }
        if package_json.contains("vitest") {
            test_runners.insert("vitest".to_string());
        } else if package_json.contains("jest") {
            test_runners.insert("jest".to_string());
        }
        if package_json.contains("next") {
            frameworks.insert("Next.js".to_string());
        }
        if package_json.contains("react") {
            frameworks.insert("React".to_string());
        }
        for script in parse_package_scripts(&package_json).into_iter().take(12) {
            common_commands.insert(script);
        }
    }

    // If the project declares test paths (for example pytest `testpaths`), keep
    // this field as configured discovery scope. Otherwise fall back to observed
    // test directories so lightweight projects still get useful output.
    if test_paths.is_empty() {
        for f in &files {
            if is_test_file(f)
                && let Some(dir) = Path::new(f).parent().and_then(|p| p.to_str())
            {
                test_paths.insert(dir.to_string());
            }
        }
    }

    instruction_files.sort();
    instruction_files.truncate(20);

    Ok(ProjectResult {
        root: root_path.display().to_string(),
        languages: languages.into_iter().collect(),
        frameworks: frameworks.into_iter().collect(),
        package_managers: package_managers.into_iter().collect(),
        test_runners: test_runners.into_iter().collect(),
        test_paths: test_paths.into_iter().collect(),
        common_commands: common_commands.into_iter().collect(),
        entrypoints: entrypoints.into_iter().collect(),
        instruction_files,
    })
}

pub fn collect_files(root: &Path) -> Vec<String> {
    WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .sort_by_file_name(|a, b| a.cmp(b))
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter_map(|e| {
            e.path()
                .strip_prefix(root)
                .ok()
                .and_then(|p| p.to_str())
                .map(|s| s.replace('\\', "/"))
        })
        .collect()
}

pub fn is_test_file(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    name == "tests.py"
        || name.starts_with("test_")
        || name.ends_with("_test.py")
        || name.ends_with("_test.go")
        || name.ends_with(".test.ts")
        || name.ends_with(".test.tsx")
        || name.ends_with(".spec.ts")
        || name.ends_with(".spec.tsx")
        || path.contains("/tests/")
}

fn read_rel(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

pub fn parse_pytest_testpaths(pyproject: &str) -> Vec<String> {
    let mut out = Vec::new();
    let re = Regex::new(r#"testpaths\s*=\s*\[(?s:.*?)\]"#).unwrap();
    let q = Regex::new(r#"[\"']([^\"']+)[\"']"#).unwrap();
    if let Some(m) = re.find(pyproject) {
        for cap in q.captures_iter(m.as_str()) {
            out.push(cap[1].to_string());
        }
    }
    out
}

fn parse_poe_tasks(pyproject: &str) -> Vec<String> {
    let mut tasks = Vec::new();
    let mut in_tasks = false;
    let re = Regex::new(r#"^([A-Za-z0-9_-]+)\s*="#).unwrap();
    for line in pyproject.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[tool.poe.tasks]") {
            in_tasks = true;
            continue;
        }
        if in_tasks && trimmed.starts_with('[') {
            break;
        }
        if in_tasks && let Some(cap) = re.captures(trimmed) {
            tasks.push(cap[1].to_string());
        }
    }
    tasks
}

fn parse_package_scripts(package_json: &str) -> Vec<String> {
    let mut scripts = Vec::new();
    let value: serde_json::Value = match serde_json::from_str(package_json) {
        Ok(v) => v,
        Err(_) => return scripts,
    };
    if let Some(obj) = value.get("scripts").and_then(|s| s.as_object()) {
        for key in obj.keys() {
            if key.contains("test") || key.contains("lint") || key.contains("typecheck") {
                scripts.push(format!("npm run {key}"));
            }
        }
    }
    scripts
}

pub fn package_dir(path: &str) -> String {
    PathBuf::from(path)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".")
        .to_string()
}
