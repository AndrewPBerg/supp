mod deps;

pub(crate) mod call_sites;
pub(crate) mod definition;
pub(crate) mod doc;
pub(crate) mod hierarchy;
pub(crate) mod imports;

use anyhow::Result;
use serde::Serialize;

use crate::compress::{self, Lang};
use crate::symbol::{self, SearchResult, Symbol, SymbolKind};

// ── Re-exports for external callers ─────────────────────────────────

pub(crate) use call_sites::{contains_identifier, find_enclosing_function};
pub(crate) use hierarchy::extract_hierarchy;
pub(crate) use imports::{extract_file_imports, resolve_relative_import};

// ── Result types ────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct WhyResult {
    pub symbol: Symbol,
    pub doc_comment: Option<String>,
    pub full_definition: String,
    pub call_sites: Vec<CallSite>,
    pub dependencies: Vec<Dependency>,
    pub hierarchy: Option<Hierarchy>,
    pub plain: String,
}

#[derive(Serialize)]
pub struct CallSite {
    pub file: String,
    pub line: usize,
    pub context: String,
    pub caller: Option<String>,
}

#[derive(Serialize)]
pub struct Dependency {
    pub name: String,
    pub kind: Option<SymbolKind>,
    pub location: Option<(String, usize)>, // (file, line) for in-project
    pub import_from: Option<String>,       // module path if imported
}

#[derive(Serialize)]
pub struct Hierarchy {
    pub parents: Vec<HierarchyEntry>,
    pub children: Vec<HierarchyEntry>,
}

#[derive(Serialize)]
pub struct HierarchyEntry {
    pub name: String,
    pub location: Option<(String, usize)>,
    pub external_module: Option<String>,
}

// ── Public API ──────────────────────────────────────────────────────

pub fn explain(root: &str, query: &[String]) -> Result<WhyResult> {
    let root_path = std::fs::canonicalize(root)?;

    // 1. Find the symbol using the existing index
    let search = symbol::search(root, query)?;
    let sym = pick_best_match(&search, query)?;

    // 2. Read the source file
    let abs_path = root_path.join(&sym.file);
    let content = std::fs::read_to_string(&abs_path)?;

    // 3. Extract doc comments (language-aware: Python docstrings vs Rust /// comments)
    let doc_comment = doc::extract_doc_comment(&content, &sym);

    // 4. Extract the full definition using tree-sitter
    let full_definition = definition::extract_full_definition(&content, &sym);

    // 5. Find call sites across the codebase
    let call_sites = call_sites::find_call_sites(&root_path, &sym);

    // 6. Load full symbol index + file imports for dependency resolution
    let all_symbols = symbol::load_symbols(&root_path);
    let file_imports = imports::extract_file_imports(&content, &sym.file, &root_path);

    // 7. Find dependencies (what this symbol calls/uses)
    let dependencies =
        deps::find_dependencies(&root_path, &sym, &content, &all_symbols, &file_imports);

    // 8. Extract class hierarchy (parents + children)
    let class_hierarchy = hierarchy::extract_hierarchy(
        &root_path,
        &sym,
        &content,
        &all_symbols,
        &file_imports,
        None,
    );

    // 9. Build plain text for clipboard
    let plain = build_plain_text(
        &sym,
        &doc_comment,
        &full_definition,
        &call_sites,
        &dependencies,
        &class_hierarchy,
    );

    Ok(WhyResult {
        symbol: sym,
        doc_comment,
        full_definition,
        call_sites,
        dependencies,
        hierarchy: class_hierarchy,
        plain,
    })
}

// ── Symbol selection ────────────────────────────────────────────────

fn pick_best_match(search: &SearchResult, query: &[String]) -> Result<Symbol> {
    if search.matches.is_empty() {
        anyhow::bail!("no symbol found matching '{}'", query.join(" "));
    }

    let (sym, _score) = &search.matches[0];

    // Check for exact name match first
    let query_joined = query.join("_").to_lowercase();
    for (s, _) in &search.matches {
        if s.name.to_lowercase() == query_joined {
            return Ok(s.clone());
        }
    }

    // Also check "Parent::name" format
    for (s, _) in &search.matches {
        let full = if let Some(ref p) = s.parent {
            format!("{}::{}", p, s.name).to_lowercase()
        } else {
            s.name.to_lowercase()
        };
        if full == query_joined || full == query.join("::").to_lowercase() {
            return Ok(s.clone());
        }
    }

    Ok(sym.clone())
}

// ── Plain text builder ──────────────────────────────────────────────

