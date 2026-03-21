mod map;

use regex::Regex;
use tree_sitter::{Language, Parser, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Full,
    Slim,
    Map,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    C,
    Cpp,
    Java,
}

pub fn detect_lang(file_path: &str) -> Option<Lang> {
    let ext = file_path.rsplit('.').next()?;
    match ext {
        "rs" => Some(Lang::Rust),
        "py" => Some(Lang::Python),
        "js" | "jsx" | "mjs" | "cjs" => Some(Lang::JavaScript),
        "ts" => Some(Lang::TypeScript),
        "tsx" => Some(Lang::Tsx),
        "go" => Some(Lang::Go),
        "c" | "h" => Some(Lang::C),
        "cpp" | "cc" | "hpp" | "cxx" => Some(Lang::Cpp),
        "java" => Some(Lang::Java),
        _ => None,
    }
}

pub fn get_language(lang: Lang) -> Language {
    match lang {
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::Python => tree_sitter_python::LANGUAGE.into(),
        Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::Go => tree_sitter_go::LANGUAGE.into(),
        Lang::C => tree_sitter_c::LANGUAGE.into(),
        Lang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Lang::Java => tree_sitter_java::LANGUAGE.into(),
    }
}

pub fn parse_source(content: &str, lang: Lang) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&get_language(lang)).ok()?;
    parser.parse(content, None)
}

#[allow(dead_code)]
pub fn lang_hint(file_path: &str) -> &'static str {
    match detect_lang(file_path) {
        Some(Lang::Rust) => "rust",
        Some(Lang::Python) => "python",
        Some(Lang::JavaScript) => "javascript",
        Some(Lang::TypeScript | Lang::Tsx) => "typescript",
        Some(Lang::Go) => "go",
        Some(Lang::C) => "c",
        Some(Lang::Cpp) => "cpp",
        Some(Lang::Java) => "java",
        None => "",
    }
}

pub fn compress(content: &str, file_path: &str, mode: Mode) -> String {
    if mode == Mode::Full {
        return content.to_string();
    }

    let lang = match detect_lang(file_path) {
        Some(l) => l,
        None => {
            // Unsupported: map falls back to slim, slim returns unchanged
            if mode == Mode::Map {
                return slim_fallback(content);
            }
            return content.to_string();
        }
    };

    match mode {
        Mode::Full => unreachable!(),
        Mode::Slim => slim(content, lang),
        Mode::Map => map::map(content, lang),
    }
}

pub fn node_text<'a>(source: &'a str, node: tree_sitter::Node) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}

// ── Slim Mode ──────────────────────────────────────────────────────

fn is_comment_kind(kind: &str, lang: Lang) -> bool {
    match lang {
        Lang::Rust | Lang::Java => matches!(kind, "line_comment" | "block_comment"),
        Lang::Python => kind == "comment",
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx | Lang::Go | Lang::C | Lang::Cpp => {
            kind == "comment"
        }
    }
}

fn slim(content: &str, lang: Lang) -> String {
    let tree = match parse_source(content, lang) {
        Some(t) => t,
        None => return content.to_string(),
    };

    // Collect comment byte ranges
    let mut comment_ranges: Vec<(usize, usize)> = Vec::new();
    let mut cursor = tree.walk();
    collect_comments(&mut cursor, lang, &mut comment_ranges);

    // Rebuild source skipping comment ranges
    let bytes = content.as_bytes();
    let mut result = String::with_capacity(content.len());
    let mut pos = 0;
    for (start, end) in &comment_ranges {
        if pos < *start {
            result.push_str(&content[pos..*start]);
        }
        pos = *end;
    }
    if pos < bytes.len() {
        result.push_str(&content[pos..]);
    }

    // Clean up lines that are now whitespace-only (from removed comments)
    let lines: Vec<&str> = result.lines().collect();
    let mut cleaned = String::new();
    for line in &lines {
        if line.trim().is_empty() {
            cleaned.push('\n');
        } else {
            cleaned.push_str(line);
            cleaned.push('\n');
        }
    }

    collapse_blank_lines(&cleaned)
}

