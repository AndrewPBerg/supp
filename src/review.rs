use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::docs_search;
use crate::git::{self, DiffOptions};
use crate::project;
use crate::tests_for;

#[derive(Debug, Clone, Serialize)]
pub struct ReviewFileSummary {
    pub path: String,
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Serialize)]
pub struct ReviewResult {
    pub label: String,
    pub files: Vec<ReviewFileSummary>,
    pub likely_test_files: Vec<String>,
    pub focused_commands: Vec<String>,
    pub warnings: Vec<String>,
    pub docs_mentions: Vec<docs_search::DocsMatch>,
    pub patch: String,
}

pub fn analyze(
    root: &str,
    opts: DiffOptions,
    regex: Option<&str>,
    pagerank_iters: usize,
) -> anyhow::Result<ReviewResult> {
    let diff = git::get_diff(root, opts, regex)?;
    let tests = tests_for::analyze(root, None, true, pagerank_iters)?;

    let mut files = Vec::new();
    let mut query_terms = BTreeSet::new();
    let project_root = project::discover_root(root)?;

    for file in &diff.files {
        files.push(ReviewFileSummary {
            path: file.path.clone(),
            status: format!("{:?}", file.status),
            additions: file.additions,
            deletions: file.deletions,
        });

        add_path_terms(&file.path, &mut query_terms);
        if let Some(old) = &file.old_path {
            add_path_terms(old, &mut query_terms);
        }
    }

    let query: Vec<String> = query_terms.into_iter().take(12).collect();
    let mut docs = if query.is_empty() {
        docs_search::DocsResult {
            query: String::new(),
            matches: Vec::new(),
            files_scanned: 0,
        }
    } else {
        docs_search::search(project_root.to_str().unwrap_or(root), &query, 20)?
    };
    // Review packets should include docs/comments only when they are likely relevant.
    // A single generic path token (README, main, test, etc.) creates noise.
    docs.matches.retain(|m| m.score >= 2);
    docs.matches.truncate(8);

    let mut warnings = tests.warnings.clone();
    if diff.files.is_empty() {
        warnings.push("No changed files found for review.".to_string());
    }
    if !diff.files.is_empty() && tests.focused_commands.is_empty() {
        warnings.push("No focused validation command found for changed files.".to_string());
    }

    let patch = build_review_packet(&diff, &tests, &docs);

    Ok(ReviewResult {
        label: diff.label,
        files,
        likely_test_files: tests.likely_test_files,
        focused_commands: tests.focused_commands,
        warnings,
        docs_mentions: docs.matches,
        patch,
    })
}

fn add_path_terms(path: &str, out: &mut BTreeSet<String>) {
    for component in Path::new(path).components() {
        let part = component.as_os_str().to_string_lossy();
        for token in part
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|t| t.len() >= 4)
        {
            if !matches!(token, "test" | "tests" | "src" | "docs") {
                out.insert(token.to_lowercase());
            }
        }
    }
}

