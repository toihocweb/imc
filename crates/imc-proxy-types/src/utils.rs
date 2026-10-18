//! Port of `main/proxy/lib/utils.py`.
//!
//! The one import this module keeps from the Python version is the global
//! `state` singleton (see [`crate::state`]).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::state::state;

// ---------------------------------------------------------------------------
// sha256 — no hashing crate is available in this workspace's dependency set
// (imc-core exposes md5 only), so the primitive get_uuid needs lives here.
// ---------------------------------------------------------------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 over the concatenation of `parts`, returning raw digest bytes.
pub fn sha256_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut msg: Vec<u8> = Vec::new();
    for p in parts {
        msg.extend_from_slice(p);
    }
    let bit_len = (msg.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut w = [0u32; 64];
    for chunk in msg.chunks(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// SHA-256 of a single byte slice.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    sha256_parts(&[data])
}

// ---------------------------------------------------------------------------
// Time formatting helpers (no chrono in the dependency set)
// ---------------------------------------------------------------------------

/// Port of `datetime.fromtimestamp(ts).isoformat()` (seconds since epoch).
pub fn iso_format(ts_secs: f64) -> String {
    let (days, secs_of_day) = secs_to_days(ts_secs);
    let (y, mo, d) = civil_from_days(days);
    let (h, mi, s) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
}

/// Port of `datetime.now().strftime("%Y-%m-%d %H:%M:%S")` — UTC.
pub fn format_datetime_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let (days, secs_of_day) = secs_to_days(now);
    let (y, mo, d) = civil_from_days(days);
    let (h, mi, s) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

/// `"%Y-%m-%d"` for the log-rotation suffix — UTC.
pub fn format_date_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let (days, _) = secs_to_days(now);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}")
}

/// Seconds since epoch (possibly fractional) -> (days since 1970, seconds of day).
fn secs_to_days(ts: f64) -> (i64, i64) {
    let secs = ts.max(0.0) as i64;
    (secs.div_euclid(86_400), secs.rem_euclid(86_400))
}

/// Howard Hinnant's civil_from_days (UTC).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// asyncio.sleep(ms) equivalent
// ---------------------------------------------------------------------------

