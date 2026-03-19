use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use ignore::WalkBuilder;
use regex::Regex;

use crate::tree;

pub struct ContextResult {
    pub plain: String,
    pub file_count: usize,
    pub total_bytes: usize,
    pub total_lines: usize,
}

pub fn generate_context(paths: &[String], depth: usize, regex: Option<&str>) -> Result<ContextResult> {
    let re = regex.map(Regex::new).transpose()?;

    // Collect all file paths to read
    let mut file_paths: Vec<String> = Vec::new();
    let mut dir_paths: Vec<String> = Vec::new();
    let mut individual_files: Vec<String> = Vec::new();

    for p in paths {
        let path = Path::new(p);
        if !path.exists() {
            bail!("path does not exist: {}", p);
        }
        if path.is_file() {
            if let Some(ref re) = re {
                if !re.is_match(p) {
                    continue;
                }
            }
            file_paths.push(p.clone());
            individual_files.push(p.clone());
        } else if path.is_dir() {
            dir_paths.push(p.clone());
            let walker = WalkBuilder::new(path)
                .sort_by_file_name(|a, b| a.cmp(b))
                .build();
            for entry in walker.flatten() {
                if entry.path().is_file() {
                    let rel = entry.path().to_string_lossy().to_string();
                    if let Some(ref re) = re {
                        if !re.is_match(&rel) {
                            continue;
                        }
                    }
                    file_paths.push(rel);
                }
            }
        }
    }

    if file_paths.is_empty() {
        bail!("no files matched the given paths/filters");
    }

    // Concurrent reads
    let results: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    std::thread::scope(|s| {
        for fp in &file_paths {
            let results = Arc::clone(&results);
            let fp = fp.clone();
            s.spawn(move || {
                if let Ok(content) = std::fs::read_to_string(&fp) {
                    results.lock().unwrap().push((fp, content));
                }
            });
        }
    });

    // Sort by original order
    let mut read_files = Arc::try_unwrap(results).unwrap().into_inner().unwrap();
    let order: std::collections::HashMap<&str, usize> = file_paths
        .iter()
        .enumerate()
        .map(|(i, p)| (p.as_str(), i))
        .collect();
    read_files.sort_by_key(|(p, _)| order.get(p.as_str()).copied().unwrap_or(usize::MAX));

    let file_count = read_files.len();
    let total_bytes: usize = read_files.iter().map(|(_, c)| c.len()).sum();
    let total_lines: usize = read_files.iter().map(|(_, c)| c.lines().count()).sum();

    // Assemble plain output
    let mut plain = String::new();
    plain.push_str("CONTEXT FOR LLM\n");
    plain.push_str("================================\n");

    // Directory/file listing header
    for dir in &dir_paths {
        plain.push_str(&format!("Directory: {}\n", dir));
        if let Ok(tree_result) = tree::build_tree(dir, Some(depth), regex, None) {
            plain.push_str(&tree_result.plain);
            if !tree_result.plain.ends_with('\n') {
                plain.push('\n');
            }
        }
    }
    if !individual_files.is_empty() {
        plain.push_str("Files:\n");
        for f in &individual_files {
            plain.push_str(&format!("  {}\n", f));
        }
    }

    plain.push_str("\n--- FILE CONTENTS ---\n");
    plain.push_str("<documents>\n");

    for (i, (path, content)) in read_files.iter().enumerate() {
        plain.push_str(&format!("<document index=\"{}\">\n", i + 1));
        plain.push_str(&format!("<source>{}</source>\n", path));
        plain.push_str("<document_content>\n");

        let lines: Vec<&str> = content.lines().collect();
        let width = if lines.is_empty() { 1 } else { lines.len().to_string().len() };
        for (line_num, line) in lines.iter().enumerate() {
            plain.push_str(&format!("{:>width$}  {}\n", line_num + 1, line, width = width));
        }

        plain.push_str("</document_content>\n");
        plain.push_str("</document>\n");
    }

    plain.push_str("</documents>\n");

    Ok(ContextResult {
        plain,
        file_count,
        total_bytes,
        total_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, content).unwrap();
        }
        dir
    }

    #[test]
    fn single_file_context() {
        let dir = setup(&[("hello.txt", "hello world")]);
        let file = dir.path().join("hello.txt");
        let result = generate_context(
            &[file.to_string_lossy().to_string()],
            2,
            None,
        ).unwrap();
        assert_eq!(result.file_count, 1);
        assert!(result.plain.contains("1  hello world"));
        assert!(result.plain.contains("CONTEXT FOR LLM"));
    }

    #[test]
    fn directory_reads_all_files() {
        let dir = setup(&[("a.txt", "aaa"), ("b.txt", "bbb")]);
        let result = generate_context(
            &[dir.path().to_string_lossy().to_string()],
            2,
            None,
        ).unwrap();
        assert_eq!(result.file_count, 2);
        assert!(result.plain.contains("aaa"));
        assert!(result.plain.contains("bbb"));
    }

    #[test]
    fn regex_filters_files() {
        let dir = setup(&[("main.rs", "fn main()"), ("readme.md", "# Readme")]);
        let result = generate_context(
            &[dir.path().to_string_lossy().to_string()],
            2,
            Some(r"\.rs$"),
        ).unwrap();
        assert_eq!(result.file_count, 1);
        assert!(result.plain.contains("fn main()"));
        assert!(!result.plain.contains("# Readme"));
    }

    #[test]
    fn nonexistent_path_errors() {
        let result = generate_context(&["/tmp/nonexistent_supp_test_path".to_string()], 2, None);
        assert!(result.is_err());
    }

    #[test]
    fn no_matching_files_errors() {
        let dir = setup(&[("readme.md", "hi")]);
        let result = generate_context(
            &[dir.path().to_string_lossy().to_string()],
            2,
            Some(r"\.rs$"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn output_contains_metadata() {
        let dir = setup(&[("test.txt", "content")]);
        let file = dir.path().join("test.txt");
        let result = generate_context(
            &[file.to_string_lossy().to_string()],
            2,
            None,
        ).unwrap();
        assert!(result.plain.contains("================================"));
        assert!(result.plain.contains("--- FILE CONTENTS ---"));
        assert!(result.plain.contains("<documents>"));
        assert!(result.plain.contains("<document_content>"));
    }

    #[test]
    fn total_bytes_correct() {
        let dir = setup(&[("a.txt", "12345"), ("b.txt", "67890")]);
        let result = generate_context(
            &[dir.path().to_string_lossy().to_string()],
            2,
            None,
        ).unwrap();
        assert_eq!(result.total_bytes, 10);
    }
}
