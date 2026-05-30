//! Integration test for `GET /dashboard`.
//!
//! Gated on `DATABASE_URL`. Skips silently otherwise. Drives the full
//! handler path (DB queries → Tera render) via `axum::Router::oneshot`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::dashboard::{
    dashboard_audit, dashboard_breaches, dashboard_outbox, dashboard_page, dashboard_wards,
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::connect;
use patient_administration_system::streaming::InMemoryEventPublisher;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn dashboard_renders_html_against_live_db() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping dashboard_renders_html_against_live_db");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let state = AppState::new(db, publisher);

    let app = axum::Router::new()
        .route("/dashboard", axum::routing::get(dashboard_page))
        .route("/dashboard/wards", axum::routing::get(dashboard_wards))
        .route(
            "/dashboard/breaches",
            axum::routing::get(dashboard_breaches),
        )
        .route("/dashboard/outbox", axum::routing::get(dashboard_outbox))
        .route("/dashboard/audit", axum::routing::get(dashboard_audit))
        .with_state(state);

    // --- Full page renders, returns text/html, contains all panel headers,
    //     wires up HTMX polling on every panel. ---
    let html = fetch_html(&app, "/dashboard").await;
    for header in ["Ward occupancy", "RTT breaches", "Outbox", "Recent audit"] {
        assert!(html.contains(header), "missing panel {header:?} in: {html}");
    }
    assert!(html.contains(env!("CARGO_PKG_VERSION")));
    assert!(html.contains("htmx.org@"), "page must embed HTMX");
    for hx in [
        "hx-get=\"/dashboard/wards\"",
        "hx-get=\"/dashboard/breaches\"",
        "hx-get=\"/dashboard/outbox\"",
        "hx-get=\"/dashboard/audit\"",
    ] {
        assert!(html.contains(hx), "page missing {hx}");
    }

    // --- Each fragment endpoint returns its panel body (not a full HTML
    //     document) — sanity-check by asserting we do NOT see the outer
    //     <header> wrapper that only the full page emits. ---
    for path in [
        "/dashboard/wards",
        "/dashboard/breaches",
        "/dashboard/outbox",
        "/dashboard/audit",
    ] {
        let frag = fetch_html(&app, path).await;
        assert!(
            !frag.contains("<header>"),
            "fragment {path} should NOT include the page header: {frag}"
        );
        assert!(
            !frag.contains("htmx.org@"),
            "fragment {path} should NOT include the HTMX script: {frag}"
        );
    }
}

async fn fetch_html(app: &axum::Router, uri: &str) -> String {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "GET {uri} non-200");
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.starts_with("text/html"),
        "GET {uri} expected text/html, got {ct:?}"
    );
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).expect("utf8")
}
