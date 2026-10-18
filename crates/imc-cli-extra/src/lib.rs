//! Port of the Claude Code integration surface of `main/cli/commands/*`.
//!
//! Scope: `imc trace` (AST call-hierarchy tracer), ported from
//! `main/cli/commands/trace.py` (89 lines) into `src/trace_cmd.rs`.
//!
//! The tracer itself (`main/core/tracer.py` → `imc-trace` crate) is still a
//! stub, so `trace_cmd` expresses the needed surface as a `Tracer` trait; the
//! orchestration logic (candidate resolution, callee/caller selection, JSON and
//! ASCII-tree output shaping, error messages/exit codes) is ported 1:1.

pub mod trace_cmd;