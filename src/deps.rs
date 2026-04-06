use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::Result;
use colored::Colorize;
use serde::Serialize;

use crate::compress;
use crate::pick;
use crate::why;

// ── Result types ───────────────────────────────────────────────────

#[derive(Serialize)]
pub struct DepsResult {
    pub target: Option<String>,
    pub reverse: bool,
    pub edges: BTreeMap<String, BTreeSet<String>>,
    pub display: String,
    pub plain: String,
    pub dot: Option<String>,
    pub file_count: usize,
    pub edge_count: usize,
}

// ── Public API ─────────────────────────────────────────────────────

pub fn analyze(
    root: &str,
    target: Option<&str>,
    reverse: bool,
    depth: Option<usize>,
    dot: bool,
    regex: Option<&str>,
) -> Result<DepsResult> {
    let root_path = std::fs::canonicalize(root)?;

    // 1. Build the full file-level dependency graph
    let full_graph = build_file_graph(&root_path, regex)?;

    // 2. Extract relevant subgraph or use full graph
    let (edges, target_normalized) = if let Some(target_raw) = target {
        let normalized = normalize_target(target_raw, &root_path);
        if !full_graph.contains_key(&normalized) {
            // Target might be a dep but not a key — check values too
            let exists = full_graph.values().any(|deps| deps.contains(&normalized));
            if !exists {
                let candidates: Vec<String> = full_graph.keys().cloned().collect();
                let msg = crate::pick::error_with_suggestions(
                    &format!("file '{}' not found in dependency graph", target_raw),
                    target_raw,
                    &candidates,
                );
                anyhow::bail!("{}", msg);
            }
        }
        let sub = extract_subgraph(&full_graph, &normalized, reverse, depth);
        (sub, Some(normalized))
    } else {
        let edges = if let Some(max_depth) = depth {
            // Find root files (files that nothing depends on) and BFS from them
            let all_deps: HashSet<&String> = full_graph.values().flat_map(|d| d.iter()).collect();
            let roots: Vec<String> = full_graph
                .keys()
                .filter(|k| !all_deps.contains(k))
                .cloned()
                .collect();
            let mut combined = BTreeMap::new();
            for r in &roots {
                for (k, v) in extract_subgraph(&full_graph, r, false, Some(max_depth)) {
                    combined.entry(k).or_insert_with(BTreeSet::new).extend(v);
                }
            }
            combined
        } else {
            full_graph_to_btree(&full_graph)
        };
        (edges, None)
    };

    // 3. Compute stats
    let mut all_files: BTreeSet<&String> = BTreeSet::new();
    let mut edge_count = 0;
    for (k, vs) in &edges {
        all_files.insert(k);
        for v in vs {
            all_files.insert(v);
        }
        edge_count += vs.len();
    }
    let file_count = all_files.len();

    // 4. Generate output
    let dot_output = if dot {
        Some(render_dot(&edges, target_normalized.as_deref()))
    } else {
        None
    };

    let (display, plain) = if dot {
        let d = dot_output.as_ref().unwrap().clone();
        (d.clone(), d)
    } else {
        render_tree(&edges, target_normalized.as_deref(), reverse)
    };

    Ok(DepsResult {
        target: target_normalized,
        reverse,
        edges,
        display,
        plain,
        dot: dot_output,
        file_count,
        edge_count,
    })
}

// ── Graph building ─────────────────────────────────────────────────

