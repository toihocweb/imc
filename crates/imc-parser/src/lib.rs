//! Tree-sitter based AST extraction: symbols, call sites, import edges and
//! AST-scoped chunks. This replaces `main/ingest/parser.py` (extraction half)
//! and `main/indexing/graph.py` (graph half) of the Python implementation.

use tree_sitter::{Node, Parser};

use imc_core::ingest::language_of;
use imc_core::types::{Edge, EdgeKind, Symbol, SymbolKind};

pub const SUPPORTED: &[&str] = &[
    "python", "javascript", "typescript", "tsx", "rust", "go",
];

pub struct ParsedFile {
    pub symbols: Vec<Symbol>,
    pub edges: Vec<Edge>,
    pub chunks: Vec<RawChunk>,
}

#[derive(Debug, Clone)]
pub struct RawChunk {
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
    pub sym_id: Option<String>,
}

fn parser_for(lang: &str) -> Option<Parser> {
    let mut p = Parser::new();
    let ok = match lang {
        "python" => p.set_language(&tree_sitter_python::LANGUAGE.into()).is_ok(),
        "javascript" => p.set_language(&tree_sitter_javascript::LANGUAGE.into()).is_ok(),
        "typescript" => p
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .is_ok(),
        "tsx" => p
            .set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
            .is_ok(),
        "rust" => p.set_language(&tree_sitter_rust::LANGUAGE.into()).is_ok(),
        "go" => p.set_language(&tree_sitter_go::LANGUAGE.into()).is_ok(),
        _ => false,
    };
    if ok {
        Some(p)
    } else {
        None
    }
}

/// Parse `text` for `rel` (project-relative path). Returns `None` when the
/// language has no tree-sitter grammar wired up.
pub fn parse(rel: &str, text: &str) -> Option<ParsedFile> {
    let lang = language_of(rel)?;
    if !SUPPORTED.contains(&lang) {
        return None;
    }
    let mut parser = parser_for(lang)?;
    let tree = parser.parse(text, None)?;
    let bytes = text.as_bytes();

    let mut out = ParsedFile {
        symbols: Vec::new(),
        edges: Vec::new(),
        chunks: Vec::new(),
    };
    walk(tree.root_node(), rel, bytes, text, lang, &mut out, None, None);
    Some(out)
}

