//! Chunk truncation must never slice a character in half.
//!
//! `chunk_of` caps large symbols at ~4 KB by byte offset. Byte 4096 can land
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
    for c in &parsed.chunks {
        assert!(c.text.len() < src.len(), "chunk was truncated");
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
