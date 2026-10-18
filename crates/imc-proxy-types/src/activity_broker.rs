//! Port of `main/proxy/lib/activity_broker.py`.
//!
//! In-process pub/sub for Claude Code tool-call activity. Python uses
//! `asyncio.Queue` subscribers plus a 100-entry ring buffer; the Rust port uses
//! `tokio::sync::mpsc` channels with the same bounded-drop semantics (full
//! queue => event dropped for that subscriber only).

use std::sync::{Mutex, OnceLock};

use serde_json::{Map, Value};
use tokio::sync::mpsc;

/// Port of `activity_broker._RECENT_MAX`.
pub const RECENT_MAX: usize = 100;
/// Port of the `asyncio.Queue(maxsize=500)` bound.
pub const SUBSCRIBER_QUEUE_MAX: usize = 500;

struct BrokerState {
    subscribers: Vec<mpsc::Sender<Value>>,
    recent: Vec<Value>,
    seq: u64,
}

fn state() -> &'static Mutex<BrokerState> {
    static STATE: OnceLock<Mutex<BrokerState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(BrokerState {
            subscribers: Vec::new(),
            recent: Vec::new(),
            seq: 0,
        })
    })
}

/// Port of `activity_broker._next_seq`.
fn next_seq(state: &mut BrokerState) -> u64 {
    state.seq += 1;
    state.seq
}

/// Handle returned by [`subscribe_handle`]: keeps the receiving half plus enough
/// identity to unsubscribe the exact queue later.
pub struct SubscriberHandle {
    pub receiver: mpsc::Receiver<Value>,
    sender: mpsc::Sender<Value>,
}

impl SubscriberHandle {
    /// Port of `activity_broker.unsubscribe`.
    pub async fn unsubscribe(self) {
        if let Ok(mut s) = state().lock() {
            s.subscribers
                .retain(|q| q.max_capacity() == 0 || !same_sender(q, &self.sender));
        }
    }
}

fn same_sender(a: &mpsc::Sender<Value>, b: &mpsc::Sender<Value>) -> bool {
    // tokio Sender has no Eq; two senders are the same subscriber when they
    // were created from the same channel, which the pointer compare detects.
    std::ptr::eq(a, b)
}

/// Port of `activity_broker.subscribe` — returns the paired handle; keep the
/// handle alive for as long as the SSE connection lives.
pub async fn subscribe_handle() -> SubscriberHandle {
    let (tx, rx) = mpsc::channel(SUBSCRIBER_QUEUE_MAX);
    if let Ok(mut s) = state().lock() {
        s.subscribers.push(tx.clone());
    }
    SubscriberHandle { receiver: rx, sender: tx }
}

/// Bare-receiver form of [`subscribe`], for call sites that want Python's exact
/// return type (`asyncio.Queue`) and manage unsubscription themselves.
pub async fn subscribe() -> mpsc::Receiver<Value> {
    subscribe_handle().await.receiver
}

/// Port of `activity_broker.unsubscribe` — pass the sender that came from a
/// [`SubscriberHandle`]; drop the handle's receiver separately.
pub async fn unsubscribe(sender: mpsc::Sender<Value>) {
    if let Ok(mut s) = state().lock() {
        s.subscribers.retain(|q| !same_sender(q, &sender));
    }
}

/// Port of `activity_broker.recent` — snapshot of the ring buffer.
pub async fn recent() -> Vec<Value> {
    match state().lock() {
        Ok(s) => s.recent.clone(),
        Err(_) => Vec::new(),
    }
}

/// Port of `activity_broker.publish` — enriches with a monotonic `seq` and a
/// `ts` timestamp when absent, then fans out non-blocking.
pub fn publish(event: &Value) {
    // `event = dict(event)` in Python: only mapping keys are carried over.
    let mut map = match event.as_object() {
        Some(obj) => obj.clone(),
        None => Map::new(),
    };

    let seq = {
        let Ok(mut s) = state().lock() else {
            return;
        };
        next_seq(&mut s)
    };
    map.entry("seq").or_insert_with(|| Value::from(seq));
    if !map.contains_key("ts") {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        map.insert("ts".to_string(), serde_json::json!(ts));
    }
    let event_value = Value::Object(map);

    if let Ok(mut s) = state().lock() {
        s.recent.push(event_value.clone());
        if s.recent.len() > RECENT_MAX {
            let excess = s.recent.len() - RECENT_MAX;
            s.recent.drain(0..excess);
        }
        // Drop the ones whose queue is full — only for that subscriber.
        // A closed channel means the SSE client is gone; remove it outright.
        let mut i = 0;
        while i < s.subscribers.len() {
            if s.subscribers[i].is_closed() {
                s.subscribers.remove(i);
                continue;
            }
            // `try_send` mirrors `put_nowait`: Err means full (drop event) or
            // closed (drop subscriber), and both are non-fatal.
            if s.subscribers[i].try_send(event_value.clone()).is_err()
                && s.subscribers[i].is_closed()
            {
                s.subscribers.remove(i);
                continue;
            }
            i += 1;
        }
    }
}

/// Port of `activity_broker.sse_frame` — `data: {json}\n\n` as UTF-8 bytes.
pub fn sse_frame(event: &Value) -> Vec<u8> {
    let payload = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    format!("data: {payload}\n\n").into_bytes()
}

/// Port of the module-level `_seq` counter, exposed for tests.
pub fn current_seq() -> u64 {
    state().lock().map(|s| s.seq).unwrap_or(0)
}