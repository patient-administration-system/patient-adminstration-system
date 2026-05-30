//! Integration test for the bearer-token middleware. This test does NOT
//! require `DATABASE_URL`: it composes the auth middleware on a tiny test
//! router with a synthetic handler, so we can exercise the auth logic in
//! isolation.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::get,
};
use patient_administration_system::api::rest::{RequireBearerToken, require_bearer};
use tower::ServiceExt;

async fn body_text(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn make_app() -> Router {
    let auth_state = RequireBearerToken::new("super-secret");
    Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/secret", get(|| async { "shhh" }))
        .layer(middleware::from_fn_with_state(auth_state, require_bearer))
}

#[tokio::test]
async fn health_is_always_exempt() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_text(resp).await, "ok");
}

#[tokio::test]
async fn missing_token_returns_401() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body = body_text(resp).await;
    assert!(body.contains("UNAUTHORIZED"), "body: {body}");
}

#[tokio::test]
async fn wrong_token_returns_401() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secret")
                .header("authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn correct_token_allows_access() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secret")
                .header("authorization", "Bearer super-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_text(resp).await, "shhh");
}