/// Port of `utils.sleep(ms)`.
pub async fn sleep(ms: i64) {
    let ms = ms.max(0) as u64;
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

// ---------------------------------------------------------------------------
// Type predicates
// ---------------------------------------------------------------------------

/// Port of `utils.is_nullish` — Python's `value is None`.
pub fn is_nullish(value: Option<&Value>) -> bool {
    value.is_none()
}

/// Port of `utils.is_async_iterable`. Rust has no async-iterable values to duck
/// type here; the proxy layer already knows which responses stream, so the
/// predicate is a documented no-op kept for call-site parity.
pub fn is_async_iterable(_value: &Value) -> bool {
    false
}

/// Port of `utils.is_responses_stream` — same duck typing as Python.
pub fn is_responses_stream(value: &Value) -> bool {
    is_async_iterable(value)
}

/// Port of `utils.is_record` — Python's `isinstance(value, dict)`.
pub fn is_record(value: &Value) -> bool {
    value.is_object()
}

// ---------------------------------------------------------------------------
// user_id parsing
// ---------------------------------------------------------------------------

/// Port of `utils.get_user_id_json_field`.
pub fn get_user_id_json_field(
    user_id_payload: Option<&Value>,
    field: &str,
) -> Option<String> {
    let payload = user_id_payload?;
    if !is_record(payload) {
        return None;
    }
    match payload.get(field) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Port of `utils.parse_json_user_id` — returns the parsed object only when it
/// is a JSON object.
pub fn parse_json_user_id(user_id: &str) -> Option<Value> {
    let parsed = serde_json::from_str::<Value>(user_id).ok()?;
    if is_record(&parsed) {
        Some(parsed)
    } else {
        None
    }
}

/// Return value of `utils.parse_user_id_metadata` — Python returns
/// `{"safetyIdentifier": ..., "sessionId": ...}`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserIdMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Port of `utils.parse_user_id_metadata`.
pub fn parse_user_id_metadata(user_id: Option<&str>) -> UserIdMetadata {
    let mut out = UserIdMetadata::default();
    let Some(user_id) = user_id.filter(|u| !u.is_empty()) else {
        return out;
    };

    static LEGACY_SAFETY: OnceLock<Regex> = OnceLock::new();
    static LEGACY_SESSION: OnceLock<Regex> = OnceLock::new();

    let legacy_safety_identifier = LEGACY_SAFETY
        .get_or_init(|| Regex::new(r"user_([^_]+)_account").unwrap())
        .captures(user_id)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    let legacy_session_id = LEGACY_SESSION
        .get_or_init(|| Regex::new(r"_session_(.+)$").unwrap())
        .captures(user_id)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    let parsed_user_id = if legacy_safety_identifier.is_some() && legacy_session_id.is_some() {
        None
    } else {
        parse_json_user_id(user_id)
    };
    let parsed_ref = parsed_user_id.as_ref();

    out.safety_identifier = legacy_safety_identifier
        .clone()
        .or_else(|| get_user_id_json_field(parsed_ref, "device_id"))
        .or_else(|| get_user_id_json_field(parsed_ref, "account_uuid"));
    out.session_id = legacy_session_id
        .clone()
        .or_else(|| get_user_id_json_field(parsed_ref, "session_id"));
    out
}

// ---------------------------------------------------------------------------
// Request-id generation
// ---------------------------------------------------------------------------

/// Port of `utils.find_last_user_content`.
pub fn find_last_user_content(messages: &[Value]) -> Option<String> {
    for msg in messages.iter().rev() {
        if msg.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let content = msg.get("content")?;
        match content {
            Value::String(s) => {
                if !s.is_empty() {
                    return Some(s.clone());
                }
            }
            Value::Array(items) => {
                let mut array: Vec<Value> = Vec::new();
                for n in items {
                    let Some(obj) = n.as_object() else { continue };
                    if obj.get("type").and_then(Value::as_str) == Some("tool_result") {
                        continue;
                    }
                    let mut item = obj.clone();
                    item.remove("cache_control");
                    array.push(Value::Object(item));
                }
                if !array.is_empty() {
                    return serde_json::to_string(&array).ok();
                }
            }
            _ => {}
        }
        // Python `continue`s through the reversed scan; only a non-empty
        // user content short-circuits.
    }
    None
}

/// Port of `utils.generate_request_id_from_payload`.
pub fn generate_request_id_from_payload(
    payload: &Value,
    session_id: Option<&str>,
) -> String {
    if let Some(messages) = payload.get("messages") {
        let last_user_content = match messages {
            Value::String(s) => Some(s.clone()),
            Value::Array(arr) => find_last_user_content(arr),
            _ => None,
        };
        if let Some(last_user_content) = last_user_content {
            let content_to_hash = format!(
                "{}{}{}",
                session_id.unwrap_or(""),
                state().mac_machine_id.clone().unwrap_or_default(),
                last_user_content
            );
            return get_uuid(&content_to_hash);
        }
    }
    random_uuid_v4()
}

/// Port of `utils.get_root_session_id` — headers are expected lowercase.
pub fn get_root_session_id(anthropic_payload: &Value, headers: &BTreeMap<String, String>) -> Option<String> {
    let metadata = anthropic_payload.get("metadata");
    let user_id = metadata.and_then(|m| m.get("user_id")).and_then(Value::as_str);

    let session_id = match user_id {
        Some(uid) => parse_user_id_metadata(Some(uid)).session_id,
        None => headers.get("x-session-id").cloned(),
    };

    match session_id {
        Some(sid) if !sid.is_empty() => Some(get_uuid(&sid)),
        _ => None,
    }
}

/// Port of `utils.get_uuid` — sha256 digest, UUIDv4-shaped.
pub fn get_uuid(content: &str) -> String {
    let digest = sha256_parts(&[content.as_bytes()]);
    format_uuid_v4(&digest)
}

/// Format 16 bytes as a UUID v4 (version 4 / RFC 4122 variant), matching the
/// byte-twiddling in the Python version.
pub fn format_uuid_v4(bytes: &[u8; 32]) -> String {
    let mut uuid_bytes = [0u8; 16];
    uuid_bytes.copy_from_slice(&bytes[..16]);
    uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x40;
    uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
    let hex: String = uuid_bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// Port of `str(uuid.uuid4())` — same v4 shape, seeded from wall clock + pid +
/// a process-wide counter through the crate's sha256.
pub fn random_uuid_v4() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id() as u64;
    let digest = sha256_parts(&[
        &nanos.to_le_bytes(),
        &pid.to_le_bytes(),
        &counter.to_le_bytes(),
    ]);
    format_uuid_v4(&digest)
}