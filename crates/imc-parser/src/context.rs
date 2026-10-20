//! The context header a chunk carries.

/// The header a chunk carries: the qualified name, the signature line and the
/// first line of the docstring.
///
/// A chunk from the middle of a long function is otherwise unlabelled: the
/// agent sees `if prune:` with no idea which function it belongs to. The
/// context is a few dozen characters, so the embedding cost is negligible.
pub fn symbol_context_header(qualname: &str, body: &str, part: usize, parts: usize) -> String {
    let mut header = qualname.to_string();
    if parts > 1 {
        header.push_str(&format!(" (part {part}/{parts})"));
    }

    let mut lines = body.lines();
    let signature = lines.next().unwrap_or("").trim();
    let quote = "\"\"\"";
    let doc = body
        .lines()
        .nth(1)
        .map(str::trim)
        .filter(|l| l.starts_with(quote) || l.starts_with("'''"))
        .map(|l| {
            l.trim_matches(|c| c == '"' || c == '\'')
                .trim()
                .to_string()
        })
        .filter(|l| !l.is_empty());

    if !signature.is_empty() {
        header.push_str(" | ");
        header.push_str(&signature.chars().take(160).collect::<String>());
    }
    if let Some(doc) = doc {
        header.push_str(" | ");
        header.push_str(&doc.chars().take(160).collect::<String>());
    }
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_carries_signature_and_docstring() {
        let body = "def prune(self, root):\n    \"\"\"Delete stale memories.\"\"\"\n    return 1\n";
        let h = symbol_context_header("MemoryStore::prune", body, 2, 4);
        assert!(h.contains("MemoryStore::prune (part 2/4)"), "{h}");
        assert!(h.contains("def prune(self, root)"), "{h}");
        assert!(h.contains("Delete stale memories."), "{h}");
    }

    #[test]
    fn a_single_part_has_no_part_marker() {
        let h = symbol_context_header("f", "def f():\n    pass\n", 1, 1);
        assert!(!h.contains("part"), "{h}");
    }
}
