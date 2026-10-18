//! Port of `main/proxy/lib/types/**` — one module per Python file.
//!
//! Porting notes that apply to all four modules:
//!
//! * Field names keep the Python spelling exactly, so a sibling crate can
//!   translate call sites 1:1. Python's `Literal[...]` annotations become
//!   `String` fields with the accepted values documented on each — the proxy
//!   forwards these payloads without validating them, and keeping them `String`
//!   avoids dropping values we haven't enumerated yet.
//! * Python `Optional[X] = None` becomes `Option<X>`, with
//!   `skip_serializing_if = "Option::is_none"` so serialization matches
//!   pydantic's `exclude_none` behaviour used everywhere in this tree.
//! * Python `extra="allow"` becomes `#[serde(flatten)] extra: Map<String, Value>`.
//! * Python `Union[...]` becomes an untagged enum with the union's own name.

pub mod anthropic;
pub mod chat_completions;
pub mod models;
pub mod responses;