//! Symbol coverage for declaration forms that are not one AST node kind.
//!
//! Each case here was measured against the Python original
//! (`imc read <file> --symbols` on both binaries): where Python indexes a
//! declaration, the port must too, or `trace`/`search`/`context` silently lose
//! it. Where Python does *not* index one, the port must not invent it either.

use imc_parser::parse;

fn names(rel: &str, text: &str) -> Vec<String> {
    parse(rel, text)
        .expect("parse")
        .symbols
        .into_iter()
        .map(|s| s.name)
        .collect()
}

fn qualnames(rel: &str, text: &str) -> Vec<String> {
    parse(rel, text)
        .expect("parse")
        .symbols
        .into_iter()
        .map(|s| s.qualname)
        .collect()
}

#[test]
fn ts_arrow_consts_are_symbols_regardless_of_case() {
    let src = "const plain = () => 1;\n\
               export const exported = () => 2;\n\
               export const fn = function () { return 3; };\n\
               const Widget = () => null;\n\
               const notAFunction = 5;\n";
    let got = names("a.ts", src);
    for want in ["plain", "exported", "fn", "Widget"] {
        assert!(got.contains(&want.to_string()), "{want} missing from {got:?}");
    }
    // A const holding a value is not a callable declaration.
    assert!(!got.contains(&"notAFunction".to_string()), "{got:?}");
}

#[test]
fn ts_nested_arrow_const_is_not_a_top_level_symbol() {
    // Python indexes `outer` and `topHelper`, never `inner`.
    let src = "export function outer() {\n\
               \x20 const inner = () => 1;\n\
               \x20 return inner;\n\
               }\n\
               const topHelper = () => 2;\n";
    let got = names("b.ts", src);
    assert!(got.contains(&"outer".to_string()), "{got:?}");
    assert!(got.contains(&"topHelper".to_string()), "{got:?}");
    assert!(!got.contains(&"inner".to_string()), "nested arrow leaked: {got:?}");
}

#[test]
fn rust_impl_block_is_not_a_symbol_but_prefixes_its_methods() {
    // The impl's qualname is the type name, so emitting it collides with the
    // struct on `file::Pair` and the struct loses its primary key.
    let src = "pub struct Pair {\n    a: u32,\n}\n\n\
               impl Pair {\n    pub fn new(a: u32) -> Self {\n        Pair { a }\n    }\n}\n";
    let parsed = parse("src/lib.rs", src).expect("parse");
    let names: Vec<String> = parsed.symbols.iter().map(|s| s.name.clone()).collect();
    let kinds: Vec<String> = parsed
        .symbols
        .iter()
        .map(|s| s.kind.as_str().to_string())
        .collect();
    assert!(
        !kinds.iter().any(|k| k == "impl"),
        "impl block must not be a symbol: {names:?} {kinds:?}"
    );
    assert_eq!(names, vec!["Pair".to_string(), "new".to_string()], "{names:?}");
    // The method keeps the type as its qualname prefix, as Python reports it.
    assert!(qualnames("src/lib.rs", src).contains(&"Pair::new".to_string()));
    // And the struct keeps the identity the methods hang off.
    let pair = parsed.symbols.iter().find(|s| s.name == "Pair").unwrap();
    assert_eq!(pair.line, 1, "the struct definition must be the `Pair` symbol");
}