fn collect_comments(
    cursor: &mut tree_sitter::TreeCursor,
    lang: Lang,
    ranges: &mut Vec<(usize, usize)>,
) {
    loop {
        let node = cursor.node();
        if is_comment_kind(node.kind(), lang) {
            ranges.push((node.start_byte(), node.end_byte()));
        } else if cursor.goto_first_child() {
            collect_comments(cursor, lang, ranges);
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Collapse runs of 3+ newlines down to 2 (one blank line)
fn collapse_blank_lines(s: &str) -> String {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\n{3,}").unwrap());
    re.replace_all(s, "\n\n").to_string()
}

/// Slim fallback for unsupported languages: just collapse blank lines
fn slim_fallback(content: &str) -> String {
    collapse_blank_lines(content)
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Language detection ──────────────────────────────────────

    #[test]
    fn detect_lang_rust() {
        assert_eq!(detect_lang("src/main.rs"), Some(Lang::Rust));
    }

    #[test]
    fn detect_lang_python() {
        assert_eq!(detect_lang("script.py"), Some(Lang::Python));
    }

    #[test]
    fn detect_lang_js_variants() {
        assert_eq!(detect_lang("app.js"), Some(Lang::JavaScript));
        assert_eq!(detect_lang("app.jsx"), Some(Lang::JavaScript));
        assert_eq!(detect_lang("app.mjs"), Some(Lang::JavaScript));
        assert_eq!(detect_lang("app.cjs"), Some(Lang::JavaScript));
    }

    #[test]
    fn detect_lang_ts_variants() {
        assert_eq!(detect_lang("app.ts"), Some(Lang::TypeScript));
        assert_eq!(detect_lang("app.tsx"), Some(Lang::Tsx));
    }

    #[test]
    fn detect_lang_go() {
        assert_eq!(detect_lang("main.go"), Some(Lang::Go));
    }

    #[test]
    fn detect_lang_c_cpp() {
        assert_eq!(detect_lang("main.c"), Some(Lang::C));
        assert_eq!(detect_lang("main.h"), Some(Lang::C));
        assert_eq!(detect_lang("main.cpp"), Some(Lang::Cpp));
        assert_eq!(detect_lang("main.cc"), Some(Lang::Cpp));
        assert_eq!(detect_lang("main.hpp"), Some(Lang::Cpp));
    }

    #[test]
    fn detect_lang_java() {
        assert_eq!(detect_lang("Main.java"), Some(Lang::Java));
    }

    #[test]
    fn detect_lang_unknown() {
        assert_eq!(detect_lang("file.txt"), None);
        assert_eq!(detect_lang("Makefile"), None);
    }

    // ── Full mode passthrough ──────────────────────────────────

    #[test]
    fn full_mode_returns_unchanged() {
        let src = "fn main() { println!(\"hello\"); }";
        assert_eq!(compress(src, "main.rs", Mode::Full), src);
    }

    // ── Unsupported language handling ──────────────────────────

    #[test]
    fn slim_unsupported_returns_unchanged() {
        let src = "some text\n\n\ncontent";
        assert_eq!(compress(src, "file.txt", Mode::Slim), src);
    }

    #[test]
    fn map_unsupported_falls_back_to_slim() {
        let src = "line1\n\n\n\nline2";
        let result = compress(src, "file.txt", Mode::Map);
        assert_eq!(result, "line1\n\nline2");
    }

    // ── Slim mode: Rust ────────────────────────────────────────

    #[test]
    fn slim_rust_removes_line_comments() {
        let src = "// a comment\nfn main() {}\n// another\n";
        let result = compress(src, "main.rs", Mode::Slim);
        assert!(!result.contains("// a comment"));
        assert!(!result.contains("// another"));
        assert!(result.contains("fn main() {}"));
    }

    #[test]
    fn slim_rust_removes_block_comments() {
        let src = "/* block\ncomment */\nfn main() {}\n";
        let result = compress(src, "main.rs", Mode::Slim);
        assert!(!result.contains("block"));
        assert!(result.contains("fn main() {}"));
    }

    #[test]
    fn slim_rust_collapses_blank_lines() {
        let src = "use std::io;\n\n\n\n\nfn main() {}\n";
        let result = compress(src, "main.rs", Mode::Slim);
        assert!(!result.contains("\n\n\n"));
        assert!(result.contains("use std::io;"));
        assert!(result.contains("fn main() {}"));
    }

    #[test]
    fn slim_rust_preserves_string_contents() {
        let src = "let s = \"// not a comment\";\n";
        let result = compress(src, "main.rs", Mode::Slim);
        assert!(result.contains("// not a comment"));
    }

    #[test]
    fn slim_rust_trailing_comment() {
        let src = "let x = 1; // comment\n";
        let result = compress(src, "main.rs", Mode::Slim);
        assert!(result.contains("let x = 1;"));
        assert!(!result.contains("// comment"));
    }

    #[test]
    fn slim_empty_file() {
        assert_eq!(compress("", "main.rs", Mode::Slim), "");
    }

    #[test]
    fn slim_only_comments() {
        let src = "// just a comment\n/* and this */\n";
        let result = compress(src, "main.rs", Mode::Slim);
        let trimmed = result.trim();
        assert!(trimmed.is_empty(), "expected empty, got: {:?}", trimmed);
    }

    // ── Slim mode: Python ──────────────────────────────────────

    #[test]
    fn slim_python_removes_comments() {
        let src = "# comment\nx = 1\n# another\n";
        let result = compress(src, "script.py", Mode::Slim);
        assert!(!result.contains("# comment"));
        assert!(result.contains("x = 1"));
    }

    // ── Slim mode: JS ──────────────────────────────────────────

    #[test]
    fn slim_js_removes_comments() {
        let src = "// line comment\nconst x = 1;\n/* block */\n";
        let result = compress(src, "app.js", Mode::Slim);
        assert!(!result.contains("// line"));
        assert!(!result.contains("block"));
        assert!(result.contains("const x = 1;"));
    }

    // ── Map mode: Rust ─────────────────────────────────────────

    #[test]
    fn map_rust_function_bodies_replaced() {
        let src = "\
use std::io;

fn hello(name: &str) -> String {
    format!(\"Hello, {}!\", name)
}

fn main() {
    let msg = hello(\"world\");
    println!(\"{}\", msg);
}
";
        let result = compress(src, "main.rs", Mode::Map);
        assert!(result.contains("use std::io;"));
        assert!(result.contains("fn hello(name: &str) -> String { ... }"));
        assert!(result.contains("fn main() { ... }"));
        assert!(!result.contains("format!"));
        assert!(!result.contains("println!"));
    }

    #[test]
    fn map_rust_struct_enum() {
        let src = "\
struct Point {
    x: f64,
    y: f64,
}

enum Color {
    Red,
    Green,
    Blue,
}
";
        let result = compress(src, "main.rs", Mode::Map);
        assert!(result.contains("struct Point { ... }"));
        assert!(result.contains("enum Color { ... }"));
        assert!(!result.contains("x: f64"));
    }

    #[test]
    fn map_rust_impl_block() {
        let src = "\
impl Point {
    fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    fn distance(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}
";
        let result = compress(src, "main.rs", Mode::Map);
        assert!(result.contains("impl Point {"));
        assert!(result.contains("fn new(x: f64, y: f64) -> Self { ... }"));
        assert!(result.contains("fn distance(&self) -> f64 { ... }"));
        assert!(!result.contains("sqrt"));
    }

    #[test]
    fn map_rust_trait() {
        let src = "\
trait Drawable {
    fn draw(&self);
    fn area(&self) -> f64 {
        0.0
    }
}
";
        let result = compress(src, "main.rs", Mode::Map);
        assert!(result.contains("trait Drawable {"));
        assert!(result.contains("fn draw(&self);"));
        assert!(result.contains("fn area(&self) -> f64 { ... }"));
    }

    // ── Map mode: Python ───────────────────────────────────────

    #[test]
    fn map_python_functions_and_classes() {
        let src = "\
import os
from pathlib import Path

def greet(name: str) -> str:
    return f\"Hello, {name}\"

class MyClass:
    x: int = 0

    def method(self) -> None:
        print(\"hello\")

    @staticmethod
    def static_method() -> int:
        return 42
";
        let result = compress(src, "main.py", Mode::Map);
        assert!(result.contains("import os"));
        assert!(result.contains("from pathlib import Path"));
        assert!(result.contains("def greet(name: str) -> str: ..."));
        assert!(result.contains("class MyClass:"));
        assert!(result.contains("def method(self) -> None: ..."));
        assert!(result.contains("@staticmethod"));
        assert!(result.contains("def static_method() -> int: ..."));
        assert!(!result.contains("return f\"Hello"));
        assert!(!result.contains("print("));
    }

    // ── Map mode: JavaScript ───────────────────────────────────

    #[test]
    fn map_js_functions_and_classes() {
        let src = "\
import { foo } from './bar';

function greet(name) {
    return `Hello, ${name}`;
}

class Animal {
    constructor(name) {
        this.name = name;
    }

    speak() {
        console.log(this.name);
    }
}
";
        let result = compress(src, "app.js", Mode::Map);
        assert!(result.contains("import { foo } from './bar';"));
        assert!(result.contains("function greet(name) { ... }"));
        assert!(result.contains("class Animal {"));
        assert!(result.contains("constructor(name) { ... }"));
        assert!(result.contains("speak() { ... }"));
        assert!(!result.contains("console.log"));
    }

    // ── Map mode: TypeScript ───────────────────────────────────

    #[test]
    fn map_ts_interfaces_and_types() {
        let src = "\
import { Request } from 'express';

interface User {
    name: string;
    age: number;
}

type ID = string | number;

enum Status {
    Active,
    Inactive,
}

function handler(req: Request): void {
    console.log(req);
}
";
        let result = compress(src, "app.ts", Mode::Map);
        assert!(result.contains("import { Request } from 'express';"));
        assert!(result.contains("interface User { ... }"));
        assert!(result.contains("type ID = string | number;"));
        assert!(result.contains("enum Status { ... }"));
        assert!(result.contains("function handler(req: Request): void { ... }"));
    }

    // ── Map mode: Go ───────────────────────────────────────────

    #[test]
    fn map_go_functions_and_types() {
        let src = "\
package main

import \"fmt\"

func greet(name string) string {
\treturn fmt.Sprintf(\"Hello, %s\", name)
}

func main() {
\tfmt.Println(greet(\"world\"))
}
";
        let result = compress(src, "main.go", Mode::Map);
        assert!(result.contains("package main"));
        assert!(result.contains("import \"fmt\""));
        assert!(result.contains("func greet(name string) string { ... }"));
        assert!(result.contains("func main() { ... }"));
        assert!(!result.contains("Sprintf"));
    }

    // ── Map mode: Java ─────────────────────────────────────────

    #[test]
    fn map_java_class() {
        let src = "\
package com.example;

import java.util.List;

public class Main {
    private int count;

    public Main(int count) {
        this.count = count;
    }

    public int getCount() {
        return this.count;
    }
}
";
        let result = compress(src, "Main.java", Mode::Map);
        assert!(result.contains("package com.example;"));
        assert!(result.contains("import java.util.List;"));
        assert!(result.contains("public class Main {"));
        assert!(result.contains("public Main(int count) { ... }"));
        assert!(result.contains("public int getCount() { ... }"));
        assert!(!result.contains("return this.count"));
    }

    // ── Map mode: C ────────────────────────────────────────────

    #[test]
    fn map_c_functions_and_structs() {
        let src = "\
#include <stdio.h>
#define MAX 100

struct Point {
    int x;
    int y;
};

int add(int a, int b) {
    return a + b;
}

int main() {
    printf(\"%d\\n\", add(1, 2));
    return 0;
}
";
        let result = compress(src, "main.c", Mode::Map);
        assert!(result.contains("#include <stdio.h>"));
        assert!(result.contains("#define MAX 100"));
        assert!(result.contains("int add(int a, int b) { ... }"));
        assert!(result.contains("int main() { ... }"));
        assert!(!result.contains("printf"));
    }
}
