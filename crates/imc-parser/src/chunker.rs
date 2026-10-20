//! cAST-style chunking: recursive split-then-merge over the syntax tree.
//!
//! Port of the method described in "cAST: Enhancing Code Retrieval-Augmented
//! Generation with Structural Chunking via Abstract Syntax Tree"
//! (arXiv 2506.15655, CMU + Augment Code):
//!
//!   1. walk the tree top-down — a node that fits the budget becomes one chunk,
//!   2. a node that does not fit is split into its children (recursively),
//!   3. adjacent small siblings are merged back together, greedily, so the
//!      index is not full of one-line fragments.
//!
//! The budget is counted in **non-whitespace characters**, like the paper: deep
//! indentation and comment blocks should not eat the budget.
//!
//! Why not the crate `astchunk` (a Rust port of the same paper)? It pins its
//! own `tree-sitter` version; this workspace already builds one tree-sitter
//! per grammar and a second version in the graph would mean parsing every
//! file twice. The algorithm is small enough to own.

use tree_sitter::Node;

use imc_core::types::Symbol;

/// The chunk budget in non-whitespace characters.
///
/// The paper measured retrieval and generation peaking around 2,000–2,500
/// non-whitespace characters per chunk.
pub const BUDGET_NWS: usize = 2000;

/// Non-whitespace characters — the size unit the cAST paper budgets in.
pub fn nws_len(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

/// A chunk before it is turned into a `RawChunk`.
pub struct Piece {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    /// The symbols whose definition starts inside this piece.
    pub symbols: Vec<String>,
}

/// Split a file into chunks.
///
/// `symbols` is the symbol list of the same file, used to label a chunk with
/// the symbol it belongs to (the search result shows it).
pub fn chunk_tree(
    root: Node<'_>,
    text: &str,
    symbols: &[Symbol],
    budget: usize,
) -> Vec<Piece> {
    let mut out = Vec::new();
    let children: Vec<Node<'_>> = root.named_children(&mut root.walk()).collect();
    if children.is_empty() {
        // A file with no top-level node (or an empty one): one chunk.
        if !text.trim().is_empty() {
            out.push(piece(root, text, symbols));
        }
        return out;
    }
    let mut pieces = Vec::new();
    for child in children {
        pieces.extend(split_node(child, text, symbols, budget));
    }
    out.extend(merge_siblings(pieces, text, symbols, budget));
    out
}

/// Split one node: it becomes a chunk when it fits, otherwise it is split into
/// its children.
fn split_node(node: Node<'_>, text: &str, symbols: &[Symbol], budget: usize) -> Vec<Piece> {
    let source = slice(text, node.start_byte(), node.end_byte());
    if nws_len(&source) <= budget {
        return vec![piece(node, text, symbols)];
    }

    let children: Vec<Node<'_>> = node.named_children(&mut node.walk()).collect();
    if children.is_empty() {
        // A leaf that is too big (a long statement, a big literal): split it
        // by lines. Truncating it would lose the tail of the code.
        return hard_split(node, text, symbols, budget);
    }

    let mut pieces = Vec::new();
    for child in children {
        pieces.extend(split_node(child, text, symbols, budget));
    }
    merge_siblings(pieces, text, symbols, budget)
}

/// Greedily merge adjacent pieces while they fit the budget.
fn merge_siblings(
    pieces: Vec<Piece>,
    text: &str,
    symbols: &[Symbol],
    budget: usize,
) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::with_capacity(pieces.len());
    let mut current: Option<Piece> = None;

    for piece in pieces {
        match current.take() {
            None => current = Some(piece),
            Some(mut acc) => {
                let merged = slice(text, acc.start_byte, piece.end_byte);
                if nws_len(&merged) <= budget {
                    acc.end_byte = piece.end_byte;
                    acc.end_line = piece.end_line;
                    for id in piece.symbols {
                        if !acc.symbols.contains(&id) {
                            acc.symbols.push(id);
                        }
                    }
                    current = Some(acc);
                } else {
                    out.push(acc);
                    current = Some(piece);
                }
            }
        }
    }
    if let Some(acc) = current {
        out.push(acc);
    }
    let _ = symbols;
    out
}

/// Split an oversized leaf by lines.
fn hard_split(node: Node<'_>, text: &str, symbols: &[Symbol], budget: usize) -> Vec<Piece> {
    let source = slice(text, node.start_byte(), node.end_byte());
    let mut out = Vec::new();
    let mut current = String::new();
    let mut start_line = node.start_position().row + 1;
    let line_no = node.start_position().row + 1;

    for (offset, line) in source.split_inclusive('\n').enumerate() {
        if current.is_empty() {
            start_line = line_no + offset;
        }
        if !current.is_empty() && nws_len(&current) + nws_len(line) > budget {
            out.push(Piece {
                start_byte: node.start_byte(),
                end_byte: node.start_byte() + current.len().min(source.len()),
                start_line,
                end_line: start_line + current.matches('\n').count().saturating_sub(1),
                symbols: symbols_at(symbols, node.start_byte(), node.end_byte()),
            });
            current.clear();
            start_line = line_no + offset;
        }
        current.push_str(line);
    }
    if !current.trim().is_empty() {
        out.push(Piece {
            start_byte: node.start_byte(),
            end_byte: node.start_byte() + current.len(),
            start_line,
            end_line: start_line + current.matches('\n').count().saturating_sub(1),
            symbols: symbols_at(symbols, node.start_byte(), node.end_byte()),
        });
    }
    out
}

/// One piece of source with the symbols that start inside it.
fn piece(node: Node<'_>, _text: &str, symbols: &[Symbol]) -> Piece {
    Piece {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        symbols: symbols_at(symbols, node.start_byte(), node.end_byte()),
    }
}

/// The ids of the symbols defined inside `[start, end)`.
fn symbols_at(symbols: &[Symbol], start: usize, end: usize) -> Vec<String> {
    let mut ids: Vec<String> = symbols
        .iter()
        .filter(|s| s.start_byte >= start && s.start_byte < end)
        .take(8)
        .map(|s| s.id.clone())
        .collect();
    ids.truncate(3);
    ids
}

/// Split a text into parts of at most `budget` non-whitespace characters,
/// cutting on line boundaries.
pub fn split_text_by_lines(text: &str, budget: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if !current.is_empty() && nws_len(&current) + nws_len(line) > budget {
            out.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(text.to_string());
    }
    out
}

pub fn slice(text: &str, start: usize, end: usize) -> String {
    let start = start.min(text.len());
    let end = end.min(text.len()).max(start);
    let mut start = start;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    let mut end = end;
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    text[start..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nws_ignores_whitespace() {
        assert_eq!(nws_len("a b\n\tc"), 3);
        assert_eq!(nws_len("   "), 0);
    }

    #[test]
    fn slice_is_char_boundary_safe() {
        let s = "héllo wörld";
        assert_eq!(slice(s, 0, 2), "hé".chars().take(2).collect::<String>());
        // A byte index inside a multi-byte character must not panic.
        let _ = slice(s, 1, 3);
    }
}
