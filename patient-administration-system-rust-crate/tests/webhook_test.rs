//! Integration tests for the v0.14 outbox webhook publisher.
//!
//! These tests exercise the **public crate surface** (`streaming::Webhook
//! EventPublisher` and `streaming::CompositePublisher`) against an
//! in-process `tokio::net::TcpListener` acting as a minimal HTTP/1.1
//! receiver. They are **DB-free**: like `auth_test.rs` and
//! `rate_limit_test.rs`, they run every `cargo test` regardless of
//! `DATABASE_URL`.

use patient_administration_system::streaming::{
    CompositePublisher, DomainEvent, EventPublisher, InMemoryEventPublisher, WebhookEventPublisher,
};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// Recorder for what the one-shot HTTP server received.
type Recorder = Arc<Mutex<Option<(Vec<u8>, HashMap<String, String>)>>>;

async fn start_oneshot_server(status_line: &'static str, recorder: Recorder) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::with_capacity(4096);
        let mut tmp = [0u8; 1024];
        loop {
            let n = sock.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(header_end) = find_double_crlf(&buf) {
                let headers_str = std::str::from_utf8(&buf[..header_end]).unwrap();
                let mut headers: HashMap<String, String> = HashMap::new();
                for line in headers_str.split("\r\n").skip(1) {
                    if let Some((k, v)) = line.split_once(": ") {
                        headers.insert(k.to_ascii_lowercase(), v.to_string());
                    }
                }
                let content_length: usize = headers
                    .get("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let body_so_far = buf.len() - (header_end + 4);
                if body_so_far >= content_length {
                    let body = buf[(header_end + 4)..(header_end + 4 + content_length)].to_vec();
                    *recorder.lock().await = Some((body, headers));
                    let response = format!("{status_line}\r\nContent-Length: 0\r\n\r\n");
                    sock.write_all(response.as_bytes()).await.unwrap();
                    sock.shutdown().await.ok();
                    break;
                }
            }
        }
    });
    addr
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// End-to-end smoke: build a `WebhookEventPublisher` exactly like
/// `main.rs` does, publish an event, and verify the receiver saw the
/// right body + headers including a valid HMAC signature.
#[tokio::test]
async fn webhook_publisher_posts_event_with_hmac_signature() {
    let recorder: Recorder = Arc::new(Mutex::new(None));
    let addr = start_oneshot_server("HTTP/1.1 202 Accepted", recorder.clone()).await;
    let publisher = WebhookEventPublisher::new(
        format!("http://{addr}/pas-events"),
        Some(b"production-secret".to_vec()),
        Duration::from_secs(2),
    )
    .expect("publisher builds");

    let event = DomainEvent::new(
        "EncounterAdmitted",
        json!({ "encounter_id": "abc", "bed_code": "B-1" }),
    );
    let event_id = event.id;
    let event_type = event.event_type.clone();
    publisher.publish(event).await.expect("202 must be Ok");

    let (body, headers) = recorder.lock().await.clone().expect("recorder");
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["event_type"], event_type);
    assert_eq!(payload["payload"]["bed_code"], "B-1");
    assert_eq!(payload["id"], event_id.to_string());
    assert_eq!(
        headers.get("x-pas-event-id").map(String::as_str),
        Some(event_id.to_string().as_str())
    );
    assert_eq!(
        headers.get("x-pas-event-type").map(String::as_str),
        Some(event_type.as_str())
    );
    let sig = headers
        .get("x-pas-signature")
        .expect("signature header must be present when secret is set");
    let expected = WebhookEventPublisher::sign(b"production-secret", &body);
    assert_eq!(sig, &expected);
    assert!(sig.starts_with("sha256="));
    assert_eq!(sig.len(), 7 + 64);
}

/// Composite publisher: when a webhook + an in-memory publisher are
/// both configured (the production "fan out to many subscribers"
/// scenario), every successful publish lands at every subscriber.
#[tokio::test]
async fn composite_publisher_fans_out_to_webhook_and_in_memory() {
    let recorder: Recorder = Arc::new(Mutex::new(None));
    let addr = start_oneshot_server("HTTP/1.1 200 OK", recorder.clone()).await;
    let webhook = Arc::new(
        WebhookEventPublisher::new(format!("http://{addr}/hook"), None, Duration::from_secs(2))
            .unwrap(),
    );
    let memory = Arc::new(InMemoryEventPublisher::new());
    let composite: Arc<dyn EventPublisher> = Arc::new(CompositePublisher::new(vec![
        webhook.clone(),
        memory.clone(),
    ]));
    composite
        .publish(DomainEvent::new("PatientCreated", json!({ "x": 42 })))
        .await
        .expect("ok");

    // Webhook received it.
    let (body, _) = recorder.lock().await.clone().expect("webhook receiver");
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["event_type"], "PatientCreated");
    assert_eq!(payload["payload"]["x"], 42);

    // In-memory publisher also recorded it.
    let in_mem = memory.events().await;
    assert_eq!(in_mem.len(), 1);
    assert_eq!(in_mem[0].event_type, "PatientCreated");
    assert_eq!(in_mem[0].payload["x"], 42);
}