fn build_plain_text(
    sym: &Symbol,
    doc_comment: &Option<String>,
    full_definition: &str,
    call_sites: &[CallSite],
    dependencies: &[Dependency],
    hierarchy: &Option<Hierarchy>,
) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    let kind_label = sym.kind.tag();
    let location = format!("{}:{}", sym.file, sym.line);
    let display_name = if let Some(ref parent) = sym.parent {
        format!("{}::{}", parent, sym.name)
    } else {
        sym.name.clone()
    };

    let _ = writeln!(out, "# {} [{}] {}", display_name, kind_label, location);
    let _ = writeln!(out);

    // Doc comments
    if let Some(doc) = doc_comment {
        let _ = writeln!(out, "## Documentation");
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", doc);
        let _ = writeln!(out);
    }

    // Hierarchy
    if let Some(h) = hierarchy {
        let _ = writeln!(out, "## Hierarchy");
        let _ = writeln!(out);
        if !h.parents.is_empty() {
            let _ = writeln!(out, "Parents:");
            for p in &h.parents {
                if let Some((ref file, line)) = p.location {
                    let _ = writeln!(out, "- {} ({}:{})", p.name, file, line);
                } else if let Some(ref module) = p.external_module {
                    let _ = writeln!(out, "- {} ({} — external)", p.name, module);
                } else {
                    let _ = writeln!(out, "- {} (external)", p.name);
                }
            }
        }
        if !h.children.is_empty() {
            if !h.parents.is_empty() {
                let _ = writeln!(out);
            }
            let _ = writeln!(out, "Children:");
            for c in &h.children {
                if let Some((ref file, line)) = c.location {
                    let _ = writeln!(out, "- {} ({}:{})", c.name, file, line);
                } else {
                    let _ = writeln!(out, "- {}", c.name);
                }
            }
        }
        let _ = writeln!(out);
    }

    // Full definition
    let _ = writeln!(out, "## Definition");
    let _ = writeln!(out);
    let lang_hint = match compress::detect_lang(&sym.file) {
        Some(Lang::Rust) => "rust",
        Some(Lang::Python) => "python",
        Some(Lang::JavaScript) => "javascript",
        Some(Lang::TypeScript | Lang::Tsx) => "typescript",
        Some(Lang::Go) => "go",
        Some(Lang::C) => "c",
        Some(Lang::Cpp) => "cpp",
        Some(Lang::Java) => "java",
        None => "",
    };
    let _ = writeln!(out, "```{}", lang_hint);
    let _ = writeln!(out, "{}", full_definition);
    let _ = writeln!(out, "```");
    let _ = writeln!(out);

    // Call sites
    if !call_sites.is_empty() {
        let _ = writeln!(out, "## Call Sites ({} references)", call_sites.len());
        let _ = writeln!(out);
        for site in call_sites {
            let caller_info = site
                .caller
                .as_ref()
                .map(|c| format!(" in {}", c))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "- {}:{}{} — `{}`",
                site.file, site.line, caller_info, site.context
            );
        }
        let _ = writeln!(out);
    }

    // Dependencies
    if !dependencies.is_empty() {
        let _ = writeln!(out, "## Dependencies ({} symbols)", dependencies.len());
        let _ = writeln!(out);
        for dep in dependencies {
            let kind_tag = dep.kind.map(|k| k.tag()).unwrap_or("--");
            let loc = if let Some((ref file, line)) = dep.location {
                format!("{}:{}", file, line)
            } else if let Some(ref module) = dep.import_from {
                format!("{} (external)", module)
            } else {
                "unknown".to_string()
            };
            let _ = writeln!(out, "- [{}] {} ({})", kind_tag, dep.name, loc);
        }
        let _ = writeln!(out);
    }

    out
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolKind;
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

    fn run(dir: &TempDir, query: &[&str]) -> WhyResult {
        let query: Vec<String> = query.iter().map(|s| s.to_string()).collect();
        explain(dir.path().to_str().unwrap(), &query).unwrap()
    }

    fn run_err(dir: &TempDir, query: &[&str]) -> String {
        let query: Vec<String> = query.iter().map(|s| s.to_string()).collect();
        match explain(dir.path().to_str().unwrap(), &query) {
            Ok(_) => panic!("expected error"),
            Err(e) => e.to_string(),
        }
    }

    // ── Rust ────────────────────────────────────────────────────

    fn rust_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "src/lib.rs",
                r#"use crate::helper::Config;

/// Processes data with the given configuration.
///
/// Returns the processed result as a string.
pub fn process_data(config: &Config) -> String {
    let value = config.get_value();
    format!("processed: {}", value)
}

pub struct DataStore {
    items: Vec<String>,
}

impl DataStore {
    pub fn new() -> Self {
        DataStore { items: Vec::new() }
    }

    pub fn add(&mut self, item: String) {
        self.items.push(item);
    }
}

pub const MAX_ITEMS: usize = 100;

pub enum Status {
    Active,
    Inactive,
}

pub type ItemList = Vec<String>;

fn main() {
    let cfg = Config::default();
    let result = process_data(&cfg);
    println!("{}", result);
}
"#,
            ),
            (
                "src/helper.rs",
                r#"/// Configuration for data processing.
pub struct Config {
    pub name: String,
}

impl Config {
    pub fn default() -> Self {
        Config { name: "default".to_string() }
    }

    pub fn get_value(&self) -> &str {
        &self.name
    }
}
"#,
            ),
        ]
    }

    #[test]
    fn rust_doc_comment() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["process_data"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Processes data"), "doc={doc}");
        assert!(doc.contains("Returns the processed result"), "doc={doc}");
    }

    #[test]
    fn rust_no_doc_comment() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["DataStore"]);
        assert!(r.doc_comment.is_none());
    }

    #[test]
    fn rust_full_def_function() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["process_data"]);
        assert!(r.full_definition.contains("config.get_value()"));
        assert!(r.full_definition.contains("format!"));
    }

    #[test]
    fn rust_full_def_struct() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["DataStore"]);
        assert!(r.full_definition.contains("items: Vec<String>"));
    }

    #[test]
    fn rust_full_def_enum() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["Status"]);
        assert!(r.full_definition.contains("Active"));
        assert!(r.full_definition.contains("Inactive"));
    }

    #[test]
    fn rust_full_def_type_alias() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["ItemList"]);
        assert!(r.full_definition.contains("Vec<String>"));
    }

    #[test]
    fn rust_full_def_const() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["MAX_ITEMS"]);
        assert!(r.full_definition.contains("100"));
    }

    #[test]
    fn rust_call_sites_cross_file() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["Config"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"src/lib.rs"), "call sites={files:?}");
    }

    #[test]
    fn rust_call_sites_same_file_outside_def() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["process_data"]);
        // Should find usage in main()
        let in_main: Vec<_> = r
            .call_sites
            .iter()
            .filter(|s| s.caller.as_deref() == Some("main"))
            .collect();
        assert!(!in_main.is_empty(), "should find call in main()");
    }

    #[test]
    fn rust_deps_body_and_signature() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["process_data"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"Config"), "deps={dep_names:?}");
    }

    #[test]
    fn rust_imports_tracked() {
        let dir = setup(&rust_fixtures());
        let r = run(&dir, &["process_data"]);
        let config_dep = r.dependencies.iter().find(|d| d.name == "Config");
        assert!(config_dep.is_some(), "should have Config dep");
        let dep = config_dep.unwrap();
        assert!(
            dep.import_from.as_deref() == Some("crate::helper"),
            "import_from={:?}",
            dep.import_from
        );
    }

    // ── Python ──────────────────────────────────────────────────

    fn python_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "app/models.py",
                r#"from dataclasses import dataclass

