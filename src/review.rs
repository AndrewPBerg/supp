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
