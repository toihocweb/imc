//! A rate limit is waited out, not worked around.
//!
//! The endpoint that answers 429 says when to come back. Splitting the batch
//! (what a refusal gets) would make more requests and rate-limit harder, and
//! giving up loses the vector: the chunk is then unreachable by the vector
//! leg of the search, silently.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use imc_embed::{Embedder, RemoteEmbedder};

/// A server that answers 429 with `Retry-After: 1` for the first `fail_times`
/// requests, then answers normally.
fn stub_server(fail_times: usize) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n < fail_times {
                let body = r#"{"error":{"code":"RATE_LIMIT_EXCEEDED","message":"Rate limit exceeded. Please retry after 1 seconds."}}"#;
                let response = format!(
                    "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            } else {
                // A valid response: one 3-dimensional vector per input.
                let payload = r#"{"data":[{"index":0,"embedding":[0.1,0.2,0.3]},{"index":1,"embedding":[0.4,0.5,0.6]}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    payload.len(),
                    payload
                );
                let _ = stream.write_all(response.as_bytes());
            }
        }
    });
    (format!("http://{addr}/v1"), seen)
}

#[test]
fn a_rate_limit_is_waited_out_and_the_vectors_still_come_back() {
    // The endpoint refuses the first two requests.
    let (base, seen) = stub_server(2);
    let embedder = RemoteEmbedder::new(imc_core::config::EmbedConfig {
        base_url: base,
        api_key: Some("test-key".to_string()),
        model: "stub".to_string(),
        fallback_dim: 8,
    });
    let texts = vec!["one".to_string(), "two".to_string()];

    let vectors = embedder.embed(&texts).expect("the batch must survive");

    assert_eq!(vectors.len(), 2, "{vectors:?}");
    assert!(
        vectors.iter().all(|v| !v.is_empty()),
        "no input may lose its vector: {vectors:?}"
    );
    assert!(seen.load(Ordering::SeqCst) >= 3, "the request was retried");
}

#[test]
fn query_vector_cache_hits_without_network_request() {
    let (base, seen) = stub_server(0);
    let embedder = RemoteEmbedder::new(imc_core::config::EmbedConfig {
        base_url: base,
        api_key: Some("test-key".to_string()),
        model: "stub".to_string(),
        fallback_dim: 8,
    });
    let query = vec!["how does authentication work".to_string()];

    // First call: hits the network
    let v1 = embedder.embed(&query).expect("first embed");
    let network_calls_after_first = seen.load(Ordering::SeqCst);
    assert_eq!(network_calls_after_first, 1);

    // Second call with same text: served completely from cache without network call
    let v2 = embedder.embed(&query).expect("second embed (cached)");
    let network_calls_after_second = seen.load(Ordering::SeqCst);
    assert_eq!(network_calls_after_second, 1, "cache must not make second network call");
    assert_eq!(v1, v2);
}
