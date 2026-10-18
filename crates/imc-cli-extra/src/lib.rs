//! Port of the Claude Code integration surface of `main/cli/commands/*`.
//!
//! Scope: `imc trace` (AST call-hierarchy tracer), ported from
//! `main/cli/commands/trace.py` (89 lines) into `src/trace_cmd.rs`.
//!
//! `trace_cmd` is the faithful port of Python's `main/cli/commands/trace.py`
//! (candidate resolution, callee/caller selection, `--both` ordering, JSON and
//! ASCII-tree output shaping, stderr messages + exit codes).
//!
//! `adapter` implements `trace_cmd::Tracer` over the real `imc-trace` engine,
//! so the ported orchestration drives the ported tracer.
//!
//! `endpoint_bridge` adapts `imc-support`'s `EndpointBridge` to the engine's
//! `RouteBridge` trait, which is how Python's `ASTTracer` always runs: with a
//! bridge attached, `GET /api/x` targets and frontend call sites resolve.

pub mod adapter;
pub mod endpoint_bridge;
pub mod trace_cmd;