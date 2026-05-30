//! HTTP webhook outbound publisher (v0.14).
//!
//! Implements [`EventPublisher`] by POSTing each outbox [`DomainEvent`] as
//! JSON to a configured URL. Pairs cleanly with the existing dispatcher /
//! dead-letter machinery from v0.5: a non-2xx response (or a transport
//! failure) returns `Error::Streaming`, which leaves the outbox row
//! unpublished so the next dispatcher tick will retry it; once the retry
//! budget (`PAS_OUTBOX_MAX_RETRIES`) is exhausted the row moves to
//! `outbox_dead_letters` like any other publish failure.
//!
//! ## Wire format
//!
//! `POST <url>` with `Content-Type: application/json`. The body is the
//! serialised `DomainEvent`:
//!
//! ```json
//! {
//!   "id":         "9e7e…",
//!   "event_type": "EncounterAdmitted",
//!   "payload":    { ... },
//!   "at":         "2026-05-25T12:00:00Z"
//! }
//! ```
//!
//! Headers:
//!
//! | Header               | Always sent? | Notes                                       |
//! |----------------------|--------------|---------------------------------------------|
//! | `Content-Type`       | yes          | `application/json`                          |
//! | `X-PAS-Event-Id`     | yes          | UUID of the event                           |
//! | `X-PAS-Event-Type`   | yes          | e.g. `EncounterAdmitted`                    |
//! | `X-PAS-Signature`    | when secret set | `sha256=<lowercase hex>` of HMAC-SHA256 over the raw body, keyed by the secret. Receivers MUST verify this constant-time. |
//!
//! No bearer / static auth header is sent; HMAC is the only auth mode in
//! v0.14. Use `PAS_WEBHOOK_SECRET` to enable it.
//!
//! ## Failure semantics
//!
//! - 2xx response (any) → `Ok(())`; outbox row marked published.
//! - 4xx / 5xx → `Err(Error::Streaming(...))`; outbox row stays in the
//!   pending state and the dispatcher will retry on its next tick.
//! - Transport failure (DNS, TLS, connect, read, timeout) → same as above.
//! - Receivers should be idempotent on `X-PAS-Event-Id` — the dispatcher
//!   will retry on any non-2xx, including post-200 network failures the
//!   server already committed locally.

use std::time::Duration;

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

type HmacSha256 = Hmac<Sha256>;

/// Default request timeout when `PAS_WEBHOOK_TIMEOUT_SECS` is unset.
pub const DEFAULT_TIMEOUT_SECS: u64 = 10;

/// HTTP webhook publisher. Built once at startup; cheap to clone via
/// `Arc`.
pub struct WebhookEventPublisher {
    url: String,
    secret: Option<Vec<u8>>,
    client: reqwest::Client,
}

impl WebhookEventPublisher {
    /// Build a new webhook publisher.
    ///
    /// * `url` — full URL the publisher will POST to. Empty string is
    ///   rejected (callers should branch on this in `main.rs`).
    /// * `secret` — optional HMAC-SHA256 secret. When `Some`, every
    ///   request carries an `X-PAS-Signature: sha256=<hex>` header.
    /// * `timeout` — request timeout. The dispatcher polls every 2 s so
    ///   we want this strictly shorter than the poll interval to keep
    ///   the loop responsive.
    pub fn new(url: impl Into<String>, secret: Option<Vec<u8>>, timeout: Duration) -> Result<Self> {
        let url = url.into();
        if url.is_empty() {
            return Err(Error::Streaming("webhook URL must be non-empty".into()));
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| Error::Streaming(format!("build reqwest client: {e}")))?;
        Ok(Self {
            url,
            secret,
            client,
        })
    }

