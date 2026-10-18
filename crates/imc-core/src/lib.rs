//! Shared config, types and SQLite storage for the IndexMyCode Rust port.

pub mod config;
pub mod hash;
pub mod settings;
pub mod ingest;
pub mod store;
pub mod types;

pub use anyhow::Result;