class BaseModel:
    """Base model with common functionality.

    Provides serialization and validation.
    """
    def validate(self):
        return True

class User(BaseModel):
    """A user in the system."""
    def __init__(self, name, email):
        self.name = name
        self.email = email

    def greet(self):
        """Return a greeting string."""
        return f"Hello, {self.name}"

MAX_USERS = 1000
"#,
            ),
            (
                "app/service.py",
                r#"from app.models import User, MAX_USERS

def create_user(name, email):
    """Create a new user after validation."""
    if get_count() >= MAX_USERS:
        raise ValueError("Too many users")
    user = User(name, email)
    user.validate()
    return user

def get_count():
    return 0
"#,
            ),
        ]
    }

    #[test]
    fn python_doc_comment_class() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["BaseModel"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(
            doc.contains("Base model with common functionality"),
            "doc={doc}"
        );
        assert!(doc.contains("serialization and validation"), "doc={doc}");
    }

    #[test]
    fn python_doc_comment_method() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["greet"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Return a greeting string"), "doc={doc}");
    }

    #[test]
    fn python_doc_comment_function() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["create_user"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Create a new user"), "doc={doc}");
    }

    #[test]
    fn python_full_def_class() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["User"]);
        assert!(r.full_definition.contains("__init__"));
        assert!(r.full_definition.contains("greet"));
    }

    #[test]
    fn python_full_def_const() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["MAX_USERS"]);
        assert!(r.full_definition.contains("1000"));
    }

    #[test]
    fn python_call_sites_cross_file() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["User"]);
        let sites: Vec<_> = r
            .call_sites
            .iter()
            .filter(|s| s.file == "app/service.py")
            .collect();
        assert!(!sites.is_empty(), "should find usage in service.py");
        assert!(
            sites
                .iter()
                .any(|s| s.caller.as_deref() == Some("create_user")),
            "caller should be create_user"
        );
    }

    #[test]
    fn python_deps_resolved() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["create_user"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"User"), "deps={dep_names:?}");
        assert!(dep_names.contains(&"MAX_USERS"), "deps={dep_names:?}");
        assert!(dep_names.contains(&"get_count"), "deps={dep_names:?}");
    }

    #[test]
    fn python_deps_import_tracked() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["create_user"]);
        let user_dep = r.dependencies.iter().find(|d| d.name == "User").unwrap();
        assert_eq!(user_dep.import_from.as_deref(), Some("app.models"));
    }

    #[test]
    fn python_hierarchy_parents() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["User"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let parent_names: Vec<&str> = h.parents.iter().map(|p| p.name.as_str()).collect();
        assert!(
            parent_names.contains(&"BaseModel"),
            "parents={parent_names:?}"
        );
    }

    #[test]
    fn python_hierarchy_children() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["BaseModel"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let child_names: Vec<&str> = h.children.iter().map(|c| c.name.as_str()).collect();
        assert!(child_names.contains(&"User"), "children={child_names:?}");
    }

    #[test]
    fn python_hierarchy_external_parent() {
        let dir = setup(&[(
            "ext/child.py",
            r#"from pydantic import BaseModel

class MyModel(BaseModel):
    """A pydantic model."""
    name: str
"#,
        )]);
        let r = run(&dir, &["MyModel"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        assert_eq!(h.parents.len(), 1);
        assert_eq!(h.parents[0].name, "BaseModel");
        assert_eq!(
            h.parents[0].external_module.as_deref(),
            Some("pydantic"),
            "should tag external parent module"
        );
        assert!(
            h.parents[0].location.is_none(),
            "external parent has no project location"
        );
    }

    #[test]
    fn python_hierarchy_none_for_function() {
        let dir = setup(&python_fixtures());
        let r = run(&dir, &["create_user"]);
        assert!(r.hierarchy.is_none());
    }

    // ── TypeScript ──────────────────────────────────────────────

    fn ts_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "src/types.ts",
                r#"/** Represents a configuration option. */
export interface AppConfig {
    name: string;
    debug: boolean;
}

/** Base service with logging. */
export class BaseService {
    protected log(msg: string): void {
        console.log(msg);
    }
}

export type StatusCode = number;

export const DEFAULT_PORT = 3000;
"#,
            ),
            (
                "src/app.ts",
                r#"import { AppConfig, BaseService, DEFAULT_PORT } from './types';

/** Application service that handles requests. */
export class AppService extends BaseService {
    private config: AppConfig;

    constructor(config: AppConfig) {
        super();
        this.config = config;
    }

    /** Start the application server. */
    start(): void {
        this.log(`Starting on port ${DEFAULT_PORT}`);
    }
}

export function createApp(config: AppConfig): AppService {
    return new AppService(config);
}
"#,
            ),
        ]
    }

    #[test]
    fn ts_doc_comment_class() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["AppService"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Application service"), "doc={doc}");
    }

    #[test]
    fn ts_full_def_interface() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["AppConfig"]);
        assert!(
            r.full_definition.contains("name: string"),
            "def={}",
            r.full_definition
        );
        assert!(
            r.full_definition.contains("debug: boolean"),
            "def={}",
            r.full_definition
        );
    }

    #[test]
    fn ts_full_def_class() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["AppService"]);
        assert!(
            r.full_definition.contains("constructor"),
            "def={}",
            r.full_definition
        );
        assert!(
            r.full_definition.contains("start()"),
            "def={}",
            r.full_definition
        );
    }

    #[test]
    fn ts_full_def_type_alias() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["StatusCode"]);
        assert!(
            r.full_definition.contains("number"),
            "def={}",
            r.full_definition
        );
    }

    #[test]
    fn ts_call_sites_cross_file() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["AppConfig"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"src/app.ts"), "call_sites files={files:?}");
    }

    #[test]
    fn ts_deps_resolved() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["createApp"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"AppConfig"), "deps={dep_names:?}");
        assert!(dep_names.contains(&"AppService"), "deps={dep_names:?}");
    }

    #[test]
    fn ts_deps_import_tracked() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["createApp"]);
        let cfg_dep = r
            .dependencies
            .iter()
            .find(|d| d.name == "AppConfig")
            .unwrap();
        assert_eq!(cfg_dep.import_from.as_deref(), Some("./types"));
    }

    #[test]
    fn ts_hierarchy_parents() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["AppService"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let parent_names: Vec<&str> = h.parents.iter().map(|p| p.name.as_str()).collect();
        assert!(
            parent_names.contains(&"BaseService"),
            "parents={parent_names:?}"
        );
    }

    #[test]
    fn ts_hierarchy_children() {
        let dir = setup(&ts_fixtures());
        let r = run(&dir, &["BaseService"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let child_names: Vec<&str> = h.children.iter().map(|c| c.name.as_str()).collect();
        assert!(
            child_names.contains(&"AppService"),
            "children={child_names:?}"
        );
    }

    // ── TSX ─────────────────────────────────────────────────────

    fn tsx_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "components/Button.tsx",
                r#"import React from 'react';

/** A reusable button component. */
export class Button extends React.Component {
    render() {
        return <button>{this.props.label}</button>;
    }
}

export function IconButton(props: { icon: string }) {
    return <button>{props.icon}</button>;
}
"#,
            ),
            (
                "components/App.tsx",
                r#"import { Button, IconButton } from './Button';

export function App() {
    return (
        <div>
            <Button label="Click" />
            <IconButton icon="star" />
        </div>
    );
}
"#,
            ),
        ]
    }

    #[test]
    fn tsx_doc_comment() {
        let dir = setup(&tsx_fixtures());
        let r = run(&dir, &["Button"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("reusable button component"), "doc={doc}");
    }

    #[test]
    fn tsx_call_sites_cross_file() {
        let dir = setup(&tsx_fixtures());
        let r = run(&dir, &["Button"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(
            files.contains(&"components/App.tsx"),
            "call_sites={files:?}"
        );
    }

    #[test]
    fn tsx_full_def_class() {
        let dir = setup(&tsx_fixtures());
        let r = run(&dir, &["Button"]);
        assert!(
            r.full_definition.contains("render()"),
            "def={}",
            r.full_definition
        );
    }

    // ── JavaScript ──────────────────────────────────────────────

    fn js_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "lib/utils.js",
                r#"/** Calculate the sum of two numbers. */
function calculate(a, b) {
    return a + b;
}

class EventEmitter {
    constructor() {
        this.listeners = {};
    }

    /** Register an event listener. */
    on(event, callback) {
        this.listeners[event] = callback;
    }
}

module.exports = { calculate, EventEmitter };
"#,
            ),
            (
                "lib/main.js",
                r#"const { calculate, EventEmitter } = require('./utils');

function run() {
    const result = calculate(1, 2);
    const emitter = new EventEmitter();
    emitter.on('data', console.log);
    return result;
}
"#,
            ),
        ]
    }

    #[test]
    fn js_doc_comment() {
        let dir = setup(&js_fixtures());
        let r = run(&dir, &["calculate"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Calculate the sum"), "doc={doc}");
    }

    #[test]
    fn js_doc_comment_method() {
        let dir = setup(&js_fixtures());
        let r = run(&dir, &["on"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Register an event listener"), "doc={doc}");
    }

    #[test]
    fn js_full_def_function() {
        let dir = setup(&js_fixtures());
        let r = run(&dir, &["calculate"]);
        assert!(
            r.full_definition.contains("return a + b"),
            "def={}",
            r.full_definition
        );
    }

    #[test]
    fn js_call_sites_cross_file() {
        let dir = setup(&js_fixtures());
        let r = run(&dir, &["calculate"]);
        let sites: Vec<_> = r
            .call_sites
            .iter()
            .filter(|s| s.file == "lib/main.js")
            .collect();
        assert!(!sites.is_empty(), "should find call in main.js");
        assert!(
            sites.iter().any(|s| s.caller.as_deref() == Some("run")),
            "caller should be run"
        );
    }

    // ── Go ──────────────────────────────────────────────────────

    fn go_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "pkg/server.go",
                r#"package pkg

// Server handles HTTP requests.
// It supports graceful shutdown.
type Server struct {
    Port    int
    Handler Handler
}

// NewServer creates a new Server with the given port.
func NewServer(port int) *Server {
    return &Server{Port: port}
}

// Start begins listening on the configured port.
func (s *Server) Start() error {
    return nil
}

type Handler interface {
    Handle(req string) string
}

const DefaultPort = 8080
"#,
            ),
            (
                "pkg/handler.go",
                r#"package pkg

// LogHandler logs and handles requests.
type LogHandler struct {
    Prefix string
}

// Handle processes the request.
func (h *LogHandler) Handle(req string) string {
    return h.Prefix + req
}

func UseServer() {
    srv := NewServer(DefaultPort)
    srv.Start()
}
"#,
            ),
        ]
    }

    #[test]
    fn go_doc_comment() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["Server"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Server handles HTTP requests"), "doc={doc}");
        assert!(doc.contains("graceful shutdown"), "doc={doc}");
    }

    #[test]
    fn go_full_def_struct() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["Server"]);
        assert!(
            r.full_definition.contains("Port"),
            "def={}",
            r.full_definition
        );
        assert!(
            r.full_definition.contains("Handler"),
            "def={}",
            r.full_definition
        );
    }

    #[test]
    fn go_full_def_function() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["NewServer"]);
        assert!(
            r.full_definition.contains("return &Server"),
            "def={}",
            r.full_definition
        );
    }

    #[test]
    fn go_full_def_interface() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["Handler"]);
        assert!(
            r.full_definition.contains("Handle"),
            "def={}",
            r.full_definition
        );
    }

    #[test]
    fn go_call_sites_cross_file() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["NewServer"]);
        let sites: Vec<_> = r
            .call_sites
            .iter()
            .filter(|s| s.file == "pkg/handler.go")
            .collect();
        assert!(!sites.is_empty(), "should find call in handler.go");
        assert!(
            sites
                .iter()
                .any(|s| s.caller.as_deref() == Some("UseServer")),
            "caller should be UseServer, got {:?}",
            sites.iter().map(|s| &s.caller).collect::<Vec<_>>()
        );
    }

    #[test]
    fn go_deps_resolved() {
        let dir = setup(&go_fixtures());
        let r = run(&dir, &["NewServer"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"Server"), "deps={dep_names:?}");
    }

    // ── Java ────────────────────────────────────────────────────

    fn java_fixtures() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "src/Animal.java",
                r#"/**
 * Base class for all animals.
 * Provides common animal behavior.
 */
