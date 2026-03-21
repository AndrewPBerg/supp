use super::{Lang, is_comment_kind, node_text, parse_source, slim};

// ── Map Mode ───────────────────────────────────────────────────────

pub(super) fn map(content: &str, lang: Lang) -> String {
    let tree = match parse_source(content, lang) {
        Some(t) => t,
        None => return slim(content, lang),
    };

    let root = tree.root_node();
    let mut out = String::new();
    let mut cursor = root.walk();

    if cursor.goto_first_child() {
        loop {
            let node = cursor.node();
            emit_node(content, node, lang, &mut out, 0);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    if out.ends_with("\n\n") {
        out.pop();
    }

    out
}

fn emit_node(source: &str, node: tree_sitter::Node, lang: Lang, out: &mut String, depth: usize) {
    let kind = node.kind();

    // Skip comments in map mode
    if is_comment_kind(kind, lang) {
        return;
    }

    match lang {
        Lang::Rust => emit_rust(source, node, out, depth),
        Lang::Python => emit_python(source, node, out, depth),
        Lang::JavaScript => emit_js(source, node, out, depth, false),
        Lang::TypeScript => emit_js(source, node, out, depth, true),
        Lang::Tsx => emit_js(source, node, out, depth, true),
        Lang::Go => emit_go(source, node, out, depth),
        Lang::C => emit_c(source, node, out, depth, false),
        Lang::Cpp => emit_c(source, node, out, depth, true),
        Lang::Java => emit_java(source, node, out, depth),
    }
}

fn indent(depth: usize) -> String {
    "    ".repeat(depth)
}

// ── Rust ───────────────────────────────────────────────────────────

fn emit_rust(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    let kind = node.kind();
    match kind {
        "use_declaration" | "type_item" | "const_item" | "static_item" => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        "mod_item" => {
            // If it has a body (declaration_list), show mod name { ... }
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" { ... }\n");
            } else {
                out.push_str(&indent(depth));
                out.push_str(node_text(source, node));
                out.push('\n');
            }
        }
        "function_item" => {
            emit_rust_fn_sig(source, node, out, depth);
        }
        "struct_item" => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" { ... }\n");
            } else {
                out.push_str(&indent(depth));
                out.push_str(node_text(source, node));
                out.push('\n');
            }
        }
        "enum_item" => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" { ... }\n");
            } else {
                out.push_str(&indent(depth));
                out.push_str(node_text(source, node));
                out.push('\n');
            }
        }
        "trait_item" => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" {\n");
                let mut cursor = body.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        if child.kind() == "function_item"
                            || child.kind() == "function_signature_item"
                        {
                            emit_rust_fn_sig(source, child, out, depth + 1);
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
                out.push_str(&indent(depth));
                out.push_str("}\n");
            }
        }
        "impl_item" => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" {\n");
                let mut cursor = body.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        match child.kind() {
                            "function_item" => {
                                emit_rust_fn_sig(source, child, out, depth + 1);
                            }
                            "type_item" | "const_item" => {
                                out.push_str(&indent(depth + 1));
                                out.push_str(node_text(source, child));
                                out.push('\n');
                            }
                            _ => {}
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
                out.push_str(&indent(depth));
                out.push_str("}\n");
            }
        }
        "attribute_item" | "inner_attribute_item" => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        "macro_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                out.push_str(&indent(depth));
                out.push_str(&format!(
                    "macro_rules! {} {{ ... }}\n",
                    node_text(source, name)
                ));
            }
        }
        _ => {}
    }
}

fn emit_rust_fn_sig(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    if let Some(body) = node.child_by_field_name("body") {
        let before = &source[node.start_byte()..body.start_byte()];
        out.push_str(&indent(depth));
        out.push_str(before.trim_end());
        out.push_str(" { ... }\n");
    } else {
        out.push_str(&indent(depth));
        out.push_str(node_text(source, node));
        out.push('\n');
    }
}

// ── Python ─────────────────────────────────────────────────────────