fn build_file_graph(root: &Path, regex: Option<&str>) -> Result<HashMap<String, HashSet<String>>> {
    // collect_files returns paths relative to the given root, so pass "."
    // and resolve absolute paths via `root` for file I/O
    let all_files = pick::collect_files(".", regex)?;

    // Normalize paths: strip leading "./" for clean keys
    let source_files: Vec<String> = all_files
        .into_iter()
        .map(|f| f.strip_prefix("./").unwrap_or(&f).to_string())
        .filter(|f| compress::detect_lang(f).is_some())
        .collect();

    let project_files: HashSet<String> = source_files.iter().cloned().collect();

    let mut graph: HashMap<String, HashSet<String>> = HashMap::new();

    for file in &source_files {
        let abs_path = root.join(file);
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let imports = why::extract_file_imports(&content, file, root);

        // Collect unique modules
        let modules: HashSet<&String> = imports.values().collect();
        let mut deps: HashSet<String> = HashSet::new();

        for module in modules {
            if let Some(resolved) = resolve_module_to_file(module, file, root, &project_files)
                && resolved != *file
            {
                deps.insert(resolved);
            }
        }

        // For Rust files, also resolve `mod` declarations to file paths
        if matches!(compress::detect_lang(file), Some(compress::Lang::Rust)) {
            for mod_dep in resolve_rust_mod_decls(&content, file, &project_files) {
                deps.insert(mod_dep);
            }
        }

        graph.insert(file.clone(), deps);
    }

    Ok(graph)
}

// ── Module-to-file resolution ──────────────────────────────────────

fn resolve_module_to_file(
    module: &str,
    from_file: &str,
    root: &Path,
    project_files: &HashSet<String>,
) -> Option<String> {
    // 1. Direct file match (C/C++ local includes are already resolved to paths)
    if project_files.contains(module) {
        return Some(module.to_string());
    }

    // 2. Python relative imports
    if module.starts_with('.') {
        return why::resolve_relative_import(module, from_file, root);
    }

    // 3. JS/TS relative paths
    if module.starts_with("./") || module.starts_with("../") {
        return resolve_js_relative(module, from_file, root, project_files);
    }

    // 4. Rust crate/super paths
    if module.starts_with("crate::") || module.starts_with("super::") {
        return resolve_rust_module(module, from_file, root, project_files);
    }

    // 5. Python absolute imports (foo.bar → foo/bar.py)
    if module.contains('.') && !module.contains('/') && !module.starts_with('<') {
        return resolve_python_module(module, root, project_files);
    }

    None
}

fn resolve_js_relative(
    module: &str,
    from_file: &str,
    root: &Path,
    project_files: &HashSet<String>,
) -> Option<String> {
    let from_dir = Path::new(from_file).parent().unwrap_or(Path::new(""));
    let candidate = from_dir.join(module);
    let base = normalize_path(&candidate);

    // Try with various extensions
    let extensions = [".ts", ".tsx", ".js", ".jsx"];
    for ext in &extensions {
        let with_ext = format!("{}{}", base, ext);
        if project_files.contains(&with_ext) {
            return Some(with_ext);
        }
    }

    // Try as directory with index file
    for ext in &extensions {
        let index = format!("{}/index{}", base, ext);
        if project_files.contains(&index) {
            return Some(index);
        }
    }

    // Maybe it already has the extension
    if project_files.contains(&base) {
        return Some(base);
    }

    // Check if the file exists on disk (handles cases where module path maps directly)
    let abs = root.join(&base);
    if abs.exists() && project_files.contains(&base) {
        return Some(base);
    }

    None
}

fn resolve_rust_module(
    module: &str,
    from_file: &str,
    root: &Path,
    project_files: &HashSet<String>,
) -> Option<String> {
    if let Some(rest) = module.strip_prefix("crate::") {
        let parts: Vec<&str> = rest.split("::").collect();
        let path_str = parts.join("/");

        // Try src/<path>.rs
        let candidate = format!("src/{}.rs", path_str);
        if project_files.contains(&candidate) {
            return Some(candidate);
        }

        // Try src/<path>/mod.rs
        let candidate = format!("src/{}/mod.rs", path_str);
        if project_files.contains(&candidate) {
            return Some(candidate);
        }

        // Try just <path>.rs (for non-src layouts)
        let candidate = format!("{}.rs", path_str);
        if project_files.contains(&candidate) {
            return Some(candidate);
        }
    }

    if let Some(rest) = module.strip_prefix("super::") {
        let from_dir = Path::new(from_file).parent().unwrap_or(Path::new(""));
        let parent = from_dir.parent().unwrap_or(Path::new(""));
        let parts: Vec<&str> = rest.split("::").collect();
        let path_str = parts.join("/");

        // Try parent/<path>.rs
        let candidate = parent.join(format!("{}.rs", path_str));
        let candidate_str = candidate.to_string_lossy().to_string();
        if project_files.contains(&candidate_str) {
            return Some(candidate_str);
        }

        // Try parent/<path>/mod.rs
        let candidate = parent.join(format!("{}/mod.rs", path_str));
        let candidate_str = candidate.to_string_lossy().to_string();
        if project_files.contains(&candidate_str) {
            return Some(candidate_str);
        }
    }

    // Fallback: try resolving as a local module in the same directory
    // e.g., module = "crate::compress" and we have "src/compress/mod.rs"
    let _ = root; // already used above via project_files
    None
}

