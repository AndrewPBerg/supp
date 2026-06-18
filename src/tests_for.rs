use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::project;
use crate::symbol;

#[derive(Debug, Serialize, Clone)]
pub struct TestsForResult {
    pub target: String,
    pub resolved_target: Option<String>,
    pub likely_test_files: Vec<String>,
    pub focused_commands: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn analyze(
    root: &str,
    target: Option<&str>,
    diff: bool,
    pagerank_iters: usize,
) -> anyhow::Result<TestsForResult> {
    let root_path = project::discover_root(root)?;
    let files = project::collect_files(&root_path);
    let project_info = project::analyze(root)?;

    let mut targets = Vec::new();
    let label = if diff {
        "--diff".to_string()
    } else {
        target.unwrap_or(".").to_string()
    };

    if diff {
        targets.extend(changed_files(&root_path)?);
    } else if let Some(t) = target {
        let normalized = project::normalize_target(&root_path, t);
        if root_path.join(&normalized).exists() || Path::new(&normalized).extension().is_some() {
            targets.push(normalized);
        } else {
            let query = vec![t.to_string()];
            if let Ok(result) = symbol::search(root, &query, pagerank_iters)
                && let Some((sym, _)) = result.matches.first()
            {
                targets.push(sym.file.clone());
            }
        }
    }

    targets.sort();
    targets.dedup();

    let mut tests = BTreeSet::new();
    let mut commands = BTreeSet::new();
    let mut warnings = BTreeSet::new();

    for t in &targets {
        for f in likely_tests_for_file(&root_path, t, &files) {
            tests.insert(f);
        }
        for cmd in validation_commands_for_file(&root_path, t, &project_info) {
            commands.insert(cmd);
        }
    }

    if targets.is_empty() {
        warnings
            .insert("No target files resolved; try a concrete path or symbol name.".to_string());
    }

    for test in &tests {
        if !project_info.test_paths.is_empty() && is_python_test(test) {
            let included = project_info
                .test_paths
                .iter()
                .any(|p| test == p || test.starts_with(&format!("{}/", p.trim_end_matches('/'))));
            if !included {
                warnings.insert(format!("{test} is outside configured pytest testpaths; run it explicitly or update testpaths if it should be in default discovery."));
            }
        }
    }

    if tests.is_empty() && !targets.is_empty() {
        warnings.insert("No direct test file found by naming/path heuristics; use the focused command as a starting point.".to_string());
    }

    Ok(TestsForResult {
        target: label,
        resolved_target: targets.first().cloned(),
        likely_test_files: tests.into_iter().collect(),
        focused_commands: commands.into_iter().collect(),
        warnings: warnings.into_iter().collect(),
    })
}

fn likely_tests_for_file(root: &Path, target: &str, files: &[String]) -> Vec<String> {
    if project::is_test_file(target) || has_inline_rust_tests(root, target) {
        return vec![target.to_string()];
    }

    let path = Path::new(target);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let dir = path.parent().and_then(|p| p.to_str()).unwrap_or("");
    let mut out = Vec::new();

    for f in files {
        if !project::is_test_file(f) {
            continue;
        }
        let fname = Path::new(f)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if fname.contains(stem)
            || fname == format!("test_{stem}.py")
            || fname == format!("{stem}_test.py")
            || fname == format!("{stem}_test.go")
            || fname == format!("{stem}.test.ts")
            || fname == format!("{stem}.test.tsx")
            || (!dir.is_empty() && f.contains(dir))
        {
            out.push(f.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

fn validation_commands_for_file(
    root: &Path,
    target: &str,
    project_info: &project::ProjectResult,
) -> Vec<String> {
    let mut commands = Vec::new();
    let dir = project::package_dir(target);
    if target.ends_with(".rs") || project_info.package_managers.iter().any(|p| p == "cargo") {
        if has_inline_rust_tests(root, target) {
            if let Some(stem) = Path::new(target).file_stem().and_then(|s| s.to_str()) {
                commands.push(format!("cargo test {stem}"));
            } else {
                commands.push("cargo test".to_string());
            }
        } else {
            commands.push("cargo test".to_string());
        }
    }
    if target.ends_with(".go") || project_info.package_managers.iter().any(|p| p == "go") {
        commands.push(format!(
            "go test ./{}",
            if dir == "." { "..." } else { dir.as_str() }
        ));
    }
    if target.ends_with(".py") || project_info.test_runners.iter().any(|p| p == "pytest") {
        let prefix = if project_info.package_managers.iter().any(|p| p == "uv") {
            "uv run "
        } else {
            ""
        };
        if project::is_test_file(target) {
            commands.push(format!("{prefix}pytest {target}"));
        } else {
            commands.push(format!("{prefix}pytest {dir}"));
        }
    }
    if target.ends_with(".ts")
        || target.ends_with(".tsx")
        || target.ends_with(".js")
        || target.ends_with(".jsx")
    {
        let runner = if project_info.test_runners.iter().any(|r| r == "vitest") {
            "vitest"
        } else {
            "test"
        };
        let pm = project_info
            .package_managers
            .first()
            .map(|s| s.as_str())
            .unwrap_or("npm");
        commands.push(match pm {
            "pnpm" => format!("pnpm {runner} {target}"),
            "yarn" => format!("yarn {runner} {target}"),
            "bun" => format!("bun {runner} {target}"),
            _ => format!("npm run {runner} -- {target}"),
        });
    }
    commands.sort();
    commands.dedup();
    commands
}

fn has_inline_rust_tests(root: &Path, target: &str) -> bool {
    if !target.ends_with(".rs") {
        return false;
    }
    let Ok(content) = std::fs::read_to_string(root.join(target)) else {
        return false;
    };
    content.contains("#[cfg(test)]") || content.contains("#[test]")
}

fn changed_files(root: &Path) -> anyhow::Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(|line| line.get(3..))
        .map(|s| s.split(" -> ").last().unwrap_or(s).to_string())
        .collect())
}

fn is_python_test(path: &str) -> bool {
    path.ends_with(".py") && project::is_test_file(path)
}

#[allow(dead_code)]
fn _pathbuf(s: &str) -> PathBuf {
    PathBuf::from(s)
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

    fn project_info() -> project::ProjectResult {
        project::ProjectResult {
            root: String::new(),
            languages: vec![],
            frameworks: vec![],
            package_managers: vec!["cargo".into(), "uv".into(), "pnpm".into()],
            test_runners: vec!["pytest".into(), "vitest".into()],
            test_paths: vec!["tests".into()],
            common_commands: vec![],
            entrypoints: vec![],
            instruction_files: vec![],
        }
    }

    #[test]
    fn likely_tests_match_by_name_directory_and_inline_rust() {
        let dir = tempdir().unwrap();
        write(dir.path(), "src/parser.rs", "#[cfg(test)]\nmod tests {}\n");
        let files = vec![
            "src/parser.rs".to_string(),
            "tests/parser_test.py".to_string(),
            "src/parser.test.ts".to_string(),
            "other/unrelated.py".to_string(),
        ];

        assert_eq!(
            likely_tests_for_file(dir.path(), "src/parser.rs", &files),
            vec!["src/parser.rs"]
        );
        let tests = likely_tests_for_file(dir.path(), "src/view.ts", &files);
        assert!(tests.contains(&"src/parser.test.ts".to_string()));
        assert!(
            likely_tests_for_file(dir.path(), "tests/parser_test.py", &files)
                .contains(&"tests/parser_test.py".to_string())
        );
    }

    #[test]
    fn validation_commands_cover_languages_and_package_managers() {
        let dir = tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "#[test]\nfn it_works() {}\n");
        let info = project_info();

        assert!(
            validation_commands_for_file(dir.path(), "src/lib.rs", &info)
                .contains(&"cargo test lib".to_string())
        );
        assert!(
            validation_commands_for_file(dir.path(), "pkg/main.go", &info)
                .contains(&"go test ./pkg".to_string())
        );
        assert!(
            validation_commands_for_file(dir.path(), "tests/test_app.py", &info)
                .contains(&"uv run pytest tests/test_app.py".to_string())
        );
        let mut ts_info = info.clone();
        ts_info.package_managers = vec!["pnpm".to_string()];
        assert!(
            validation_commands_for_file(dir.path(), "src/app.tsx", &ts_info)
                .contains(&"pnpm vitest src/app.tsx".to_string())
        );
    }

    #[test]
    fn analyze_resolves_path_and_warns_for_tests_outside_pytest_paths() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "pyproject.toml",
            "[project]\ndependencies=['pytest']\n[tool.pytest.ini_options]\ntestpaths = ['tests']\n",
        );
        write(dir.path(), "src/api.py", "def api(): pass\n");
        write(dir.path(), "other/test_api.py", "def test_api(): pass\n");

        let result = analyze(dir.path().to_str().unwrap(), Some("src/api.py"), false, 5).unwrap();
        assert_eq!(result.resolved_target.as_deref(), Some("src/api.py"));
        assert!(
            result
                .likely_test_files
                .contains(&"other/test_api.py".to_string())
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("outside configured pytest testpaths"))
        );
    }

    #[test]
    fn analyze_warns_when_target_cannot_resolve() {
        let dir = tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[package]\nname='x'\n");
        let result = analyze(
            dir.path().to_str().unwrap(),
            Some("missing_symbol"),
            false,
            1,
        )
        .unwrap();
        assert!(result.resolved_target.is_none());
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("No target files resolved"))
        );
    }

    #[test]
    fn helpers_detect_inline_tests_and_python_tests() {
        let dir = tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "#[test]\nfn t() {}\n");
        write(dir.path(), "src/main.rs", "fn main() {}\n");
        assert!(has_inline_rust_tests(dir.path(), "src/lib.rs"));
        assert!(!has_inline_rust_tests(dir.path(), "src/main.rs"));
        assert!(!has_inline_rust_tests(dir.path(), "src/app.py"));
        assert!(is_python_test("tests/test_app.py"));
        assert!(!is_python_test("tests/app.rs"));
    }
}