fn walk(
    node: Node<'_>,
    rel: &str,
    bytes: &[u8],
    text: &str,
    lang: &str,
    out: &mut ParsedFile,
    parent_qualified: Option<&str>,
    parent_id: Option<&str>,
) {
    // Explicit stack keeps deep trees well inside the thread stack limit.
    let mut stack: Vec<(Node<'_>, Option<String>, Option<String>)> =
        vec![(node, parent_qualified.map(String::from), parent_id.map(String::from))];

    while let Some((n, pq, pid)) = stack.pop() {
        if let Some(sym) = maybe_symbol(n, rel, bytes, text, lang, pq.as_deref()) {
            out.chunks.push(chunk_of(&sym, bytes, text));
            let qual = sym.qualname.clone();
            let id = sym.id.clone();
            out.symbols.push(sym);
            for i in (0..n.child_count()).rev() {
                if let Some(c) = n.child(i) {
                    stack.push((c, Some(qual.clone()), Some(id.clone())));
                }
            }
            continue;
        }

        if let Some(edge) = maybe_edge(n, bytes, text, lang, pid.as_deref()) {
            out.edges.push(edge);
        }

        for i in (0..n.child_count()).rev() {
            if let Some(c) = n.child(i) {
                stack.push((c, pq.clone(), pid.clone()));
            }
        }
    }
}


fn node_text<'a>(node: Node<'_>, bytes: &'a [u8]) -> &'a str {
    node.utf8_text(bytes).unwrap_or("")
}

fn make_symbol(
    node: Node<'_>,
    name: &str,
    kind: SymbolKind,
    qualname: &str,
    rel: &str,
    parent: Option<String>,
) -> Symbol {
    let start = node.start_position();
    let end = node.end_position();
    Symbol {
        id: Symbol::make_id(rel, qualname),
        name: name.to_string(),
        qualname: qualname.to_string(),
        kind,
        file: rel.to_string(),
        line: start.row + 1,
        end_line: end.row + 1,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        parent,
    }
}

fn chunk_of(sym: &Symbol, bytes: &[u8], text: &str) -> RawChunk {
    let end = sym.end_byte.min(bytes.len());
    let start = sym.start_byte.min(end);
    let mut body = &text[start..end];
    // Cap very large symbols at ~4 KB to keep embedding batches sane.
    if body.len() > 4096 {
        if let Some(cut) = body[4096..].rfind('\n') {
            body = &body[..4096 + cut];
        } else {
            body = &body[..4096];
        }
    }
    RawChunk {
        start_line: sym.line,
        end_line: sym.end_line,
        text: format!("{} {}", sym.qualname, body.trim_end()),
        sym_id: Some(sym.id.clone()),
    }
}

fn field<'a>(node: Node<'_>, name: &str, bytes: &'a [u8]) -> Option<&'a str> {
    node.child_by_field_name(name)
        .map(|n| node_text(n, bytes))
        .filter(|s| !s.is_empty())
}

fn maybe_symbol<'a>(
    n: Node<'_>,
    rel: &str,
    bytes: &'a [u8],
    _text: &str,
    lang: &str,
    parent_qualified: Option<&str>,
) -> Option<Symbol> {
    let kind_name = n.kind();
    let qualified = |base: &str, own: &str| match parent_qualified {
        Some(p) => format!("{p}::{own}"),
        None => {
            let _ = base;
            own.to_string()
        }
    };

    let (name, kind) = match (lang, kind_name) {
        ("python", "function_definition") => {
            (field(n, "name", bytes)?, SymbolKind::Function)
        }
        ("python", "class_definition") => (field(n, "name", bytes)?, SymbolKind::Class),
        ("javascript" | "typescript" | "tsx", "function_declaration") => {
            (field(n, "name", bytes)?, SymbolKind::Function)
        }
        ("javascript" | "typescript" | "tsx", "method_definition") => {
            (field(n, "name", bytes)?, SymbolKind::Method)
        }
        ("javascript" | "typescript" | "tsx", "class_declaration") => {
            (field(n, "name", bytes)?, SymbolKind::Class)
        }
        ("typescript" | "tsx", "interface_declaration") => {
            (field(n, "name", bytes)?, SymbolKind::Interface)
        }
        ("typescript" | "tsx", "type_alias_declaration") => {
            (field(n, "name", bytes)?, SymbolKind::Type)
        }
        ("typescript" | "tsx", "enum_declaration") => (field(n, "name", bytes)?, SymbolKind::Enum),
        ("rust", "function_item") => (field(n, "name", bytes)?, SymbolKind::Function),
        ("rust", "struct_item") => (field(n, "name", bytes)?, SymbolKind::Struct),
        ("rust", "enum_item") => (field(n, "name", bytes)?, SymbolKind::Enum),
        ("rust", "trait_item") => (field(n, "name", bytes)?, SymbolKind::Trait),
        ("rust", "impl_item") => {
            let type_node = n.child_by_field_name("type")?;
            let own = match type_node.kind() {
                "generic_type" | "type_identifier" => node_text(type_node, bytes),
                _ => node_text(type_node, bytes),
            };
            if own.is_empty() {
                return None;
            }
            (own, SymbolKind::Impl)
        }
        ("rust", "mod_item") => (field(n, "name", bytes)?, SymbolKind::Module),
        ("go", "function_declaration") => (field(n, "name", bytes)?, SymbolKind::Function),
        ("go", "method_declaration") => (field(n, "name", bytes)?, SymbolKind::Method),
        ("go", "type_declaration") => {
            // Go groups named types: `type ( A struct{...} )`
            return None;
        }
        ("go", "type_spec") => {
            let name = field(n, "name", bytes)?;
            let parent_type = n.parent().map(|p| p.kind()) == Some("type_declaration");
            let is_struct = n
                .child_by_field_name("type")
                .map(|t| t.kind() == "struct_type")
                .unwrap_or(false);
            let kind = if is_struct {
                SymbolKind::Struct
            } else {
                SymbolKind::Type
            };
            let _ = parent_type;
            (name, kind)
        }
        ("javascript" | "typescript" | "tsx", "lexical_declaration") => {
            // `const Foo = () => {}` / `const Foo = function () {}` — components.
            let mut it = n.walk();
            for child in n.children(&mut it) {
                if child.kind() != "variable_declarator" {
                    continue;
                }
                let name = field(child, "name", bytes)?;
                let value = child.child_by_field_name("value")?;
                let is_fn = matches!(
                    value.kind(),
                    "arrow_function" | "function_expression" | "function_declaration"
                );
                let upper = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                if is_fn && upper {
                    let qual = qualified(rel, name);
                    return Some(make_symbol(child, name, SymbolKind::Component, &qual, rel, None));
                }
            }
            return None;
        }
        _ => return None,
    };

    if name.is_empty() {
        return None;
    }
    let qual = qualified(rel, name);
    Some(make_symbol(n, name, kind, &qual, rel, None))
}

/// Extract call/import edges from a node.
fn maybe_edge(n: Node<'_>, bytes: &[u8], _text: &str, lang: &str, parent_id: Option<&str>) -> Option<Edge> {
    let src = parent_id?;
    let start = n.start_position().row + 1;
    let file_line_hint = src.split("::").next().unwrap_or("").to_string();

    match (lang, n.kind()) {
        (_, "call" | "call_expression") => {
            let callee = n.child_by_field_name("function")?;
            let name = match callee.kind() {
                "identifier" | "field_expression" | "member_expression" | "property_access"
                | "scoped_identifier" => {
                    let raw = node_text(callee, bytes);
                    // Keep the right-most segment: `foo.bar.baz()` → `baz`.
                    raw.rsplit(&['.', ':'][..]).next().unwrap_or(raw)
                }
                _ => return None,
            };
            if name.is_empty() {
                return None;
            }
            Some(Edge {
                src: src.to_string(),
                dst: name.to_string(),
                kind: EdgeKind::Call,
                file: file_line_hint,
                line: start,
            })
        }
        ("python", "import_statement") | ("python", "import_from_statement") => {
            Some(Edge {
                src: src.to_string(),
                dst: node_text(n, bytes).to_string(),
                kind: EdgeKind::Import,
                file: file_line_hint,
                line: start,
            })
        }
        ("rust", "use_declaration") | ("go", "import_declaration") => Some(Edge {
            src: src.to_string(),
            dst: node_text(n, bytes).to_string(),
            kind: EdgeKind::Import,
            file: file_line_hint,
            line: start,
        }),
        ("rust", "mod_item") if n.child_by_field_name("name").is_some() => None,
        _ => None,
    }
}