fn resolve_python_module(
    module: &str,
    root: &Path,
    project_files: &HashSet<String>,
) -> Option<String> {
    let parts: Vec<&str> = module.split('.').collect();
    let path_str = parts.join("/");

    // Try <path>.py
    let candidate = format!("{}.py", path_str);
    if project_files.contains(&candidate) {
        return Some(candidate);
    }

    // Try <path>/__init__.py
    let candidate = format!("{}/__init__.py", path_str);
    if project_files.contains(&candidate) {
        return Some(candidate);
    }

    let _ = root;
    None
}

/// Extract `mod foo;` declarations from Rust files and resolve to file paths.
fn resolve_rust_mod_decls(
    content: &str,
    from_file: &str,
    project_files: &HashSet<String>,
) -> Vec<String> {
    let mut results = Vec::new();
    let from_dir = Path::new(from_file).parent().unwrap_or(Path::new(""));

    // Check if this file is a mod.rs or lib.rs/main.rs (affects child module paths)
    let file_name = Path::new(from_file)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let is_mod_root = file_name == "mod.rs" || file_name == "lib.rs" || file_name == "main.rs";

    for line in content.lines() {
        let trimmed = line.trim();
        // Match `mod foo;` but not `mod foo {` (inline modules) or `// mod foo;` (comments)
        if let Some(rest) = trimmed.strip_prefix("mod ")
            && let Some(name) = rest.strip_suffix(';')
        {
            let name = name.trim();
            if name.is_empty() || name.contains(' ') {
                continue;
            }

            if is_mod_root {
                // mod.rs/main.rs/lib.rs: children are in sibling files/dirs
                // e.g., src/main.rs → mod cli → src/cli.rs or src/cli/mod.rs
                let candidate = from_dir.join(format!("{}.rs", name));
                let candidate_str = candidate.to_string_lossy().to_string();
                if project_files.contains(&candidate_str) {
                    results.push(candidate_str);
                    continue;
                }
                let candidate = from_dir.join(name).join("mod.rs");
                let candidate_str = candidate.to_string_lossy().to_string();
                if project_files.contains(&candidate_str) {
                    results.push(candidate_str);
                }
            } else {
                // Regular file (e.g., src/foo.rs → mod bar → src/foo/bar.rs)
                let stem = Path::new(from_file)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy();
                let candidate = from_dir.join(stem.as_ref()).join(format!("{}.rs", name));
                let candidate_str = candidate.to_string_lossy().to_string();
                if project_files.contains(&candidate_str) {
                    results.push(candidate_str);
                    continue;
                }
                let candidate = from_dir.join(stem.as_ref()).join(name).join("mod.rs");
                let candidate_str = candidate.to_string_lossy().to_string();
                if project_files.contains(&candidate_str) {
                    results.push(candidate_str);
                }
            }
        }
    }
    results
}

fn normalize_path(path: &Path) -> String {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::CurDir => {}
            std::path::Component::Normal(p) => parts.push(p),
            _ => {}
        }
    }
    parts
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_target(target: &str, root: &Path) -> String {
    // If target is an absolute path, make it relative to root
    let target_path = Path::new(target);
    if target_path.is_absolute()
        && let Ok(rel) = target_path.strip_prefix(root)
    {
        return rel.to_string_lossy().to_string();
    }
    // Strip leading "./"
    target.strip_prefix("./").unwrap_or(target).to_string()
}

// ── Graph operations ───────────────────────────────────────────────

