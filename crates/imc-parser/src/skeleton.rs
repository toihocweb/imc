//! AST Code Folding / Skeleton generator using tree-sitter.
//!
//! Generates an interface skeleton of a source file by folding function and
//! method bodies to `{ ... }` (or `: ...` in Python), while preserving
//! struct/class/interface/enum definitions, type signatures, and original line numbers.

use tree_sitter::Node;
use imc_core::ingest::language_of;

/// Represents a line in the generated skeleton, carrying its 1-based line number
/// in the original source file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkeletonLine {
    pub line_number: usize,
    pub content: String,
}

/// Check if a tree-sitter node represents a function or method definition.
fn is_function_node(node: Node<'_>, lang: &str) -> bool {
    let k = node.kind();
    match lang {
        "rust" => k == "function_item",
        "python" => k == "function_definition",
        "javascript" | "typescript" | "tsx" => {
            matches!(k, "function_declaration" | "method_definition" | "arrow_function")
        }
        "go" => matches!(k, "function_declaration" | "method_declaration"),
        "java" => matches!(k, "method_declaration" | "constructor_declaration"),
        _ => false,
    }
}

/// Fold replacement descriptor: start_byte, end_byte, and replacement string.
struct Fold {
    start_byte: usize,
    end_byte: usize,
    replacement: String,
}