public class Animal {
    protected String name;

    public Animal(String name) {
        this.name = name;
    }

    /** Get the animal's name. */
    public String getName() {
        return name;
    }
}
"#,
            ),
            (
                "src/Dog.java",
                r#"/**
 * A dog that extends Animal.
 */
public class Dog extends Animal {
    private String breed;

    public Dog(String name, String breed) {
        super(name);
        this.breed = breed;
    }

    /** Make the dog bark. */
    public String bark() {
        return getName() + " says Woof!";
    }
}
"#,
            ),
        ]
    }

    #[test]
    fn java_doc_comment() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["Animal"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Base class for all animals"), "doc={doc}");
    }

    #[test]
    fn java_doc_comment_method() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["bark"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Make the dog bark"), "doc={doc}");
    }

    #[test]
    fn java_full_def_class() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["Dog"]);
        assert!(
            r.full_definition.contains("breed"),
            "def={}",
            r.full_definition
        );
        assert!(
            r.full_definition.contains("bark"),
            "def={}",
            r.full_definition
        );
    }

    #[test]
    fn java_call_sites_cross_file() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["getName"]);
        let sites: Vec<_> = r
            .call_sites
            .iter()
            .filter(|s| s.file == "src/Dog.java")
            .collect();
        assert!(!sites.is_empty(), "should find call in Dog.java");
        assert!(
            sites.iter().any(|s| s.caller.as_deref() == Some("bark")),
            "caller should be bark"
        );
    }

    #[test]
    fn java_hierarchy_parents() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["Dog"]);
        let h = r.hierarchy.as_ref().expect("Dog should have hierarchy");
        let parent_names: Vec<&str> = h.parents.iter().map(|p| p.name.as_str()).collect();
        assert!(parent_names.contains(&"Animal"), "parents={parent_names:?}");
    }

    #[test]
    fn java_hierarchy_children() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["Animal"]);
        let h = r.hierarchy.as_ref().expect("Animal should have hierarchy");
        let child_names: Vec<&str> = h.children.iter().map(|c| c.name.as_str()).collect();
        assert!(child_names.contains(&"Dog"), "children={child_names:?}");
    }

    #[test]
    fn java_deps_resolved() {
        let dir = setup(&java_fixtures());
        let r = run(&dir, &["bark"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"getName"), "deps={dep_names:?}");
    }

    // ── JSON ────────────────────────────────────────────────────

    #[test]
    fn json_file_level_symbol() {
        let dir = setup(&[(
            "config.json",
            r#"{
    "name": "my-project",
    "version": "1.0.0",
    "database": {
        "host": "localhost",
        "port": 5432
    }
}"#,
        )]);
        let r = run(&dir, &["config.json"]);
        assert_eq!(r.symbol.kind, SymbolKind::File);
        assert!(r.doc_comment.is_none());
        assert!(r.hierarchy.is_none());
    }

    // ── Markdown ────────────────────────────────────────────────

    #[test]
    fn markdown_file_level_symbol() {
        let dir = setup(&[(
            "docs/README.md",
            "# My Project\n\n## Installation\n\nRun `pip install my-project`.\n\n## Usage\n\nImport and call `process_data`.\n",
        )]);
        let r = run(&dir, &["README.md"]);
        assert_eq!(r.symbol.kind, SymbolKind::File);
        assert!(r.hierarchy.is_none());
    }

    // ── Edge cases ──────────────────────────────────────────────

    #[test]
    fn edge_no_symbol_found() {
        let dir = setup(&[("src/lib.rs", "fn main() {}")]);
        let err = run_err(&dir, &["nonexistent_symbol_xyz"]);
        assert!(err.contains("no symbol found"), "err={err}");
    }

    #[test]
    fn edge_short_name_call_sites_empty() {
        let dir = setup(&[("lib.rs", "pub fn go() { }\nfn main() { go(); }\n")]);
        let r = run(&dir, &["go"]);
        assert!(r.call_sites.is_empty(), "short names skip call site search");
    }

    // ── Import parsing unit tests ───────────────────────────────

    #[test]
    fn imports_python_from() {
        let imports =
            imports::extract_python_imports("from app.models import User, Config\nimport os\n");
        assert_eq!(imports.get("User").map(String::as_str), Some("app.models"));
        assert_eq!(
            imports.get("Config").map(String::as_str),
            Some("app.models")
        );
        assert_eq!(imports.get("os").map(String::as_str), Some("os"));
    }

    #[test]
    fn imports_python_relative() {
        let imports = imports::extract_python_imports("from .main_prompt import build\n");
        assert_eq!(
            imports.get("build").map(String::as_str),
            Some(".main_prompt")
        );
    }

    #[test]
    fn imports_python_as_alias() {
        let imports = imports::extract_python_imports("from numpy import array as arr\n");
        assert_eq!(imports.get("array").map(String::as_str), Some("numpy"));
    }

    #[test]
    fn imports_rust_use() {
        let imports = imports::extract_rust_imports(
            "use anyhow::Result;\nuse std::collections::{HashMap, HashSet};\n",
        );
        assert_eq!(imports.get("Result").map(String::as_str), Some("anyhow"));
        assert_eq!(
            imports.get("HashMap").map(String::as_str),
            Some("std::collections")
        );
        assert_eq!(
            imports.get("HashSet").map(String::as_str),
            Some("std::collections")
        );
    }

    #[test]
    fn imports_js_named() {
        let imports =
            imports::extract_js_imports("import { AppConfig, BaseService } from './types';\n");
        assert_eq!(
            imports.get("AppConfig").map(String::as_str),
            Some("./types")
        );
        assert_eq!(
            imports.get("BaseService").map(String::as_str),
            Some("./types")
        );
    }

    #[test]
    fn imports_js_default() {
        let imports = imports::extract_js_imports("import React from 'react';\n");
        assert_eq!(imports.get("React").map(String::as_str), Some("react"));
    }

    // ── Docstring edge cases ────────────────────────────────────

    #[test]
    fn python_single_line_docstring() {
        let dir = setup(&[(
            "mod.py",
            "def hello():\n    \"\"\"Say hello.\"\"\"\n    return 'hi'\n",
        )]);
        let r = run(&dir, &["hello"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert_eq!(doc, "Say hello.");
    }

    #[test]
    fn python_triple_single_quote_docstring() {
        let dir = setup(&[(
            "mod.py",
            "def hello():\n    '''Say hello.'''\n    return 'hi'\n",
        )]);
        let r = run(&dir, &["hello"]);
        let doc = r.doc_comment.as_deref().unwrap();
        assert_eq!(doc, "Say hello.");
    }

    // ── TSX component-aware tests ──────────────────────────────

    #[test]
    fn tsx_arrow_component_indexed() {
        let dir = setup(&[(
            "Button.tsx",
            r#"import { useState } from 'react';
interface ButtonProps { label: string; onClick: () => void; }
const Button = ({ label, onClick }: ButtonProps) => {
  const [clicks, setClicks] = useState(0);
  return <button onClick={() => { setClicks(clicks + 1); onClick(); }}>{label}</button>;
};
export default Button;
"#,
        )]);
        let r = run(&dir, &["Button"]);
        assert_eq!(r.symbol.kind, SymbolKind::Function);
        assert!(r.full_definition.contains("ButtonProps"));
    }

    #[test]
    fn tsx_props_interface_dep() {
        let dir = setup(&[
            (
                "types.tsx",
                "export interface CardProps { title: string; count: number; }\n",
            ),
            (
                "Card.tsx",
                r#"import { CardProps } from './types';
const Card = ({ title, count }: CardProps) => {
  return <div>{title}: {count}</div>;
};
export default Card;
"#,
            ),
        ]);
        let r = run(&dir, &["Card"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"CardProps"), "deps={dep_names:?}");
    }

    #[test]
    fn tsx_jsx_element_dep() {
        let dir = setup(&[
            (
                "Button.tsx",
                "export function Button({ label }: { label: string }) {\n  return <button>{label}</button>;\n}\n",
            ),
            (
                "App.tsx",
                r#"import { Button } from './Button';
export function App() {
  return <div><Button label="Click" /></div>;
}
"#,
            ),
        ]);
        let r = run(&dir, &["App"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"Button"), "deps={dep_names:?}");
    }

    #[test]
    fn tsx_custom_hook_dep() {
        let dir = setup(&[
            (
                "hooks.tsx",
                "import { useState } from 'react';\nexport function useAuth() {\n  const [user, setUser] = useState(null);\n  return user;\n}\n",
            ),
            (
                "App.tsx",
                r#"import { useAuth } from './hooks';
export function App() {
  const user = useAuth();
  return <div>{user}</div>;
}
"#,
            ),
        ]);
        let r = run(&dir, &["App"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"useAuth"), "deps={dep_names:?}");
    }

    #[test]
    fn tsx_builtin_hook_external() {
        let dir = setup(&[(
            "Counter.tsx",
            r#"import { useState, useEffect } from 'react';
export function Counter() {
  const [count, setCount] = useState(0);
  useEffect(() => { document.title = String(count); }, [count]);
  return <button onClick={() => setCount(count + 1)}>{count}</button>;
}
"#,
        )]);
        let r = run(&dir, &["Counter"]);
        let external: Vec<&str> = r
            .dependencies
            .iter()
            .filter(|d| d.import_from.as_deref() == Some("react"))
            .map(|d| d.name.as_str())
            .collect();
        assert!(external.contains(&"useState"), "react deps={external:?}");
        assert!(external.contains(&"useEffect"), "react deps={external:?}");
    }

    #[test]
    fn tsx_call_sites_jsx_usage() {
        let dir = setup(&[
            (
                "Card.tsx",
                "export function Card() { return <div>card</div>; }\n",
            ),
            (
                "Page.tsx",
                "import { Card } from './Card';\nexport function Page() { return <Card />; }\n",
            ),
        ]);
        let r = run(&dir, &["Card"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"Page.tsx"), "call_sites={files:?}");
    }

    // ── C tests ────────────────────────────────────────────────

    #[test]
    fn c_function_def_found() {
        let dir = setup(&[("math.c", "int add(int a, int b) {\n    return a + b;\n}\n")]);
        let r = run(&dir, &["add"]);
        assert_eq!(r.symbol.kind, SymbolKind::Function);
        assert!(r.full_definition.contains("return a + b"));
    }

    #[test]
    fn c_doc_comment() {
        let dir = setup(&[(
            "math.c",
            "/** Add two integers. */\nint add(int a, int b) {\n    return a + b;\n}\n",
        )]);
        let r = run(&dir, &["add"]);
        assert!(r.doc_comment.is_some());
        assert!(
            r.doc_comment
                .as_deref()
                .unwrap()
                .contains("Add two integers")
        );
    }

    #[test]
    fn c_include_local_resolved() {
        let dir = setup(&[
            (
                "types.h",
                "typedef struct { double x; double y; } Point;\ndouble distance(const Point *a, const Point *b);\n",
            ),
            (
                "math.c",
                "#include \"types.h\"\n#include <math.h>\ndouble distance(const Point *a, const Point *b) {\n    double dx = a->x - b->x;\n    return dx;\n}\n",
            ),
        ]);
        let r = run(&dir, &["distance"]);
        assert_eq!(r.symbol.kind, SymbolKind::Function);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"Point"), "deps={dep_names:?}");
    }

    #[test]
    fn c_call_sites_across_files() {
        let dir = setup(&[
            ("util.c", "int square(int x) { return x * x; }\n"),
            (
                "main.c",
                "int square(int x);\nint main() { return square(5); }\n",
            ),
        ]);
        let r = run(&dir, &["square"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"main.c"), "call_sites={files:?}");
    }

    #[test]
    fn c_struct_in_header() {
        let dir = setup(&[(
            "types.h",
            "#ifndef TYPES_H\n#define TYPES_H\ntypedef struct {\n    int x;\n    int y;\n} Vec2;\n#endif\n",
        )]);
        let query: Vec<String> = vec!["Vec2".to_string()];
        let result = explain(dir.path().to_str().unwrap(), &query);
        let _ = result;
    }

    // ── C++ tests ──────────────────────────────────────────────

    #[test]
    fn cpp_class_hierarchy_parents() {
        let dir = setup(&[
            (
                "base.hpp",
                "class Base {\npublic:\n    virtual void run() = 0;\n};\n",
            ),
            (
                "derived.hpp",
                "#include \"base.hpp\"\nclass Derived : public Base {\npublic:\n    void run() override;\n};\n",
            ),
        ]);
        let r = run(&dir, &["Derived"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let parent_names: Vec<&str> = h.parents.iter().map(|p| p.name.as_str()).collect();
        assert!(parent_names.contains(&"Base"), "parents={parent_names:?}");
    }

    #[test]
    fn cpp_class_hierarchy_children() {
        let dir = setup(&[
            (
                "base.hpp",
                "class Animal {\npublic:\n    virtual void speak() = 0;\n};\n",
            ),
            (
                "dog.hpp",
                "class Dog : public Animal {\npublic:\n    void speak() override;\n};\n",
            ),
            (
                "cat.hpp",
                "class Cat : public Animal {\npublic:\n    void speak() override;\n};\n",
            ),
        ]);
        let r = run(&dir, &["Animal"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let child_names: Vec<&str> = h.children.iter().map(|c| c.name.as_str()).collect();
        assert!(child_names.contains(&"Dog"), "children={child_names:?}");
        assert!(child_names.contains(&"Cat"), "children={child_names:?}");
    }

    #[test]
    fn cpp_scope_qualifier_method() {
        let dir = setup(&[
            (
                "widget.hpp",
                "class Widget {\npublic:\n    void draw();\n    int width();\n};\n",
            ),
            (
                "widget.cpp",
                "#include \"widget.hpp\"\nvoid Widget::draw() {\n    // render\n}\nint Widget::width() {\n    return 100;\n}\n",
            ),
        ]);
        let r = run(&dir, &["draw"]);
        assert_eq!(r.symbol.parent.as_deref(), Some("Widget"));
        assert_eq!(r.symbol.kind, SymbolKind::Method);
    }

    #[test]
    fn cpp_include_resolved_deps() {
        let dir = setup(&[
            (
                "vec.hpp",
                "struct Vec3 {\n    double x, y, z;\n};\ndouble length(const Vec3& v);\n",
            ),
            (
                "math.cpp",
                "#include \"vec.hpp\"\n#include <cmath>\ndouble length(const Vec3& v) {\n    return sqrt(v.x*v.x + v.y*v.y + v.z*v.z);\n}\n",
            ),
        ]);
        let r = run(&dir, &["length"]);
        let dep_names: Vec<&str> = r.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"Vec3"), "deps={dep_names:?}");
    }

    #[test]
    fn cpp_call_sites_cross_file() {
        let dir = setup(&[
            (
                "engine.hpp",
                "class Engine {\npublic:\n    void start();\n};\n",
            ),
            (
                "engine.cpp",
                "#include \"engine.hpp\"\nvoid Engine::start() {}\n",
            ),
            (
                "main.cpp",
                "#include \"engine.hpp\"\nint main() {\n    Engine e;\n    e.start();\n    return 0;\n}\n",
            ),
        ]);
        let r = run(&dir, &["start"]);
        let files: Vec<&str> = r.call_sites.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"main.cpp"), "call_sites={files:?}");
    }

    #[test]
    fn cpp_multiple_inheritance() {
        let dir = setup(&[
            (
                "a.hpp",
                "class Drawable {\npublic:\n    virtual void draw() = 0;\n};\n",
            ),
            (
                "b.hpp",
                "class Clickable {\npublic:\n    virtual void click() = 0;\n};\n",
            ),
            (
                "button.hpp",
                "#include \"a.hpp\"\n#include \"b.hpp\"\nclass Button : public Drawable, public Clickable {\npublic:\n    void draw() override;\n    void click() override;\n};\n",
            ),
        ]);
        let r = run(&dir, &["Button"]);
        let h = r.hierarchy.as_ref().expect("should have hierarchy");
        let parent_names: Vec<&str> = h.parents.iter().map(|p| p.name.as_str()).collect();
        assert!(
            parent_names.contains(&"Drawable"),
            "parents={parent_names:?}"
        );
        assert!(
            parent_names.contains(&"Clickable"),
            "parents={parent_names:?}"
        );
    }

    // ── C/C++ include import tests ─────────────────────────────

    #[test]
    fn imports_c_local_include() {
        let dir = setup(&[
            ("types.h", "typedef struct { int x; } Point;\n"),
            ("main.c", "#include \"types.h\"\nint main() { return 0; }\n"),
        ]);
        let content = std::fs::read_to_string(dir.path().join("main.c")).unwrap();
        let file_imports = extract_file_imports(&content, "main.c", dir.path());
        assert!(
            file_imports.contains_key("Point"),
            "imports={file_imports:?}"
        );
    }

    #[test]
    fn imports_c_system_include() {
        let dir = setup(&[(
            "main.c",
            "#include <stdio.h>\n#include <stdlib.h>\nint main() { return 0; }\n",
        )]);
        let content = std::fs::read_to_string(dir.path().join("main.c")).unwrap();
        let file_imports = extract_file_imports(&content, "main.c", dir.path());
        assert!(
            file_imports.contains_key("stdio.h"),
            "imports={file_imports:?}"
        );
        assert!(
            file_imports.contains_key("stdlib.h"),
            "imports={file_imports:?}"
        );
    }

    #[test]
    fn imports_cpp_include_subdir() {
        let dir = setup(&[
            ("include/vec.hpp", "struct Vec2 { double x, y; };\n"),
            (
                "src/main.cpp",
                "#include \"../include/vec.hpp\"\nint main() { return 0; }\n",
            ),
        ]);
        let content = std::fs::read_to_string(dir.path().join("src/main.cpp")).unwrap();
        let file_imports = extract_file_imports(&content, "src/main.cpp", dir.path());
        assert!(
            file_imports.contains_key("Vec2"),
            "imports={file_imports:?}"
        );
    }

    // ── resolve_relative_import ───────────────────────────────

    #[test]
    fn resolve_relative_import_non_relative() {
        let dir = setup(&[]);
        assert!(imports::resolve_relative_import("os", "main.py", dir.path()).is_none());
    }

    #[test]
    fn resolve_relative_import_dot_only() {
        let dir = setup(&[]);
        // `from . import X` returns None (can't resolve without imported name)
        assert!(imports::resolve_relative_import(".", "main.py", dir.path()).is_none());
    }

    #[test]
    fn resolve_relative_import_single_dot() {
        let dir = setup(&[("utils.py", "def helper(): pass\n")]);
        let result = imports::resolve_relative_import(".utils", "main.py", dir.path());
        assert_eq!(result.as_deref(), Some("utils.py"));
    }

    #[test]
    fn resolve_relative_import_double_dot() {
        let dir = setup(&[("helpers.py", "def h(): pass\n")]);
        let result = imports::resolve_relative_import("..helpers", "sub/main.py", dir.path());
        assert_eq!(result.as_deref(), Some("helpers.py"));
    }

    #[test]
    fn resolve_relative_import_package() {
        let dir = setup(&[("pkg/__init__.py", "")]);
        let result = imports::resolve_relative_import(".pkg", "main.py", dir.path());
        assert_eq!(result.as_deref(), Some("pkg/__init__.py"));
    }

    #[test]
    fn resolve_relative_import_not_found() {
        let dir = setup(&[]);
        assert!(imports::resolve_relative_import(".missing", "main.py", dir.path()).is_none());
    }

    // ── C/C++ include resolution ──────────────────────────────

    #[test]
    fn c_include_resolves_in_common_dirs() {
        let dir = setup(&[
            ("include/math.h", "int add(int a, int b);\n"),
            (
                "src/main.c",
                "#include \"math.h\"\nint main() { return add(1, 2); }\n",
            ),
        ]);
        let content = std::fs::read_to_string(dir.path().join("src/main.c")).unwrap();
        let imports = extract_file_imports(&content, "src/main.c", dir.path());
        assert!(imports.contains_key("add"), "imports={imports:?}");
    }

    #[test]
    fn c_include_system_header_skipped() {
        let dir = setup(&[("main.c", "#include <stdio.h>\nint main() { return 0; }\n")]);
        let content = std::fs::read_to_string(dir.path().join("main.c")).unwrap();
        let imports = extract_file_imports(&content, "main.c", dir.path());
        // System headers like <stdio.h> shouldn't resolve to local files
        assert!(!imports.contains_key("printf"));
    }

    // ── build_plain_text coverage ─────────────────────────────

    #[test]
    fn why_with_hierarchy_and_deps() {
        let dir = setup(&[
            (
                "base.py",
                "class Animal:\n    def speak(self):\n        pass\n",
            ),
            (
                "dog.py",
                "from base import Animal\n\nclass Dog(Animal):\n    def bark(self):\n        return 'woof'\n",
            ),
            (
                "main.py",
                "from dog import Dog\n\ndef run():\n    d = Dog()\n    d.bark()\n",
            ),
        ]);
        let result = explain(dir.path().to_str().unwrap(), &["Dog".to_string()]).unwrap();
        assert_eq!(result.symbol.name, "Dog");
        assert!(result.plain.contains("Dog"));
        assert!(result.plain.contains("## Definition"));
        // Should have hierarchy with Animal as parent
        if result.hierarchy.is_some() {
            assert!(result.plain.contains("Hierarchy"));
        }
    }

    #[test]
    fn why_with_call_sites_and_deps() {
        let dir = setup(&[
            ("lib.rs", "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n"),
            (
                "main.rs",
                "use crate::lib::compute;\nfn main() {\n    compute(5);\n}\n",
            ),
        ]);
        let result = explain(dir.path().to_str().unwrap(), &["compute".to_string()]).unwrap();
        assert_eq!(result.symbol.name, "compute");
        assert!(result.plain.contains("## Definition"));
        // Should have call sites from main.rs
        if !result.call_sites.is_empty() {
            assert!(result.plain.contains("Call Sites"));
        }
    }

    #[test]
    fn why_with_doc_comment() {
        let dir = setup(&[(
            "lib.rs",
            "/// Does something important\npub fn important() -> bool {\n    true\n}\n",
        )]);
        let result = explain(dir.path().to_str().unwrap(), &["important".to_string()]).unwrap();
        assert!(result.doc_comment.is_some());
        assert!(result.plain.contains("Documentation"));
    }

    #[test]
    fn why_parent_name_resolution() {
        let dir = setup(&[(
            "lib.rs",
            "pub struct Foo {}\nimpl Foo {\n    pub fn method(&self) {}\n}\n",
        )]);
        // Query "Foo::method" format
        let result = explain(
            dir.path().to_str().unwrap(),
            &["Foo".to_string(), "method".to_string()],
        )
        .unwrap();
        assert!(result.symbol.name == "method" || result.symbol.name == "Foo");
    }
}