fn invert_graph(graph: &HashMap<String, HashSet<String>>) -> HashMap<String, HashSet<String>> {
    let mut inverted: HashMap<String, HashSet<String>> = HashMap::new();
    for (file, deps) in graph {
        for dep in deps {
            inverted
                .entry(dep.clone())
                .or_default()
                .insert(file.clone());
        }
    }
    inverted
}

fn extract_subgraph(
    full_graph: &HashMap<String, HashSet<String>>,
    target: &str,
    reverse: bool,
    max_depth: Option<usize>,
) -> BTreeMap<String, BTreeSet<String>> {
    let effective_graph = if reverse {
        invert_graph(full_graph)
    } else {
        full_graph.clone()
    };

    let mut result: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    queue.push_back((target.to_string(), 0));
    visited.insert(target.to_string());

    while let Some((file, depth)) = queue.pop_front() {
        if let Some(max) = max_depth
            && depth >= max
        {
            continue;
        }

        if let Some(deps) = effective_graph.get(&file) {
            let sorted_deps: BTreeSet<String> = deps.iter().cloned().collect();
            for dep in &sorted_deps {
                if !visited.contains(dep) {
                    visited.insert(dep.clone());
                    queue.push_back((dep.clone(), depth + 1));
                }
            }
            if !sorted_deps.is_empty() {
                result.insert(file, sorted_deps);
            }
        }
    }

    result
}

fn full_graph_to_btree(
    graph: &HashMap<String, HashSet<String>>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut bt: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (k, vs) in graph {
        if !vs.is_empty() {
            bt.insert(k.clone(), vs.iter().cloned().collect());
        }
    }
    bt
}

// ── Tree rendering ─────────────────────────────────────────────────

fn render_tree(
    edges: &BTreeMap<String, BTreeSet<String>>,
    target: Option<&str>,
    reverse: bool,
) -> (String, String) {
    let mut display = String::new();
    let mut plain = String::new();

    if let Some(target_file) = target {
        // Targeted mode: rooted tree from the focus file
        render_rooted_tree(edges, target_file, reverse, &mut display, &mut plain);
    } else {
        // Whole-project mode: each file with its deps
        render_flat_graph(edges, &mut display, &mut plain);
    }

    (display, plain)
}

fn render_rooted_tree(
    edges: &BTreeMap<String, BTreeSet<String>>,
    root_file: &str,
    reverse: bool,
    display: &mut String,
    plain: &mut String,
) {
    let direction = if reverse {
        "dependents"
    } else {
        "dependencies"
    };
    let header_display = format!("{} ({}):", root_file.bold().cyan(), direction);
    let header_plain = format!("{} ({}):", root_file, direction);

    display.push_str(&header_display);
    display.push('\n');
    plain.push_str(&header_plain);
    plain.push('\n');

    let deps = edges.get(root_file);
    if let Some(deps) = deps {
        let deps_vec: Vec<&String> = deps.iter().collect();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(root_file.to_string());
        render_children(edges, &deps_vec, "", &mut visited, display, plain);
    }
}

fn render_children(
    edges: &BTreeMap<String, BTreeSet<String>>,
    children: &[&String],
    prefix: &str,
    visited: &mut HashSet<String>,
    display: &mut String,
    plain: &mut String,
) {
    for (i, child) in children.iter().enumerate() {
        let is_last = i == children.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last { "    " } else { "│   " };

        let line_display = format!("{}{}{}", prefix, connector, child.cyan());
        let line_plain = format!("{}{}{}", prefix, connector, child);

        display.push_str(&line_display);
        display.push('\n');
        plain.push_str(&line_plain);
        plain.push('\n');

        // Recurse if this child has deps and we haven't visited it yet
        if !visited.contains(*child) {
            visited.insert((*child).clone());
            if let Some(sub_deps) = edges.get(*child) {
                let sub_vec: Vec<&String> = sub_deps.iter().collect();
                let new_prefix = format!("{}{}", prefix, child_prefix);
                render_children(edges, &sub_vec, &new_prefix, visited, display, plain);
            }
        }
    }
}