fn collect_folds<'a>(
    node: Node<'a>,
    lang: &str,
    text: &str,
    bytes: &[u8],
    folds: &mut Vec<Fold>,
) {
    if is_function_node(node, lang) {
        if let Some(body) = node.child_by_field_name("body") {
            let start = body.start_byte();
            let end = body.end_byte();
            if end > start {
                let replacement = get_body_replacement(body, lang, bytes);
                folds.push(Fold {
                    start_byte: start,
                    end_byte: end,
                    replacement,
                });
                // Do not recurse into function bodies — they are folded away.
                return;
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_folds(child, lang, text, bytes, folds);
    }
}

fn get_body_replacement(body: Node<'_>, lang: &str, bytes: &[u8]) -> String {
    if lang == "python" {
        let indent = " ".repeat(body.start_position().column);
        // Check if the first statement in the block is a docstring
        if let Some(first_child) = body.child(0) {
            if first_child.kind() == "expression_statement" {
                if let Some(str_node) = first_child.child(0) {
                    if str_node.kind() == "string" {
                        let doc = std::str::from_utf8(&bytes[str_node.start_byte()..str_node.end_byte()])
                            .unwrap_or("");
                        return format!("{doc}\n{indent}...");
                    }
                }
            }
        }
        "...".to_string()
    } else {
        "{ ... }".to_string()
    }
}

/// Compute 1-based line number for any byte offset in the original text.
fn build_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn line_at_byte(starts: &[usize], byte_offset: usize) -> usize {
    match starts.binary_search(&byte_offset) {
        Ok(idx) => idx + 1,
        Err(idx) => idx,
    }
}

/// Generate skeleton lines for `rel` (project-relative path) with `text`.
/// Returns `None` if the language is unsupported or parsing fails.
pub fn generate_skeleton_lines(rel: &str, text: &str) -> Option<Vec<SkeletonLine>> {
    let lang = language_of(rel)?;
    if !crate::SUPPORTED.contains(&lang) {
        return None;
    }
    let mut parser = crate::parser_for(lang)?;
    let tree = parser.parse(text, None)?;
    let bytes = text.as_bytes();

    let mut folds = Vec::new();
    collect_folds(tree.root_node(), lang, text, bytes, &mut folds);

    if folds.is_empty() {
        // No functions/methods folded; emit original text with line numbers
        return Some(
            text.lines()
                .enumerate()
                .map(|(i, l)| SkeletonLine {
                    line_number: i + 1,
                    content: l.to_string(),
                })
                .collect(),
        );
    }

    // Sort folds by start_byte ascending
    folds.sort_by_key(|f| f.start_byte);

    let line_starts = build_line_starts(text);
    let mut chunks: Vec<(usize, String)> = Vec::new();
    let mut last_byte = 0;

    for fold in folds {
        if fold.start_byte > last_byte {
            let slice = &text[last_byte..fold.start_byte];
            chunks.push((last_byte, slice.to_string()));
        }
        chunks.push((fold.start_byte, fold.replacement));
        last_byte = fold.end_byte;
    }

    if last_byte < text.len() {
        let slice = &text[last_byte..];
        chunks.push((last_byte, slice.to_string()));
    }

    // Now assemble into lines while tracking original line numbers.
    let mut result_lines = Vec::new();
    let mut current_line = String::new();
    let mut current_line_num: Option<usize> = None;

    for (byte_offset, chunk) in chunks {
        let mut char_indices = chunk.char_indices().peekable();
        while let Some((c_idx, ch)) = char_indices.next() {
            if current_line_num.is_none() {
                current_line_num = Some(line_at_byte(&line_starts, byte_offset + c_idx));
            }
            if ch == '\n' {
                result_lines.push(SkeletonLine {
                    line_number: current_line_num.unwrap_or(1),
                    content: current_line.clone(),
                });
                current_line.clear();
                current_line_num = None;
            } else {
                current_line.push(ch);
            }
        }
    }

    if !current_line.is_empty() || current_line_num.is_some() {
        result_lines.push(SkeletonLine {
            line_number: current_line_num.unwrap_or(line_starts.len()),
            content: current_line,
        });
    }

    Some(result_lines)
}

/// Generate skeleton text with `LINE_NUM\tCONTENT` formatting.
pub fn generate_skeleton(rel: &str, text: &str) -> Option<String> {
    let lines = generate_skeleton_lines(rel, text)?;
    let formatted: Vec<String> = lines
        .into_iter()
        .map(|l| format!("{}\t{}", l.line_number, l.content))
        .collect();
    Some(formatted.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_rust_function_and_methods() {
        let code = r#"pub struct Greeter {
    greeting: String,
}

impl Greeter {
    pub fn new(greeting: &str) -> Self {
        Self {
            greeting: greeting.to_string(),
        }
    }

    pub fn greet(&self, name: &str) -> String {
        format!("{}, {}!", self.greeting, name)
    }
}

pub fn standalone(x: i32) -> i32 {
    let y = x * 2;
    y + 1
}
"#;
        let lines = generate_skeleton_lines("test.rs", code).expect("skeleton generated");
        let rendered: Vec<String> = lines.iter().map(|l| format!("{}\t{}", l.line_number, l.content)).collect();
        let full = rendered.join("\n");

        assert!(full.contains("pub fn new(greeting: &str) -> Self { ... }"));
        assert!(full.contains("pub fn greet(&self, name: &str) -> String { ... }"));
        assert!(full.contains("pub fn standalone(x: i32) -> i32 { ... }"));
        assert!(!full.contains("greeting.to_string()"));
        assert!(!full.contains("y + 1"));
        assert!(full.contains("pub struct Greeter {"));
    }

    #[test]
    fn folds_python_function_and_classes() {
        let code = r#"class OrderService:
    def __init__(self, db_conn):
        self.db = db_conn

    def process_order(self, order_id: int) -> bool:
        """Process an order by ID."""
        print("processing...")
        return True

def calculate_tax(amount: float) -> float:
    rate = 0.08
    return amount * rate
"#;
        let lines = generate_skeleton_lines("test.py", code).expect("skeleton generated");
        let rendered: Vec<String> = lines.iter().map(|l| format!("{}\t{}", l.line_number, l.content)).collect();
        let full = rendered.join("\n");

        assert!(full.contains("def __init__(self, db_conn):"));
        assert!(full.contains("def process_order(self, order_id: int) -> bool:"));
        assert!(full.contains(r#""""Process an order by ID.""""#));
        assert!(full.contains("def calculate_tax(amount: float) -> float:"));
        assert!(!full.contains("print(\"processing...\")"));
        assert!(!full.contains("rate = 0.08"));
    }

    #[test]
    fn folds_typescript_functions_and_classes() {
        let code = r#"export interface User {
    id: string;
    name: string;
}

export class UserService {
    private users: User[] = [];

    async fetchUser(id: string): Promise<User | null> {
        const found = this.users.find(u => u.id === id);
        return found || null;
    }
}

export function logUser(u: User): void {
    console.log(u.name);
}
"#;
        let lines = generate_skeleton_lines("service.ts", code).expect("skeleton generated");
        let rendered: Vec<String> = lines.iter().map(|l| format!("{}\t{}", l.line_number, l.content)).collect();
        let full = rendered.join("\n");

        assert!(full.contains("export interface User {"));
        assert!(full.contains("async fetchUser(id: string): Promise<User | null> { ... }"));
        assert!(full.contains("export function logUser(u: User): void { ... }"));
        assert!(!full.contains("console.log(u.name)"));
    }
}
