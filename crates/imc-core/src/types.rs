use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Interface,
    Type,
    Enum,
    Struct,
    Trait,
    Impl,
    Module,
    Component,
    Variable,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Type => "type",
            SymbolKind::Enum => "enum",
            SymbolKind::Struct => "struct",
            SymbolKind::Trait => "trait",
            SymbolKind::Impl => "impl",
            SymbolKind::Module => "module",
            SymbolKind::Component => "component",
            SymbolKind::Variable => "variable",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "method" => SymbolKind::Method,
            "class" => SymbolKind::Class,
            "interface" => SymbolKind::Interface,
            "type" => SymbolKind::Type,
            "enum" => SymbolKind::Enum,
            "struct" => SymbolKind::Struct,
            "trait" => SymbolKind::Trait,
            "impl" => SymbolKind::Impl,
            "module" => SymbolKind::Module,
            "component" => SymbolKind::Component,
            "variable" => SymbolKind::Variable,
            _ => SymbolKind::Function,
        }
    }

    /// Definition types that enrich search results with AST evidence.
    pub fn is_definition(self) -> bool {
        matches!(
            self,
            SymbolKind::Function
                | SymbolKind::Method
                | SymbolKind::Class
                | SymbolKind::Component
                | SymbolKind::Type
                | SymbolKind::Enum
                | SymbolKind::Struct
                | SymbolKind::Trait
                | SymbolKind::Interface
                | SymbolKind::Impl
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    /// `file::qualified::name` — stable across runs.
    pub id: String,
    pub name: String,
    pub qualname: String,
    pub kind: SymbolKind,
    /// Project-relative path.
    pub file: String,
    pub line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub parent: Option<String>,
}

impl Symbol {
    pub fn make_id(file: &str, qualname: &str) -> String {
        format!("{file}::{qualname}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Call,
    Import,
    Extends,
    Implements,
    Uses,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Call => "call",
            EdgeKind::Import => "import",
            EdgeKind::Extends => "extends",
            EdgeKind::Implements => "implements",
            EdgeKind::Uses => "uses",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "import" => EdgeKind::Import,
            "extends" => EdgeKind::Extends,
            "implements" => EdgeKind::Implements,
            "uses" => EdgeKind::Uses,
            _ => EdgeKind::Call,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub src: String,
    /// Call edges keep the raw callee name; resolution to a symbol happens at query time.
    pub dst: String,
    pub kind: EdgeKind,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: i64,
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
    pub sym_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub score: f64,
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    /// Raw chunk text (full symbol source or window).
    pub text: String,
    /// Contextual excerpt around the matched region.
    pub snippet: String,
    pub sym_name: Option<String>,
    pub sym_kind: Option<String>,
    pub source: &'static str,
}