fn render_flat_graph(
    edges: &BTreeMap<String, BTreeSet<String>>,
    display: &mut String,
    plain: &mut String,
) {
    for (file, deps) in edges {
        let deps_str: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
        let joined = deps_str.join(", ");

        let line_display = format!("{} {} {}", file.bold().cyan(), "→".dimmed(), joined,);
        let line_plain = format!("{} → {}", file, joined);

        display.push_str(&line_display);
        display.push('\n');
        plain.push_str(&line_plain);
        plain.push('\n');
    }
}

// ── DOT rendering ──────────────────────────────────────────────────

fn render_dot(edges: &BTreeMap<String, BTreeSet<String>>, target: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("digraph deps {\n");
    out.push_str("  rankdir=LR;\n");
    out.push_str("  node [shape=box, fontname=\"monospace\", fontsize=10];\n");

    if let Some(t) = target {
        out.push_str(&format!(
            "  \"{}\" [style=filled, fillcolor=lightyellow];\n",
            t
        ));
    }

    for (file, deps) in edges {
        for dep in deps {
            out.push_str(&format!("  \"{}\" -> \"{}\";\n", file, dep));
        }
    }

    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalize_target ───────────────────────────────────────────

    #[test]
    fn normalize_strips_dot_slash() {
        let root = Path::new("/tmp");
        assert_eq!(normalize_target("./src/main.rs", root), "src/main.rs");
    }

    #[test]
    fn normalize_keeps_plain() {
        let root = Path::new("/tmp");
        assert_eq!(normalize_target("src/main.rs", root), "src/main.rs");
    }

    // ── invert_graph ───────────────────────────────────────────────

    #[test]
    fn invert_simple() {
        let mut g: HashMap<String, HashSet<String>> = HashMap::new();
        g.entry("a".into()).or_default().insert("b".into());
        g.entry("a".into()).or_default().insert("c".into());

        let inv = invert_graph(&g);
        assert!(inv.get("b").unwrap().contains("a"));
        assert!(inv.get("c").unwrap().contains("a"));
        assert!(!inv.contains_key("a"));
    }

    // ── extract_subgraph ───────────────────────────────────────────

    #[test]
    fn subgraph_forward() {
        let mut g: HashMap<String, HashSet<String>> = HashMap::new();
        g.entry("a".into()).or_default().insert("b".into());
        g.entry("b".into()).or_default().insert("c".into());
        g.entry("c".into()).or_default(); // leaf

        let sub = extract_subgraph(&g, "a", false, None);
        assert!(sub.contains_key("a"));
        assert!(sub.contains_key("b"));
        assert!(sub.get("a").unwrap().contains("b"));
        assert!(sub.get("b").unwrap().contains("c"));
    }

    #[test]
    fn subgraph_depth_limited() {
        let mut g: HashMap<String, HashSet<String>> = HashMap::new();
        g.entry("a".into()).or_default().insert("b".into());
        g.entry("b".into()).or_default().insert("c".into());

        let sub = extract_subgraph(&g, "a", false, Some(1));
        assert!(sub.contains_key("a"));
        assert!(!sub.contains_key("b")); // depth 1 reached, don't expand b
    }

    #[test]
    fn subgraph_reverse() {
        let mut g: HashMap<String, HashSet<String>> = HashMap::new();
        g.entry("a".into()).or_default().insert("b".into());
        g.entry("c".into()).or_default().insert("b".into());

        let sub = extract_subgraph(&g, "b", true, None);
        // b's dependents are a and c
        assert!(sub.get("b").unwrap().contains("a"));
        assert!(sub.get("b").unwrap().contains("c"));
    }

    #[test]
    fn subgraph_handles_cycles() {
        let mut g: HashMap<String, HashSet<String>> = HashMap::new();
        g.entry("a".into()).or_default().insert("b".into());
        g.entry("b".into()).or_default().insert("a".into());

        let sub = extract_subgraph(&g, "a", false, None);
        // Should not infinite loop
        assert!(sub.contains_key("a"));
        assert!(sub.contains_key("b"));
    }

    // ── full_graph_to_btree ────────────────────────────────────────

    #[test]
    fn full_graph_skips_empty() {
        let mut g: HashMap<String, HashSet<String>> = HashMap::new();
        g.insert("a".into(), HashSet::new()); // no deps
        g.entry("b".into()).or_default().insert("c".into());

        let bt = full_graph_to_btree(&g);
        assert!(!bt.contains_key("a")); // empty deps skipped
        assert!(bt.contains_key("b"));
    }

    // ── render_dot ─────────────────────────────────────────────────

    #[test]
    fn dot_basic() {
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        edges
            .entry("a.rs".into())
            .or_default()
            .insert("b.rs".into());

        let dot = render_dot(&edges, None);
        assert!(dot.contains("digraph deps"));
        assert!(dot.contains("\"a.rs\" -> \"b.rs\""));
    }

    #[test]
    fn dot_with_target_highlights() {
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        edges
            .entry("a.rs".into())
            .or_default()
            .insert("b.rs".into());

        let dot = render_dot(&edges, Some("a.rs"));
        assert!(dot.contains("\"a.rs\" [style=filled"));
    }

    // ── render_tree ────────────────────────────────────────────────

    #[test]
    fn tree_flat_mode() {
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        edges
            .entry("a.rs".into())
            .or_default()
            .insert("b.rs".into());

        let (_, plain) = render_tree(&edges, None, false);
        assert!(plain.contains("a.rs"));
        assert!(plain.contains("→"));
        assert!(plain.contains("b.rs"));
    }

    #[test]
    fn tree_rooted_mode() {
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        edges
            .entry("a.rs".into())
            .or_default()
            .insert("b.rs".into());

        let (_, plain) = render_tree(&edges, Some("a.rs"), false);
        assert!(plain.contains("a.rs (dependencies):"));
        assert!(plain.contains("└── b.rs"));
    }

    // ── resolve helpers ────────────────────────────────────────────

    #[test]
    fn resolve_js_relative_with_ext() {
        let mut files = HashSet::new();
        files.insert("src/utils.ts".to_string());

        let result = resolve_js_relative("./utils", "src/main.ts", Path::new("."), &files);
        assert_eq!(result, Some("src/utils.ts".to_string()));
    }

    #[test]
    fn resolve_js_relative_index() {
        let mut files = HashSet::new();
        files.insert("src/utils/index.ts".to_string());

        let result = resolve_js_relative("./utils", "src/main.ts", Path::new("."), &files);
        assert_eq!(result, Some("src/utils/index.ts".to_string()));
    }

    #[test]
    fn resolve_js_relative_parent() {
        let mut files = HashSet::new();
        files.insert("helpers.js".to_string());

        let result = resolve_js_relative("../helpers", "src/main.ts", Path::new("."), &files);
        assert_eq!(result, Some("helpers.js".to_string()));
    }

    #[test]
    fn resolve_rust_crate_module() {
        let mut files = HashSet::new();
        files.insert("src/compress/mod.rs".to_string());

        let result = resolve_rust_module("crate::compress", "src/main.rs", Path::new("."), &files);
        assert_eq!(result, Some("src/compress/mod.rs".to_string()));
    }

    #[test]
    fn resolve_rust_crate_file() {
        let mut files = HashSet::new();
        files.insert("src/cli.rs".to_string());

        let result = resolve_rust_module("crate::cli", "src/main.rs", Path::new("."), &files);
        assert_eq!(result, Some("src/cli.rs".to_string()));
    }

    #[test]
    fn resolve_python_absolute() {
        let mut files = HashSet::new();
        files.insert("models/user.py".to_string());

        let result = resolve_python_module("models.user", Path::new("."), &files);
        assert_eq!(result, Some("models/user.py".to_string()));
    }

    #[test]
    fn resolve_external_returns_none() {
        let files = HashSet::new();
        let result = resolve_module_to_file("react", "src/app.tsx", Path::new("."), &files);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_system_include_returns_none() {
        let files = HashSet::new();
        let result = resolve_module_to_file("<stdio.h>", "src/main.c", Path::new("."), &files);
        assert!(result.is_none());
    }
}
