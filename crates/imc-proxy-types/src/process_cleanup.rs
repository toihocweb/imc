//! Port of `main/proxy/lib/process_cleanup.py` — `atexit`-style handlers.
//!
//! Rust has no interpreter-exit hook wired by default, so the module exposes
//! [`run_process_cleanups_sync`] for the owning binary to call from its own
//! shutdown path (or a `Drop` on the process guard).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

type CleanupHandler = Box<dyn Fn() + Send + Sync>;

fn handlers() -> &'static Mutex<HashMap<u64, CleanupHandler>> {
    static HANDLERS: OnceLock<Mutex<HashMap<u64, CleanupHandler>>> = OnceLock::new();
    HANDLERS.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Port of `process_cleanup.register_process_cleanup` — returns an unregister
/// closure (Python returns a lambda that removes the handler).
pub fn register_process_cleanup<F: Fn() + Send + Sync + 'static>(handler: F) -> impl Fn() {
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut map) = handlers().lock() {
        map.insert(id, Box::new(handler));
    }
    move || {
        if let Ok(mut map) = handlers().lock() {
            map.remove(&id);
        }
    }
}

/// Port of `process_cleanup.run_process_cleanups_sync`. Python swallows handler
/// errors and only runs once; here the `done` gate is the cleared map, and
/// failures are ignored the same way.
pub fn run_process_cleanups_sync() {
    let drained: Vec<CleanupHandler> = match handlers().lock() {
        Ok(mut map) => map.drain().map(|(_, h)| h).collect(),
        Err(_) => return,
    };
    for handler in drained {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler()));
    }
}