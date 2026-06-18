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
    let root_path = discover_root(root)?;
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

pub fn discover_root(start: &str) -> anyhow::Result<PathBuf> {
    let mut path = std::fs::canonicalize(start)?;
    if path.is_file() {
        path.pop();
    }

    let markers = [
        ".git",
        "Cargo.toml",
        "go.mod",
        "pyproject.toml",
        "package.json",
    ];
    let mut cur = Some(path.as_path());
    while let Some(dir) = cur {
        if markers.iter().any(|m| dir.join(m).exists()) {
            return Ok(dir.to_path_buf());
        }
        cur = dir.parent();
    }

    Ok(path)
}

pub fn normalize_target(root: &Path, target: &str) -> String {
    let direct = root.join(target);
    if direct.exists() {
        return target.replace('\\', "/");
    }

    if let Ok(abs) = std::fs::canonicalize(target)
        && let Ok(rel) = abs.strip_prefix(root)
        && let Some(s) = rel.to_str()
    {
        return s.replace('\\', "/");
    }

    target.replace('\\', "/")
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(root: &std::path::Path, path: &str, content: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn analyze_detects_rust_go_python_and_js_project_shape() {
        let dir = tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[package]\nname='x'\n");
        write(dir.path(), "src/main.rs", "fn main() {}\n");
        write(dir.path(), "go.mod", "module example.com/x\n");
        write(
            dir.path(),
            "cmd/app/main.go",
            "package main\nfunc main() {}\n",
        );
        write(
            dir.path(),
            "pyproject.toml",
            "[project]\ndependencies=['pytest','pytest-django']\n[tool.pytest.ini_options]\ntestpaths = ['tests', 'integration']\n[tool.poe.tasks]\nlint = 'ruff check .'\n",
        );
        write(
            dir.path(),
            "package.json",
            r#"{"scripts":{"test":"vitest","lint":"eslint .","typecheck":"tsc"},"dependencies":{"next":"latest","react":"latest","vitest":"latest"}}"#,
        );
        write(dir.path(), "uv.lock", "");
        write(dir.path(), "pnpm-lock.yaml", "");
        write(dir.path(), "README.md", "# readme\n");
        write(
            dir.path(),
            "src/app.tsx",
            "export default function App() {}\n",
        );
        write(dir.path(), "tests/test_app.py", "def test_x(): pass\n");

        let result = analyze(dir.path().to_str().unwrap()).unwrap();
        assert!(result.languages.contains(&"Rust".to_string()));
        assert!(result.languages.contains(&"Go".to_string()));
        assert!(result.languages.contains(&"TypeScript".to_string()));
        assert!(result.frameworks.contains(&"Django".to_string()));
        assert!(result.frameworks.contains(&"Next.js".to_string()));
        assert!(result.package_managers.contains(&"cargo".to_string()));
        assert!(result.package_managers.contains(&"uv".to_string()));
        assert!(result.package_managers.contains(&"pnpm".to_string()));
        assert!(
            result
                .common_commands
                .contains(&"uv run poe lint".to_string())
        );
        assert!(
            result
                .common_commands
                .contains(&"npm run typecheck".to_string())
        );
        assert_eq!(
            result.test_paths,
            vec!["integration".to_string(), "tests".to_string()]
        );
        assert!(result.entrypoints.contains(&"src/main.rs".to_string()));
        assert!(result.instruction_files.contains(&"README.md".to_string()));
    }

    #[test]
    fn parser_helpers_extract_config_bits() {
        assert_eq!(
            parse_pytest_testpaths("testpaths = ['a', \"b\"]"),
            vec!["a", "b"]
        );
        assert_eq!(
            parse_poe_tasks("[tool.poe.tasks]\ntest = 'pytest'\nlint = 'ruff'\n[tool.other]\nx=1"),
            vec!["test", "lint"]
        );
        assert_eq!(
            parse_package_scripts(r#"{"scripts":{"test":"x","lint":"x","build":"x"}}"#),
            vec!["npm run lint", "npm run test"]
        );
        assert!(parse_package_scripts("not json").is_empty());
    }

    #[test]
    fn path_helpers_normalize_and_classify_tests() {
        let dir = tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "");
        assert_eq!(normalize_target(dir.path(), "src/lib.rs"), "src/lib.rs");
        assert!(is_test_file("src/tests/foo.rs"));
        assert!(is_test_file("test_api.py"));
        assert!(is_test_file("api_test.go"));
        assert!(is_test_file("api.spec.tsx"));
        assert!(!is_test_file("src/lib.rs"));
        assert_eq!(package_dir("src/lib.rs"), "src");
        assert_eq!(package_dir("main.rs"), "");
    }

    #[test]
    fn discover_root_walks_up_to_marker() {
        let dir = tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[package]\nname='x'\n");
        write(dir.path(), "src/nested/file.rs", "");
        let root = discover_root(dir.path().join("src/nested/file.rs").to_str().unwrap()).unwrap();
        assert_eq!(root, dir.path());
    }
}