fn emit_python(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    let kind = node.kind();
    match kind {
        "import_statement" | "import_from_statement" | "expression_statement" => {
            if depth == 0 || kind.starts_with("import") {
                out.push_str(&indent(depth));
                out.push_str(node_text(source, node));
                out.push('\n');
            }
        }
        "function_definition" => {
            emit_python_fn_sig(source, node, out, depth);
        }
        "class_definition" => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push('\n');
                let mut cursor = body.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        match child.kind() {
                            "function_definition" => {
                                emit_python_fn_sig(source, child, out, depth + 1);
                            }
                            "decorated_definition" => {
                                emit_python_decorated(source, child, out, depth + 1);
                            }
                            "expression_statement" => {
                                out.push_str(&indent(depth + 1));
                                out.push_str(node_text(source, child));
                                out.push('\n');
                            }
                            _ => {}
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "decorated_definition" => {
            emit_python_decorated(source, node, out, depth);
        }
        _ => {}
    }
}

fn emit_python_fn_sig(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    if let Some(body) = node.child_by_field_name("body") {
        let before = &source[node.start_byte()..body.start_byte()];
        out.push_str(&indent(depth));
        out.push_str(before.trim_end());
        out.push_str(" ...\n");
    } else {
        out.push_str(&indent(depth));
        out.push_str(node_text(source, node));
        out.push('\n');
    }
}

fn emit_python_decorated(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            match child.kind() {
                "decorator" => {
                    out.push_str(&indent(depth));
                    out.push_str(node_text(source, child));
                    out.push('\n');
                }
                "function_definition" => {
                    emit_python_fn_sig(source, child, out, depth);
                }
                "class_definition" => {
                    emit_python(source, child, out, depth);
                }
                _ => {}
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

// ── JavaScript / TypeScript ────────────────────────────────────────

fn emit_js(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize, is_ts: bool) {
    let kind = node.kind();
    match kind {
        "import_statement" => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        "function_declaration" => {
            emit_js_fn(source, node, out, depth);
        }
        "class_declaration" => {
            emit_js_class(source, node, out, depth);
        }
        "export_statement" => {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                let mut found_decl = false;
                loop {
                    let child = cursor.node();
                    match child.kind() {
                        "function_declaration"
                        | "class_declaration"
                        | "lexical_declaration"
                        | "variable_declaration" => {
                            let prefix = if node_text(source, node).starts_with("export default") {
                                "export default "
                            } else {
                                "export "
                            };
                            let mut sub_out = String::new();
                            emit_js(source, child, &mut sub_out, depth, is_ts);
                            if !sub_out.is_empty() {
                                let trimmed = sub_out.trim_start();
                                out.push_str(&indent(depth));
                                out.push_str(prefix);
                                out.push_str(trimmed);
                                found_decl = true;
                            }
                        }
                        "interface_declaration" | "type_alias_declaration" | "enum_declaration" => {
                            let prefix = "export ";
                            let mut sub_out = String::new();
                            emit_js(source, child, &mut sub_out, depth, true);
                            if !sub_out.is_empty() {
                                let trimmed = sub_out.trim_start();
                                out.push_str(&indent(depth));
                                out.push_str(prefix);
                                out.push_str(trimmed);
                                found_decl = true;
                            }
                        }
                        _ => {}
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
                if !found_decl {
                    out.push_str(&indent(depth));
                    out.push_str(node_text(source, node));
                    out.push('\n');
                }
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        "interface_declaration" if is_ts => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" { ... }\n");
            } else {
                out.push_str(&indent(depth));
                out.push_str(node_text(source, node));
                out.push('\n');
            }
        }
        "type_alias_declaration" if is_ts => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        "enum_declaration" if is_ts => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" { ... }\n");
            } else {
                out.push_str(&indent(depth));
                out.push_str(node_text(source, node));
                out.push('\n');
            }
        }
        "method_definition" => {
            emit_js_method(source, node, out, depth);
        }
        "ambient_declaration" if is_ts => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        _ => {}
    }
}

fn emit_js_fn(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    if let Some(body) = node.child_by_field_name("body") {
        let before = &source[node.start_byte()..body.start_byte()];
        out.push_str(&indent(depth));
        out.push_str(before.trim_end());
        out.push_str(" { ... }\n");
    } else {
        out.push_str(&indent(depth));
        out.push_str(node_text(source, node));
        out.push('\n');
    }
}

fn emit_js_method(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    if let Some(body) = node.child_by_field_name("body") {
        let before = &source[node.start_byte()..body.start_byte()];
        out.push_str(&indent(depth));
        out.push_str(before.trim_end());
        out.push_str(" { ... }\n");
    }
}

