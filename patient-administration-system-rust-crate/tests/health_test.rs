//! Smoke test: the REST router answers `GET /api/health` with `200 OK`.
//!
//! Skipped silently when `DATABASE_URL` is not set, so plain `cargo test`
//! works on a developer machine without Postgres.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use patient_administration_system::api::rest::router;
use tower::ServiceExt;

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping health_endpoint_returns_ok");
            return;
        }
    };
    let state = common::build_state(&url).await;
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("response");
    assert_eq!(resp.status(), StatusCode::OK);
}
