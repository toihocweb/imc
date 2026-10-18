//! Port of the two large reader commands:
//!   main/cli/commands/find.py    (804) — token-optimised file reader
//!                                   (strip blanks, JSON smart-strip, truncation)
//!   main/cli/commands/context.py (697) — imc-context: symbol-anchored context
//!
//! Create `src/find.rs` and `src/context.rs`. Do not touch other crates.

/// Symbols-only outline result, shared by `find` modes.
pub mod find;
pub mod context;