fn emit_js_class(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    if let Some(body) = node.child_by_field_name("body") {
        let before = &source[node.start_byte()..body.start_byte()];
        out.push_str(&indent(depth));
        out.push_str(before.trim_end());
        out.push_str(" {\n");
        let mut cursor = body.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                match child.kind() {
                    "method_definition" => {
                        emit_js_method(source, child, out, depth + 1);
                    }
                    "public_field_definition" | "field_definition" => {
                        out.push_str(&indent(depth + 1));
                        out.push_str(node_text(source, child));
                        out.push('\n');
                    }
                    _ => {}
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        out.push_str(&indent(depth));
        out.push_str("}\n");
    }
}

// ── Go ─────────────────────────────────────────────────────────────

fn emit_go(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    let kind = node.kind();
    match kind {
        "import_declaration" => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        "package_clause" => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        "function_declaration" | "method_declaration" => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" { ... }\n");
            } else {
                out.push_str(&indent(depth));
                out.push_str(node_text(source, node));
                out.push('\n');
            }
        }
        "type_declaration" => {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "type_spec" {
                        emit_go_type_spec(source, child, out, depth);
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "const_declaration" | "var_declaration" => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        _ => {}
    }
}

fn emit_go_type_spec(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    let mut cursor = node.walk();
    let mut has_field_list = false;
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if (child.kind() == "struct_type" || child.kind() == "interface_type")
                && let Some(_body) = child.child_by_field_name("body").or_else(|| {
                    let mut c2 = child.walk();
                    if c2.goto_first_child() {
                        loop {
                            if matches!(
                                c2.node().kind(),
                                "field_declaration_list" | "method_spec_list" | "{"
                            ) {
                                return Some(c2.node());
                            }
                            if !c2.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                    None
                })
            {
                let before = &source[node.start_byte()..child.start_byte()];
                let keyword = match child.kind() {
                    "struct_type" => "struct",
                    "interface_type" => "interface",
                    _ => child.kind(),
                };
                out.push_str(&indent(depth));
                out.push_str(&format!("type {}{} {{ ... }}\n", before.trim(), keyword));
                has_field_list = true;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    if !has_field_list {
        out.push_str(&indent(depth));
        out.push_str(&format!("type {}\n", node_text(source, node)));
    }
}

// ── C / C++ ────────────────────────────────────────────────────────

fn emit_c(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize, is_cpp: bool) {
    let kind = node.kind();
    match kind {
        "preproc_include" | "preproc_def" | "preproc_ifdef" | "preproc_ifndef" => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        "declaration" => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        "function_definition" => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" { ... }\n");
            } else {
                out.push_str(&indent(depth));
                out.push_str(node_text(source, node));
                out.push('\n');
            }
        }
        "struct_specifier" | "enum_specifier" => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" { ... }\n");
            } else {
                out.push_str(&indent(depth));
                out.push_str(node_text(source, node));
                out.push('\n');
            }
        }
        "class_specifier" if is_cpp => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" { ... }\n");
            }
        }
        "namespace_definition" if is_cpp => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" {\n");
                let mut cursor = body.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        emit_c(source, child, out, depth + 1, is_cpp);
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
                out.push_str(&indent(depth));
                out.push_str("}\n");
            }
        }
        _ => {}
    }
}

// ── Java ───────────────────────────────────────────────────────────

fn emit_java(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    let kind = node.kind();
    match kind {
        "import_declaration" | "package_declaration" => {
            out.push_str(&indent(depth));
            out.push_str(node_text(source, node));
            out.push('\n');
        }
        "class_declaration" | "interface_declaration" | "enum_declaration" => {
            if let Some(body) = node.child_by_field_name("body") {
                let before = &source[node.start_byte()..body.start_byte()];
                out.push_str(&indent(depth));
                out.push_str(before.trim_end());
                out.push_str(" {\n");
                let mut cursor = body.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        match child.kind() {
                            "method_declaration" | "constructor_declaration" => {
                                emit_java_method(source, child, out, depth + 1);
                            }
                            "field_declaration" => {
                                out.push_str(&indent(depth + 1));
                                out.push_str(node_text(source, child));
                                out.push('\n');
                            }
                            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                                emit_java(source, child, out, depth + 1);
                            }
                            _ => {}
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
                out.push_str(&indent(depth));
                out.push_str("}\n");
            }
        }
        _ => {}
    }
}

fn emit_java_method(source: &str, node: tree_sitter::Node, out: &mut String, depth: usize) {
    if let Some(body) = node.child_by_field_name("body") {
        let before = &source[node.start_byte()..body.start_byte()];
        out.push_str(&indent(depth));
        out.push_str(before.trim_end());
        out.push_str(" { ... }\n");
    } else {
        out.push_str(&indent(depth));
        out.push_str(node_text(source, node));
        out.push('\n');
    }
}
