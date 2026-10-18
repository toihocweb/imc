//! Port of `main/proxy/lib/request_context.py`.
//!
//! `contextvars.ContextVar` becomes a `tokio::task_local` (task-scoped, like a
//! Python context var) with a thread-local fallback so synchronous call sites
//! in the same request still see the value.

use std::sync::OnceLock;

use regex::Regex;

/// Port of the `RequestContext` class.
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    pub trace_id: String,
    /// Seconds since epoch, like Python's `time.time()`.
    pub start_time: f64,
    pub user_agent: String,
    pub session_affinity: Option<String>,
    pub parent_session_id: Option<String>,
}

impl RequestContext {
    /// Port of `RequestContext.__init__`.
    pub fn new(
        trace_id: impl Into<String>,
        start_time: f64,
        user_agent: impl Into<String>,
        session_affinity: Option<String>,
        parent_session_id: Option<String>,
    ) -> Self {
        Self {
            trace_id: trace_id.into(),
            start_time,
            user_agent: user_agent.into(),
            session_affinity,
            parent_session_id,
        }
    }
}

/// Port of `request_context.TRACE_ID_MAX_LENGTH`.
pub const TRACE_ID_MAX_LENGTH: usize = 64;

fn trace_id_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\w[\w.-]*$").unwrap())
}

tokio::task_local! {
    /// Port of the `request_context_var` ContextVar: scoped to the current task.
    static REQUEST_CONTEXT: std::cell::RefCell<Option<RequestContext>>;
}

thread_local! {
    /// Fallback for code outside [`with_request_context`] (sync handlers that
    /// read the value set earlier on the same thread).
    static THREAD_CONTEXT: std::cell::RefCell<Option<RequestContext>> =
        const { std::cell::RefCell::new(None) };
}

/// Port of `request_context.get_request_context`.
pub fn get_request_context() -> Option<RequestContext> {
    REQUEST_CONTEXT
        .try_with(|c| c.borrow().clone())
        .ok()
        .flatten()
        .or_else(|| THREAD_CONTEXT.with(|c| c.borrow().clone()))
}

/// Port of `request_context.set_request_context`. When called inside
/// [`with_request_context`] it mutates the task-local slot; otherwise it writes
/// the thread-local fallback.
pub fn set_request_context(context: RequestContext) {
    let mut used_task_local = false;
    let _ = REQUEST_CONTEXT.try_with(|c| {
        *c.borrow_mut() = Some(context.clone());
        used_task_local = true;
    });
    if !used_task_local {
        THREAD_CONTEXT.with(|c| *c.borrow_mut() = Some(context));
    }
}

/// Run `f` with `context` installed for the duration of the future — the Rust
/// analogue of `request_context_var.set(ctx)` at the top of a request handler.
pub async fn with_request_context<F, Fut, T>(context: RequestContext, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    REQUEST_CONTEXT
        .scope(std::cell::RefCell::new(Some(context)), async move {
            f().await
        })
        .await
}

/// Clear the thread-local fallback (call at request teardown).
pub fn clear_request_context() {
    THREAD_CONTEXT.with(|c| *c.borrow_mut() = None);
}

/// Port of `request_context.base36encode`. Python raises `TypeError` for
/// non-ints, which has no Rust analogue here (the argument is typed `i64`).
pub fn base36encode(number: i64) -> anyhow::Result<String> {
    if number < 0 {
        anyhow::bail!("number must be positive");
    }
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if number == 0 {
        return Ok("0".to_string());
    }
    let mut n = number as u64;
    let mut out = Vec::new();
    while n > 0 {
        out.push(ALPHABET[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    Ok(String::from_utf8(out).unwrap_or_default())
}

/// Port of `request_context.generate_trace_id` — `<base36 ms>-<6 random chars>`.
pub fn generate_trace_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() / 1_000_000)
        .unwrap_or(0);
    let timestamp = base36encode(now as i64).unwrap_or_else(|_| "0".to_string());
    let random_chars = crate::atomic_file::random_hex(6);
    format!("{timestamp}-{}", &random_chars[..6])
}

/// Port of `request_context.resolve_trace_id` — keeps a plausible inbound trace
/// id, otherwise mints a fresh one.
pub fn resolve_trace_id(trace_id: Option<&str>) -> String {
    let candidate = trace_id.map(str::trim).filter(|t| !t.is_empty());
    match candidate {
        Some(c) if c.len() <= TRACE_ID_MAX_LENGTH && trace_id_pattern().is_match(c) => {
            c.to_string()
        }
        _ => generate_trace_id(),
    }
}