fn build_review_packet(
    diff: &git::DiffResult,
    tests: &tests_for::TestsForResult,
    docs: &docs_search::DocsResult,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("supp review  {}\n", diff.label));
    out.push_str("---\n\n");

    out.push_str("Review focus\n");
    out.push_str("- correctness and regressions\n");
    out.push_str("- missing or stale tests\n");
    out.push_str("- docs/config/contract drift\n");
    out.push_str("- risky dependency or API changes\n\n");

    out.push_str("Changed files\n");
    if diff.files.is_empty() {
        out.push_str("- none\n");
    } else {
        for file in &diff.files {
            out.push_str(&format!(
                "- {} {:?} +{} -{}\n",
                file.path, file.status, file.additions, file.deletions
            ));
        }
    }
    out.push('\n');

    if !tests.likely_test_files.is_empty() {
        out.push_str("Likely related tests\n");
        for file in &tests.likely_test_files {
            out.push_str(&format!("- {file}\n"));
        }
        out.push('\n');
    }

    if !tests.focused_commands.is_empty() {
        out.push_str("Focused validation commands\n");
        for command in &tests.focused_commands {
            out.push_str(&format!("- {command}\n"));
        }
        out.push('\n');
    }

    if !tests.warnings.is_empty() {
        out.push_str("Validation warnings\n");
        for warning in &tests.warnings {
            out.push_str(&format!("- {warning}\n"));
        }
        out.push('\n');
    }

    if !docs.matches.is_empty() {
        out.push_str("Potential docs/comments context\n");
        for item in docs.matches.iter().take(8) {
            out.push_str(&format!(
                "- {}:{} [{}] {}\n",
                item.file, item.line, item.kind, item.text
            ));
        }
        out.push('\n');
    }

    out.push_str("Patch\n");
    out.push_str("---\n");
    out.push_str(&diff.text);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{DeltaStatus, DiffResult, FileEntry};
    use tempfile::tempdir;

    fn write(root: &std::path::Path, path: &str, content: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn git(root: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn add_path_terms_keeps_meaningful_tokens() {
        let mut terms = BTreeSet::new();
        add_path_terms("src/docs_search-review_test.rs", &mut terms);
        assert!(terms.contains("search"));
        assert!(terms.contains("review"));
        assert!(!terms.contains("src"));
        assert!(!terms.contains("test"));
    }

    #[test]
    fn build_review_packet_includes_sections() {
        let diff = DiffResult {
            label: "Tracked changes".into(),
            files: vec![FileEntry {
                path: "src/lib.rs".into(),
                old_path: None,
                status: DeltaStatus::Modified,
                additions: 2,
                deletions: 1,
                patch: "patch".into(),
            }],
            text: "diff --git a/src/lib.rs b/src/lib.rs\n".into(),
            has_conflicts: false,
            is_branch_comparison: false,
            commit_count: None,
            stale_check: None,
        };
        let tests = tests_for::TestsForResult {
            target: "--diff".into(),
            resolved_target: Some("src/lib.rs".into()),
            likely_test_files: vec!["src/lib.rs".into()],
            focused_commands: vec!["cargo test lib".into()],
            warnings: vec!["warning".into()],
        };
        let docs = docs_search::DocsResult {
            query: "lib".into(),
            matches: vec![docs_search::DocsMatch {
                file: "README.md".into(),
                line: 1,
                kind: "doc".into(),
                text: "Review docs".into(),
                score: 2,
            }],
            files_scanned: 1,
        };

        let packet = build_review_packet(&diff, &tests, &docs);
        assert!(packet.contains("Review focus"));
        assert!(packet.contains("Likely related tests"));
        assert!(packet.contains("Focused validation commands"));
        assert!(packet.contains("Potential docs/comments context"));
        assert!(packet.contains("diff --git"));
    }

    #[test]
    fn analyze_reports_empty_git_diff_warning() {
        let dir = tempdir().unwrap();
        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.email", "a@example.com"]);
        git(dir.path(), &["config", "user.name", "A"]);
        write(
            dir.path(),
            "Cargo.toml",
            "[package]\nname='x'\nversion='0.1.0'\nedition='2024'\n",
        );
        write(dir.path(), "src/main.rs", "fn main() {}\n");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "init"]);

        let result = analyze(
            dir.path().to_str().unwrap(),
            DiffOptions {
                all: true,
                ..DiffOptions::default()
            },
            None,
            1,
        )
        .unwrap();
        assert!(result.files.is_empty());
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("No changed files"))
        );
    }

    #[test]
    fn analyze_includes_changed_file_and_commands() {
        let dir = tempdir().unwrap();
        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.email", "a@example.com"]);
        git(dir.path(), &["config", "user.name", "A"]);
        write(
            dir.path(),
            "Cargo.toml",
            "[package]\nname='x'\nversion='0.1.0'\nedition='2024'\n",
        );
        write(
            dir.path(),
            "src/lib.rs",
            "pub fn thing() -> bool { true }\n",
        );
        write(dir.path(), "README.md", "# lib review thing\n");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "init"]);
        write(
            dir.path(),
            "src/lib.rs",
            "pub fn thing() -> bool { false }\n",
        );

        let result = analyze(
            dir.path().to_str().unwrap(),
            DiffOptions {
                all: true,
                ..DiffOptions::default()
            },
            None,
            1,
        )
        .unwrap();
        assert_eq!(result.files[0].path, "src/lib.rs");
        assert!(result.focused_commands.contains(&"cargo test".to_string()));
        assert!(result.patch.contains("src/lib.rs"));
    }
}
