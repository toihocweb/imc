//! Port of `main/core/tracer.py` (671 lines) — AST call-hierarchy tracer:
//! forward call trees (callees), backward call trees (callers / blast radius),
//! builtin filtering, and the high-signal vs `--all` modes.
//!
//! Python uses NetworkX; use `imc_core::store::Store` (tables `symbols` and
//! `edges`) instead — it already holds call edges as (src, dst, kind, file, line).
//!
//! Modules:
//!   - [`builtins`] — `COMMON_BUILTINS` / `COMMON_POJO_ACCESSORS` (verbatim) and
//!     the high-signal filter used when `--all` is absent.
//!   - [`tree`] — `Tracer` (forward/backward trees over `Store`),
//!     `TraceOptions` (`max_depth` / `show_all`), serde output nodes, and the
//!     `RouteBridge` hook for cross-boundary HTTP route tracing.
//!
//! ```no_run
//! use imc_core::store;
//! use imc_trace::{Tracer, TraceOptions};
//! # fn main() -> anyhow::Result<()> {
//! let store = store::open(std::path::Path::new("/tmp/project.db"))?;
//! let tracer = Tracer::new(&store);
//! for target in tracer.resolve_target("run_find", None) {
//!     let tree = tracer.trace_callees(&target.id, &TraceOptions::default());
//!     println!("{}", imc_trace::format_callee_tree(&tree));
//! }
//! # Ok(())
//! # }
//! ```

pub mod builtins;
pub mod tree;

pub use builtins::{is_builtin, is_trivial_accessor, should_filter};
pub use tree::{
    format_callee_tree, format_caller_tree, CalleeNode, ClientCall, CallerNode, HttpRoute, NoBridge,
    ResolvedTarget, RouteBridge, TraceOptions, Tracer,
};
