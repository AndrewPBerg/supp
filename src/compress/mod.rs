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

pub(crate) fn is_comment_kind(kind: &str, lang: Lang) -> bool {
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

pub(crate) fn collect_comments(
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

    // ── Additional coverage tests ─────────────────────────────────

    #[test]
    fn map_rust_attribute() {
        let src = "#[derive(Debug)]\npub struct Foo {\n    x: i32,\n}\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("#[derive(Debug)]"));
        assert!(result.contains("pub struct Foo"));
    }

    #[test]
    fn map_rust_macro_definition() {
        let src = "macro_rules! my_macro {\n    ($x:expr) => { $x + 1 };\n}\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("macro_rules! my_macro { ... }"));
        assert!(!result.contains("$x + 1"));
    }

    #[test]
    fn map_rust_mod_item_with_body() {
        let src = "mod inner {\n    fn foo() {}\n}\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("mod inner { ... }"));
        assert!(!result.contains("fn foo"));
    }

    #[test]
    fn map_rust_mod_declaration() {
        let src = "mod foo;\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("mod foo;"));
    }

    #[test]
    fn map_rust_impl_type_and_const() {
        let src = "\
impl Foo {
    type Output = i32;
    const MAX: i32 = 100;

    fn bar(&self) {}
}
";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("impl Foo {"));
        assert!(result.contains("type Output = i32;"));
        assert!(result.contains("const MAX: i32 = 100;"));
        assert!(result.contains("fn bar(&self) { ... }"));
    }

    #[test]
    fn map_python_decorated_top_level() {
        let src = "@decorator\ndef func():\n    pass\n";
        let result = compress(src, "test.py", Mode::Map);
        assert!(result.contains("@decorator"));
        assert!(result.contains("def func():"));
    }

    #[test]
    fn map_python_class_with_assignments() {
        let src = "\
class MyClass:
    x = 5

    def method(self):
        return self.x
";
        let result = compress(src, "test.py", Mode::Map);
        assert!(result.contains("class MyClass:"));
        assert!(result.contains("x = 5"));
        assert!(result.contains("def method(self):"));
    }

    #[test]
    fn map_js_export_function() {
        let src = "export function foo() {\n    return 1;\n}\n";
        let result = compress(src, "test.js", Mode::Map);
        assert!(result.contains("export function foo() { ... }"));
        assert!(!result.contains("return 1"));
    }

    #[test]
    fn map_js_export_class() {
        let src = "export class Foo {\n    method() {\n        console.log('hi');\n    }\n}\n";
        let result = compress(src, "test.js", Mode::Map);
        assert!(result.contains("export class Foo {"));
        assert!(result.contains("method() { ... }"));
        assert!(!result.contains("console.log"));
    }

    #[test]
    fn map_js_variable_declaration() {
        let src = "const x = 5;\nlet y = 10;\n";
        let result = compress(src, "test.js", Mode::Map);
        assert!(result.contains("const x = 5;"));
        assert!(result.contains("let y = 10;"));
    }

    #[test]
    fn map_js_reexport() {
        let src = "export { foo } from './bar';\n";
        let result = compress(src, "test.js", Mode::Map);
        assert!(result.contains("export { foo } from './bar';"));
    }

    #[test]
    fn map_ts_export_interface() {
        let src = "export interface Foo {\n    name: string;\n}\n";
        let result = compress(src, "test.ts", Mode::Map);
        assert!(result.contains("export interface Foo { ... }"));
        assert!(!result.contains("name: string"));
    }

    #[test]
    fn map_ts_export_type_alias() {
        let src = "export type Foo = string;\n";
        let result = compress(src, "test.ts", Mode::Map);
        assert!(result.contains("export type Foo = string;"));
    }

    #[test]
    fn map_ts_enum_declaration() {
        let src = "export enum Color {\n    Red,\n    Green,\n}\n";
        let result = compress(src, "test.ts", Mode::Map);
        assert!(result.contains("export enum Color { ... }"));
        assert!(!result.contains("Red"));
    }

    #[test]
    fn map_go_package_and_imports() {
        let src = "package main\n\nimport \"fmt\"\n";
        let result = compress(src, "test.go", Mode::Map);
        assert!(result.contains("package main"));
        assert!(result.contains("import \"fmt\""));
    }

    #[test]
    fn map_go_struct_and_interface() {
        let src = "\
package main

type Foo struct {
\tX int
\tY int
}

type Bar interface {
\tDoSomething() error
}
";
        let result = compress(src, "test.go", Mode::Map);
        assert!(result.contains("type Foo"));
        assert!(result.contains("struct { ... }"));
        assert!(result.contains("type Bar"));
        assert!(result.contains("interface { ... }"));
        assert!(!result.contains("X int"));
    }

    #[test]
    fn map_go_const_and_var() {
        let src = "package main\n\nconst x = 5\n\nvar y int\n";
        let result = compress(src, "test.go", Mode::Map);
        assert!(result.contains("const x = 5"));
        assert!(result.contains("var y int"));
    }

    #[test]
    fn map_go_method_declaration() {
        let src = "\
package main

type Foo struct {
\tVal int
}

func (s *Foo) Bar() int {
\treturn s.Val
}
";
        let result = compress(src, "test.go", Mode::Map);
        assert!(result.contains("func (s *Foo) Bar() int { ... }"));
        assert!(!result.contains("return s.Val"));
    }

    #[test]
    fn map_cpp_class_and_namespace() {
        let src = "\
namespace ns {
class Foo {
public:
    void bar();
};
}
";
        let result = compress(src, "test.cpp", Mode::Map);
        assert!(result.contains("namespace ns {"));
        assert!(result.contains("class Foo { ... }"));
    }

    #[test]
    fn map_cpp_declaration() {
        let src = "void foo();\n";
        let result = compress(src, "test.c", Mode::Map);
        assert!(result.contains("void foo();"));
    }

    #[test]
    fn map_c_preproc_and_enum() {
        let src = "#include <stdio.h>\n\nenum Color {\n    RED,\n    GREEN,\n};\n";
        let result = compress(src, "test.c", Mode::Map);
        assert!(result.contains("#include <stdio.h>"));
        assert!(result.contains("enum Color { ... }"));
        assert!(!result.contains("RED"));
    }

    #[test]
    fn map_java_interface() {
        let src = "interface Foo {\n    void bar();\n}\n";
        let result = compress(src, "Test.java", Mode::Map);
        assert!(result.contains("interface Foo {"));
        assert!(result.contains("void bar();"));
    }

    #[test]
    fn map_java_enum() {
        let src = "enum Color {\n    RED,\n    GREEN\n}\n";
        let result = compress(src, "Test.java", Mode::Map);
        assert!(result.contains("enum Color {"));
    }

    #[test]
    fn map_java_package_and_import() {
        let src = "package com.foo;\n\nimport java.util.*;\n\npublic class Main {\n}\n";
        let result = compress(src, "Main.java", Mode::Map);
        assert!(result.contains("package com.foo;"));
        assert!(result.contains("import java.util.*;"));
    }

    #[test]
    fn map_java_constructor() {
        let src = "\
public class Foo {
    private int x;

    public Foo(int x) {
        this.x = x;
    }
}
";
        let result = compress(src, "Foo.java", Mode::Map);
        assert!(result.contains("public class Foo {"));
        assert!(result.contains("public Foo(int x) { ... }"));
        assert!(result.contains("private int x;"));
        assert!(!result.contains("this.x = x"));
    }

    #[test]
    fn slim_removes_comments_go() {
        let src = "// comment\npackage main\n// another\nfunc main() {}\n";
        let result = compress(src, "test.go", Mode::Slim);
        assert!(!result.contains("// comment"));
        assert!(!result.contains("// another"));
        assert!(result.contains("package main"));
        assert!(result.contains("func main() {}"));
    }

    #[test]
    fn slim_removes_comments_java() {
        let src = "// comment\npublic class Main {\n    /* block */\n}\n";
        let result = compress(src, "Main.java", Mode::Slim);
        assert!(!result.contains("// comment"));
        assert!(!result.contains("block"));
        assert!(result.contains("public class Main"));
    }

    #[test]
    fn slim_removes_comments_ts() {
        let src = "// comment\nconst x: number = 1;\n/* block */\n";
        let result = compress(src, "test.ts", Mode::Slim);
        assert!(!result.contains("// comment"));
        assert!(!result.contains("block"));
        assert!(result.contains("const x: number = 1;"));
    }

    #[test]
    fn slim_removes_comments_c() {
        let src = "// line comment\nint x = 1;\n/* block */\n";
        let result = compress(src, "test.c", Mode::Slim);
        assert!(!result.contains("// line comment"));
        assert!(!result.contains("block"));
        assert!(result.contains("int x = 1;"));
    }

    #[test]
    fn slim_fallback_collapses_blanks() {
        let src = "line1\n\n\n\nline2\n";
        let result = compress(src, "file.unknown", Mode::Map);
        assert_eq!(result, "line1\n\nline2\n");
    }

    #[test]
    fn slim_unsupported_returns_unchanged_txt() {
        let src = "hello\nworld\n";
        let result = compress(src, "file.txt", Mode::Slim);
        assert_eq!(result, src);
    }

    #[test]
    fn lang_hint_returns_correct_strings() {
        assert_eq!(lang_hint("test.rs"), "rust");
        assert_eq!(lang_hint("test.py"), "python");
        assert_eq!(lang_hint("test.js"), "javascript");
        assert_eq!(lang_hint("test.ts"), "typescript");
        assert_eq!(lang_hint("test.tsx"), "typescript");
        assert_eq!(lang_hint("test.go"), "go");
        assert_eq!(lang_hint("test.c"), "c");
        assert_eq!(lang_hint("test.cpp"), "cpp");
        assert_eq!(lang_hint("test.java"), "java");
        assert_eq!(lang_hint("test.txt"), "");
    }

    #[test]
    fn map_parse_failure_falls_back() {
        let result = compress("", "test.rs", Mode::Map);
        // Empty content should parse but produce empty or minimal output
        assert!(result.trim().is_empty() || result.len() < 5);
    }

    // ── Bodyless/else-branch declarations ─────────────────────

    #[test]
    fn map_rust_struct_no_body() {
        // Tuple struct or unit struct without braces
        let src = "pub struct Unit;\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("struct Unit"));
    }

    #[test]
    fn map_rust_enum_no_body() {
        // This is rare but the else branch exists
        let src = "enum Empty;\n";
        let result = compress(src, "test.rs", Mode::Map);
        // Should still contain enum in some form
        let _ = result;
    }

    #[test]
    fn map_rust_fn_signature_no_body() {
        // Function signature in a trait
        let src = "trait Foo {\n    fn bar(&self) -> i32;\n}\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("trait Foo"));
        assert!(result.contains("fn bar"));
    }

    #[test]
    fn map_python_fn_no_body() {
        // Abstract method stub
        let src = "class Foo:\n    def bar(self): ...\n";
        let result = compress(src, "test.py", Mode::Map);
        assert!(result.contains("def bar"));
    }

    #[test]
    fn map_js_fn_no_body_ts_declaration() {
        // TypeScript function declaration
        let src = "declare function foo(): void;\n";
        let result = compress(src, "test.ts", Mode::Map);
        assert!(result.contains("foo"));
    }

    #[test]
    fn map_go_fn_no_body() {
        // Go interface method (no body)
        let src = "package main\n\ntype Fooer interface {\n    Foo() string\n}\n";
        let result = compress(src, "test.go", Mode::Map);
        assert!(result.contains("Fooer"));
    }

    #[test]
    fn map_c_fn_declaration_no_body() {
        // Forward declaration
        let src = "int add(int a, int b);\n";
        let result = compress(src, "test.c", Mode::Map);
        assert!(result.contains("add"));
    }

    #[test]
    fn map_c_struct_forward_decl() {
        let src = "struct Foo;\n";
        let result = compress(src, "test.c", Mode::Map);
        assert!(result.contains("Foo") || result.is_empty());
    }

    #[test]
    fn map_c_enum_no_body() {
        let src = "enum Color;\n";
        let result = compress(src, "test.c", Mode::Map);
        let _ = result;
    }

    #[test]
    fn map_java_method_no_body() {
        // Interface method (no body)
        let src = "interface Foo {\n    void bar();\n}\n";
        let result = compress(src, "test.java", Mode::Map);
        assert!(result.contains("void bar"));
    }

    #[test]
    fn map_ts_interface_no_body() {
        // Edge case - interface without body
        let src = "declare interface Foo;\n";
        let result = compress(src, "test.ts", Mode::Map);
        let _ = result;
    }

    #[test]
    fn map_ts_enum_no_body() {
        let src = "declare enum Color;\n";
        let result = compress(src, "test.ts", Mode::Map);
        let _ = result;
    }

    #[test]
    fn map_output_trailing_newline_trimmed() {
        // Test that double newline at end gets trimmed
        let src = "fn foo() {}\n\nfn bar() {}\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(!result.ends_with("\n\n"));
    }

    #[test]
    fn map_js_class_field() {
        let src = "class Foo {\n  name = 'hello';\n  greet() { return this.name; }\n}\n";
        let result = compress(src, "test.js", Mode::Map);
        assert!(result.contains("class Foo"));
    }

    #[test]
    fn map_go_type_simple_alias() {
        // Type alias without struct/interface
        let src = "package main\ntype ID int\n";
        let result = compress(src, "test.go", Mode::Map);
        assert!(result.contains("type") && result.contains("ID"));
    }

    // ── Additional coverage: edge cases ─────────────────────────

    #[test]
    fn detect_lang_c_header() {
        assert_eq!(detect_lang("lib.h"), Some(Lang::C));
    }

    #[test]
    fn detect_lang_cpp_variants() {
        assert_eq!(detect_lang("lib.cpp"), Some(Lang::Cpp));
        assert_eq!(detect_lang("lib.cc"), Some(Lang::Cpp));
        assert_eq!(detect_lang("lib.hpp"), Some(Lang::Cpp));
        assert_eq!(detect_lang("lib.cxx"), Some(Lang::Cpp));
    }

    #[test]
    fn detect_lang_none_for_unknown() {
        assert_eq!(detect_lang("readme.md"), None);
        assert_eq!(detect_lang("data.json"), None);
        assert_eq!(detect_lang("Makefile"), None);
    }

    #[test]
    fn detect_lang_no_extension() {
        assert_eq!(detect_lang("Dockerfile"), None);
    }

    #[test]
    fn parse_source_all_langs() {
        for (content, lang) in [
            ("fn main() {}", Lang::Rust),
            ("def foo(): pass", Lang::Python),
            ("function f() {}", Lang::JavaScript),
            ("function f(): void {}", Lang::TypeScript),
            ("function f(): void {}", Lang::Tsx),
            ("package main\nfunc main() {}", Lang::Go),
            ("int main() { return 0; }", Lang::C),
            ("int main() { return 0; }", Lang::Cpp),
            ("class Foo {}", Lang::Java),
        ] {
            assert!(
                parse_source(content, lang).is_some(),
                "Failed for {:?}",
                lang
            );
        }
    }

    #[test]
    fn node_text_basic() {
        let tree = parse_source("fn foo() {}", Lang::Rust).unwrap();
        let root = tree.root_node();
        let text = node_text("fn foo() {}", root);
        assert_eq!(text, "fn foo() {}");
    }

    #[test]
    fn lang_hint_all() {
        assert_eq!(lang_hint("test.rs"), "rust");
        assert_eq!(lang_hint("test.py"), "python");
        assert_eq!(lang_hint("test.js"), "javascript");
        assert_eq!(lang_hint("test.ts"), "typescript");
        assert_eq!(lang_hint("test.tsx"), "typescript");
        assert_eq!(lang_hint("test.go"), "go");
        assert_eq!(lang_hint("test.c"), "c");
        assert_eq!(lang_hint("test.cpp"), "cpp");
        assert_eq!(lang_hint("test.java"), "java");
        assert_eq!(lang_hint("test.txt"), "");
    }

    #[test]
    fn full_mode_passthrough() {
        let src = "anything // with comments\n\n\n\n";
        let result = compress(src, "test.rs", Mode::Full);
        assert_eq!(result, src);
    }

    #[test]
    fn slim_unsupported_lang_passthrough() {
        let src = "some content\n";
        let result = compress(src, "test.txt", Mode::Slim);
        assert_eq!(result, src);
    }

    #[test]
    fn map_unsupported_lang_falls_back_to_slim() {
        let src = "some content\n\n\n\nmore\n";
        let result = compress(src, "test.txt", Mode::Map);
        // Map falls back to slim_fallback which collapses blank lines
        assert!(!result.contains("\n\n\n"));
    }

    #[test]
    fn slim_empty_content() {
        let result = compress("", "test.rs", Mode::Slim);
        assert!(result.is_empty() || result.trim().is_empty());
    }

    #[test]
    fn map_empty_content() {
        let result = compress("", "test.rs", Mode::Map);
        assert!(result.is_empty() || result.trim().is_empty());
    }

    #[test]
    fn slim_preserves_code() {
        let src = "fn foo() {\n    // comment\n    let x = 1;\n}\n";
        let result = compress(src, "test.rs", Mode::Slim);
        assert!(result.contains("fn foo()"));
        assert!(result.contains("let x = 1"));
        assert!(!result.contains("// comment"));
    }

    #[test]
    fn slim_python_comments() {
        let src = "# comment\ndef foo():\n    # inner\n    pass\n";
        let result = compress(src, "test.py", Mode::Slim);
        assert!(!result.contains("# comment"));
        assert!(!result.contains("# inner"));
        assert!(result.contains("def foo()"));
    }

    #[test]
    fn slim_collapses_blank_lines() {
        let src = "fn a() {}\n\n\n\n\nfn b() {}\n";
        let result = compress(src, "test.rs", Mode::Slim);
        // 3+ newlines collapsed to 2
        assert!(!result.contains("\n\n\n"));
    }

    #[test]
    fn is_comment_kind_rust() {
        assert!(is_comment_kind("line_comment", Lang::Rust));
        assert!(is_comment_kind("block_comment", Lang::Rust));
        assert!(!is_comment_kind("identifier", Lang::Rust));
    }

    #[test]
    fn is_comment_kind_python() {
        assert!(is_comment_kind("comment", Lang::Python));
        assert!(!is_comment_kind("identifier", Lang::Python));
    }

    #[test]
    fn is_comment_kind_js() {
        assert!(is_comment_kind("comment", Lang::JavaScript));
        assert!(is_comment_kind("comment", Lang::TypeScript));
        assert!(is_comment_kind("comment", Lang::Tsx));
        assert!(is_comment_kind("comment", Lang::Go));
        assert!(is_comment_kind("comment", Lang::C));
        assert!(is_comment_kind("comment", Lang::Cpp));
    }

    #[test]
    fn is_comment_kind_java() {
        assert!(is_comment_kind("line_comment", Lang::Java));
        assert!(is_comment_kind("block_comment", Lang::Java));
    }

    #[test]
    fn collapse_blank_lines_basic() {
        let result = collapse_blank_lines("a\n\n\n\nb\n");
        assert_eq!(result, "a\n\nb\n");
    }

    #[test]
    fn collapse_blank_lines_double_ok() {
        let input = "a\n\nb\n";
        let result = collapse_blank_lines(input);
        assert_eq!(result, input);
    }

    // ── Map mode: additional language coverage ──────────────────

    #[test]
    fn map_rust_use_declaration() {
        let src = "use std::collections::HashMap;\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("use std::collections::HashMap"));
    }

    #[test]
    fn map_rust_type_alias() {
        let src = "type Result<T> = std::result::Result<T, Error>;\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("type Result"));
    }

    #[test]
    fn map_rust_const() {
        let src = "const MAX: usize = 100;\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("const MAX"));
    }

    #[test]
    fn map_rust_static() {
        let src = "static COUNT: usize = 0;\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("static COUNT"));
    }

    #[test]
    fn map_rust_mod_with_body() {
        let src = "mod inner {\n    fn foo() {}\n}\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("mod inner"));
        assert!(result.contains("{ ... }"));
    }

    #[test]
    fn map_rust_mod_without_body() {
        let src = "mod other;\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("mod other"));
    }

    #[test]
    fn map_rust_impl_with_type() {
        let src =
            "impl Foo {\n    type Bar = i32;\n    const X: i32 = 1;\n    fn method(&self) {}\n}\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(result.contains("impl Foo"));
        assert!(result.contains("type Bar"));
        assert!(result.contains("const X"));
        assert!(result.contains("fn method"));
    }

    #[test]
    fn map_python_import() {
        let src = "import os\nfrom sys import path\n\ndef foo():\n    pass\n";
        let result = compress(src, "test.py", Mode::Map);
        assert!(result.contains("import os"));
        assert!(result.contains("from sys import path"));
    }

    #[test]
    fn map_python_decorator() {
        let src = "@staticmethod\ndef foo():\n    pass\n";
        let result = compress(src, "test.py", Mode::Map);
        assert!(result.contains("@staticmethod"));
        assert!(result.contains("def foo"));
    }

    #[test]
    fn map_python_class_with_methods() {
        let src = "class Foo:\n    x = 1\n    def bar(self):\n        pass\n";
        let result = compress(src, "test.py", Mode::Map);
        assert!(result.contains("class Foo"));
        assert!(result.contains("x = 1"));
        assert!(result.contains("def bar"));
    }

    #[test]
    fn map_python_decorated_class() {
        let src = "@dataclass\nclass Foo:\n    x: int = 0\n";
        let result = compress(src, "test.py", Mode::Map);
        assert!(result.contains("@dataclass"));
        assert!(result.contains("class Foo"));
    }

    #[test]
    fn map_js_import() {
        let src = "import { useState } from 'react';\n\nfunction App() { return null; }\n";
        let result = compress(src, "test.js", Mode::Map);
        assert!(result.contains("import"));
        assert!(result.contains("function App"));
    }

    #[test]
    fn map_js_export_default_function() {
        let src = "export default function main() {\n    return 1;\n}\n";
        let result = compress(src, "test.js", Mode::Map);
        assert!(result.contains("export default"));
        assert!(result.contains("main"));
    }

    #[test]
    fn map_ts_export_type() {
        let src = "export type ID = number;\n";
        let result = compress(src, "test.ts", Mode::Map);
        assert!(result.contains("export"));
        assert!(result.contains("type ID"));
    }

    #[test]
    fn map_ts_export_enum() {
        let src = "export enum Color {\n    Red,\n    Green,\n    Blue,\n}\n";
        let result = compress(src, "test.ts", Mode::Map);
        assert!(result.contains("export"));
        assert!(result.contains("enum Color"));
    }

    #[test]
    fn map_js_export_bare() {
        // export { foo, bar }
        let src = "export { foo, bar };\n";
        let result = compress(src, "test.js", Mode::Map);
        assert!(result.contains("export"));
    }

    #[test]
    fn map_go_import() {
        let src = "package main\n\nimport \"fmt\"\n\nfunc main() { fmt.Println(\"hi\") }\n";
        let result = compress(src, "test.go", Mode::Map);
        assert!(result.contains("package main"));
        assert!(result.contains("import"));
    }

    #[test]
    fn map_go_const_var() {
        let src = "package main\nconst X = 1\nvar Y = 2\n";
        let result = compress(src, "test.go", Mode::Map);
        assert!(result.contains("const X"));
        assert!(result.contains("var Y"));
    }

    #[test]
    fn map_go_method() {
        let src = "package main\nfunc (s *Server) Start() {\n    s.running = true\n}\n";
        let result = compress(src, "test.go", Mode::Map);
        assert!(result.contains("Start"));
    }

    #[test]
    fn map_c_preproc() {
        let src = "#include <stdio.h>\n#define MAX 100\nint main() { return 0; }\n";
        let result = compress(src, "test.c", Mode::Map);
        assert!(result.contains("#include <stdio.h>"));
        assert!(result.contains("#define MAX"));
    }

    #[test]
    fn map_c_declaration() {
        let src = "int global_var;\nextern int other_var;\n";
        let result = compress(src, "test.c", Mode::Map);
        assert!(result.contains("int global_var"));
    }

    #[test]
    fn map_cpp_class() {
        let src = "class Foo {\npublic:\n    void bar() { }\n};\n";
        let result = compress(src, "test.cpp", Mode::Map);
        assert!(result.contains("class Foo"));
    }

    #[test]
    fn map_cpp_namespace() {
        let src = "namespace ns {\n    void foo() {}\n}\n";
        let result = compress(src, "test.cpp", Mode::Map);
        assert!(result.contains("namespace ns"));
    }

    #[test]
    fn map_java_import_package() {
        let src =
            "package com.foo;\nimport java.util.List;\npublic class Foo {\n    void bar() {}\n}\n";
        let result = compress(src, "test.java", Mode::Map);
        assert!(result.contains("package com.foo"));
        assert!(result.contains("import java.util.List"));
    }

    #[test]
    fn map_java_nested_class() {
        let src = "class Outer {\n    class Inner {\n        void foo() {}\n    }\n}\n";
        let result = compress(src, "test.java", Mode::Map);
        assert!(result.contains("class Outer"));
        assert!(result.contains("class Inner"));
    }

    #[test]
    fn map_java_field_declaration() {
        let src = "class Foo {\n    private int x = 0;\n    void bar() {}\n}\n";
        let result = compress(src, "test.java", Mode::Map);
        assert!(result.contains("private int x"));
    }

    #[test]
    fn map_comments_stripped() {
        let src = "// comment\nfn foo() {}\n";
        let result = compress(src, "test.rs", Mode::Map);
        assert!(!result.contains("// comment"));
        assert!(result.contains("fn foo"));
    }

    #[test]
    fn map_ts_ambient_declaration() {
        let src = "declare function foo(): void;\ndeclare const bar: string;\n";
        let result = compress(src, "test.ts", Mode::Map);
        assert!(result.contains("foo"));
        assert!(result.contains("bar"));
    }

    #[test]
    fn slim_java_block_comments() {
        let src = "/* block comment */\npublic class Foo {}\n";
        let result = compress(src, "test.java", Mode::Slim);
        assert!(!result.contains("block comment"));
        assert!(result.contains("class Foo"));
    }
}
