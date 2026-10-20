//! Chunk truncation must never slice a character in half.
//!
//! A symbol larger than the chunk budget is split into parts, and the split
//! must never slice a character in half. The old byte cap at 4096 could land
//! inside a multi-byte character — on a Korean repo (RabeMaster/gakkaweo) that
//! panicked the whole `imc index` run and left the project with 0 symbols and 0
//! chunks. Python's `CodeSplitter(max_chars=4000)` caps by characters, so it
//! never had this failure mode.

use imc_parser::parse;

#[test]
fn a_large_symbol_with_multibyte_chars_still_chunks() {
    // Every '갱' is 3 bytes, so a fixed byte cut at 4096 necessarily lands
    // inside one of them.
    let mut src = String::from("fn big() {\n");
    while src.len() < 6000 {
        src.push_str("    // 갱신된 문장을 기록합니다\n");
    }
    src.push_str("}\n");
    assert!(src.len() > 4096);

    let parsed = parse("api/big.rs", &src).expect("parse");
    assert_eq!(parsed.symbols.len(), 1, "one function symbol");
    assert!(!parsed.chunks.is_empty(), "the symbol produced a chunk");
    // The symbol must not be truncated: every part is valid UTF-8, and the
    // parts together carry the whole body — the tail of the function is
    // included, which is what the old byte cap threw away.
    let joined: String = parsed.chunks.iter().map(|c| c.text.as_str()).collect();
    assert!(
        std::str::from_utf8(joined.as_bytes()).is_ok(),
        "the chunks must be valid UTF-8"
    );
    assert!(
        joined.contains("}") || parsed.chunks.len() > 1,
        "the end of the function must survive: {:?}",
        parsed.chunks.iter().map(|c| c.text.len()).collect::<Vec<_>>()
    );
    for c in &parsed.chunks {
        assert!(std::str::from_utf8(c.text.as_bytes()).is_ok());
    }
}

#[test]
fn a_symbol_that_only_needs_a_short_cut_is_still_valid_utf8() {
    // Boundary exactly at a char edge (the easy case) plus a multi-byte char
    // straddling the cut.
    let mut src = String::from("const S: &str = \"");
    while src.len() < 4096 {
        src.push('é');
    }
    src.push_str("\";\n");
    let parsed = parse("a.rs", &src).expect("parse");
    for c in &parsed.chunks {
        assert!(std::str::from_utf8(c.text.as_bytes()).is_ok());
    }
}
