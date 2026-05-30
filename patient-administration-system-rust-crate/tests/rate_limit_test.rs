//! Integration test for the v0.12 per-IP rate-limit middleware.
//!
//! Exercises the layer end-to-end via `tower::ServiceExt::oneshot` against
//! a router that has the rate-limit middleware installed with a tight
//! budget. Verifies:
//!
//! - `/api/health` is exempt — never trips the limiter even when other
//!   endpoints have been throttled to zero.
//! - Within burst capacity, requests pass through.
//! - Past burst, requests get a `429 Too Many Requests` with a
//!   `Retry-After` header and the standard `ApiResponse` envelope
//!   carrying `error.code = "RATE_LIMITED"`.
//!
//! Doesn't need `DATABASE_URL` — exempt path requires no DB, and the
//! 429 path is generated entirely by the middleware. Hits a non-DB
//! endpoint (`/api/health` doesn't go through the limiter, so we use
//! a path that always 404s — the middleware runs before the route
//! lookup, so it still gets to apply its decision).

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use patient_administration_system::api::rest::{
    RateLimitConfig, RateLimiter, rate_limit_middleware,
};
use tower::ServiceExt;

/// Build a tiny test app: one fake handler that always returns 200 OK
/// for `/test` and `/api/health`. Wrap with the rate-limit middleware
/// at the requested config. We deliberately don't use the real PAS
/// router — this test isolates the middleware behavior.
fn build_app(cfg: RateLimitConfig) -> Router {
    let limiter = RateLimiter::new(cfg);
    Router::new()
        .route("/test", axum::routing::get(|| async { "ok" }))
        .route("/api/health", axum::routing::get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(
            limiter,
            rate_limit_middleware,
        ))
}

async fn hit(app: &Router, uri: &str) -> StatusCode {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("response");
    resp.status()
}

#[tokio::test]
async fn rate_limit_blocks_after_burst_and_serves_429_with_retry_after() {
    // Burst 2 → first two requests pass, third gets 429.
    let app = build_app(RateLimitConfig {
        requests_per_minute: 60, // 1 token/sec — so the bucket won't refill instantly
        burst: 2,
    });

    assert_eq!(hit(&app, "/test").await, StatusCode::OK);
    assert_eq!(hit(&app, "/test").await, StatusCode::OK);

    // Third hit should be denied.
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .expect("response");
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "third request should be 429"
    );
    assert!(
        resp.headers().get("retry-after").is_some(),
        "429 must carry Retry-After header"
    );
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("body");
    let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(v["success"], false);
    assert_eq!(v["error"]["code"], "RATE_LIMITED");
}

#[tokio::test]
async fn rate_limit_exempts_health_endpoint() {
    // Burst 0 + RPM 0 disables the limiter, but we want to prove the
    // EXEMPT path works — so build with a tight non-zero config and
    // hammer /api/health. Even with the bucket fully drained, health
    // pings must succeed.
    let app = build_app(RateLimitConfig {
        requests_per_minute: 60,
        burst: 1,
    });

    // Drain the bucket on /test.
    assert_eq!(hit(&app, "/test").await, StatusCode::OK);
    assert_eq!(hit(&app, "/test").await, StatusCode::TOO_MANY_REQUESTS);

    // /api/health should keep returning 200 — it's exempt.
    for _ in 0..5 {
        assert_eq!(
            hit(&app, "/api/health").await,
            StatusCode::OK,
            "/api/health must be exempt from the rate limiter"
        );
    }
}