    /// Compute `sha256=<lowercase hex>` HMAC of `body` under `secret`.
    /// Public for unit-testing receivers in-tree.
    pub fn sign(secret: &[u8], body: &[u8]) -> String {
        // Hmac::new_from_slice never panics for any key length; we accept
        // any non-empty secret. (Constant-time verification on the receive
        // side is the receiver's responsibility.)
        let mut mac =
            HmacSha256::new_from_slice(secret).expect("HMAC-SHA256 accepts any key length");
        mac.update(body);
        let result = mac.finalize().into_bytes();
        let mut out = String::with_capacity(7 + result.len() * 2);
        out.push_str("sha256=");
        for byte in result {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    /// The configured destination URL (for logs and tests).
    pub fn url(&self) -> &str {
        &self.url
    }
}

#[async_trait]
impl EventPublisher for WebhookEventPublisher {
    async fn publish(&self, event: DomainEvent) -> Result<()> {
        let body = serde_json::to_vec(&event)
            .map_err(|e| Error::Streaming(format!("serialise event: {e}")))?;
        let mut req = self
            .client
            .post(&self.url)
            .header("content-type", "application/json")
            .header("x-pas-event-id", event.id.to_string())
            .header("x-pas-event-type", &event.event_type);
        if let Some(secret) = &self.secret {
            req = req.header("x-pas-signature", Self::sign(secret, &body));
        }
        let resp = req
            .body(body)
            .send()
            .await
            .map_err(|e| Error::Streaming(format!("webhook POST to {}: {e}", self.url)))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Streaming(format!(
                "webhook POST to {} returned {} (body: {})",
                self.url,
                status,
                body.chars().take(200).collect::<String>()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Recorder for what the one-shot HTTP server received.
    type Recorder = std::sync::Arc<
        tokio::sync::Mutex<Option<(Vec<u8>, std::collections::HashMap<String, String>)>>,
    >;

    /// Tiny ad-hoc HTTP/1.1 server that accepts one request, returns a
    /// configurable status, and shuts down. Avoids pulling in a mock-server
    /// dep — we only need a single connection per test.
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
                    // Parse the Content-Length to know how many more body bytes to read.
                    let headers_str = std::str::from_utf8(&buf[..header_end]).unwrap();
                    let mut headers: std::collections::HashMap<String, String> =
                        std::collections::HashMap::new();
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
                        let body =
                            buf[(header_end + 4)..(header_end + 4 + content_length)].to_vec();
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

    #[test]
    fn test_sign_produces_expected_hmac_sha256_hex() {
        // Reference vector: empty body, key "key" — HMAC-SHA256("key", "") =
        // 5d5d139563c95b5967b9bd9a8c9b233a9dedb45072794cd232dc1b74832607d0
        let sig = WebhookEventPublisher::sign(b"key", b"");
        assert_eq!(
            sig,
            "sha256=5d5d139563c95b5967b9bd9a8c9b233a9dedb45072794cd232dc1b74832607d0"
        );
    }

    #[test]
    fn test_sign_changes_with_body() {
        let a = WebhookEventPublisher::sign(b"secret", b"{}");
        let b = WebhookEventPublisher::sign(b"secret", b"{\"x\":1}");
        assert_ne!(a, b);
        assert!(a.starts_with("sha256="));
        assert_eq!(a.len(), 7 + 64);
    }

    #[test]
    fn test_sign_changes_with_secret() {
        let a = WebhookEventPublisher::sign(b"secret-A", b"body");
        let b = WebhookEventPublisher::sign(b"secret-B", b"body");
        assert_ne!(a, b);
    }

    #[test]
    fn test_new_rejects_empty_url() {
        let err = match WebhookEventPublisher::new("", None, Duration::from_secs(1)) {
            Ok(_) => panic!("empty URL must be rejected"),
            Err(e) => e,
        };
        assert!(matches!(err, Error::Streaming(_)));
    }

    #[tokio::test]
    async fn test_publish_posts_body_and_headers() {
        let recorder = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let addr = start_oneshot_server("HTTP/1.1 200 OK", recorder.clone()).await;
        let pub_ = WebhookEventPublisher::new(
            format!("http://{addr}/hook"),
            Some(b"shared-secret".to_vec()),
            Duration::from_secs(5),
        )
        .unwrap();
        let event = DomainEvent::new("EncounterAdmitted", json!({ "patient_id": "abc" }));
        let event_id = event.id;
        pub_.publish(event).await.expect("2xx must be Ok");

        let (body, headers) = recorder
            .lock()
            .await
            .clone()
            .expect("server recorded request");
        // Body is the serialised DomainEvent.
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["event_type"], "EncounterAdmitted");
        assert_eq!(parsed["payload"]["patient_id"], "abc");
        assert_eq!(parsed["id"], event_id.to_string());
        // Required headers are present.
        assert_eq!(
            headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(
            headers.get("x-pas-event-id").map(String::as_str),
            Some(event_id.to_string().as_str())
        );
        assert_eq!(
            headers.get("x-pas-event-type").map(String::as_str),
            Some("EncounterAdmitted")
        );
        // Signature header is present and matches the body.
        let expected_sig = WebhookEventPublisher::sign(b"shared-secret", &body);
        assert_eq!(
            headers.get("x-pas-signature").map(String::as_str),
            Some(expected_sig.as_str())
        );
    }

    #[tokio::test]
    async fn test_publish_omits_signature_header_when_no_secret() {
        let recorder = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let addr = start_oneshot_server("HTTP/1.1 200 OK", recorder.clone()).await;
        let pub_ =
            WebhookEventPublisher::new(format!("http://{addr}/hook"), None, Duration::from_secs(5))
                .unwrap();
        pub_.publish(DomainEvent::new("PatientCreated", json!({})))
            .await
            .expect("ok");
        let (_, headers) = recorder.lock().await.clone().unwrap();
        assert!(!headers.contains_key("x-pas-signature"));
    }

    #[tokio::test]
    async fn test_publish_non_2xx_returns_streaming_err() {
        let recorder = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let addr = start_oneshot_server("HTTP/1.1 500 Internal Server Error", recorder).await;
        let pub_ =
            WebhookEventPublisher::new(format!("http://{addr}/hook"), None, Duration::from_secs(5))
                .unwrap();
        let err = pub_
            .publish(DomainEvent::new("PatientCreated", json!({})))
            .await
            .expect_err("500 must surface as Err");
        match err {
            Error::Streaming(msg) => assert!(msg.contains("500"), "msg: {msg}"),
            other => panic!("expected Streaming, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_publish_transport_failure_returns_streaming_err() {
        // Port 1 is reserved (tcpmux); on macOS the connect fails immediately.
        let pub_ =
            WebhookEventPublisher::new("http://127.0.0.1:1/hook", None, Duration::from_millis(500))
                .unwrap();
        let err = pub_
            .publish(DomainEvent::new("PatientCreated", json!({})))
            .await
            .expect_err("transport failure must surface as Err");
        assert!(matches!(err, Error::Streaming(_)));
    }
}
