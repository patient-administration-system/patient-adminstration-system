//! Integration test for patient CRUD + Tantivy search synchronization.
//!
//! Gated on `DATABASE_URL`. Builds an `AppState` with a temp Tantivy index
//! so the create/update/delete handlers exercise the real search wiring.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::rest::{AppState, router};
use patient_administration_system::db::connect;
use patient_administration_system::search::SearchEngine;
use patient_administration_system::streaming::InMemoryEventPublisher;
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn req(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut b = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.oneshot(b.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    (status, body_json(resp).await)
}

#[tokio::test]
async fn patient_crud_and_search_flow() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping patient_crud_and_search_flow");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db, publisher).with_search(search);

    let app = router(state.clone());

    // Create a patient
    let (status, body) = req(
        app.clone(),
        "POST",
        "/api/patients",
        Some(json!({
            "family": "Marlowe",
            "given": ["Christopher"],
            "gender": "Other",
            "birth_date": "1980-02-06",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create: {body}");
    let pid: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("id");

    // GET should return the same patient
    let (status, body) = req(app.clone(), "GET", &format!("/api/patients/{pid}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["name"]["family"], "Marlowe");

    // Search by family name (Tantivy commit is synchronous in our wrapper)
    let (status, body) = req(
        app.clone(),
        "GET",
        "/api/patients/search?q=Marlowe&limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "search: {body}");
    let hits = body["data"].as_array().expect("array");
    assert!(
        hits.iter().any(|p| p["id"] == pid.to_string()),
        "expected our patient in search hits: {body}"
    );

    // Update family name
    let (status, body) = req(
        app.clone(),
        "PUT",
        &format!("/api/patients/{pid}"),
        Some(json!({ "family": "Shakespeare" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update: {body}");
    assert_eq!(body["data"]["name"]["family"], "Shakespeare");

    // Search by new name should hit
    let (status, body) = req(
        app.clone(),
        "GET",
        "/api/patients/search?q=Shakespeare&limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let hits = body["data"].as_array().expect("array");
    assert!(
        hits.iter().any(|p| p["id"] == pid.to_string()),
        "expected updated name in search hits"
    );

    // List should include patient
    let (status, body) = req(app.clone(), "GET", "/api/patients?limit=100", None).await;
    assert_eq!(status, StatusCode::OK);
    let listed = body["data"].as_array().expect("array");
    assert!(listed.iter().any(|p| p["id"] == pid.to_string()));

    // Masked view should mask sensitive fields (telecom/identifiers; both empty here so just check 200)
    let (status, _) = req(
        app.clone(),
        "GET",
        &format!("/api/patients/{pid}/masked"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Soft-delete
    let (status, body) = req(app.clone(), "DELETE", &format!("/api/patients/{pid}"), None).await;
    assert_eq!(status, StatusCode::OK, "delete: {body}");
    assert_eq!(body["data"]["deleted"], true);

    // List should no longer include soft-deleted patient
    let (status, body) = req(app.clone(), "GET", "/api/patients?limit=100", None).await;
    assert_eq!(status, StatusCode::OK);
    let listed = body["data"].as_array().expect("array");
    assert!(
        !listed.iter().any(|p| p["id"] == pid.to_string()),
        "soft-deleted patient should not be listed"
    );

    // Search should also no longer hit (deleted from Tantivy)
    let (status, body) = req(
        app.clone(),
        "GET",
        "/api/patients/search?q=Shakespeare&limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let hits = body["data"].as_array().expect("array");
    assert!(
        !hits.iter().any(|p| p["id"] == pid.to_string()),
        "deleted patient should not appear in search"
    );

    // Audit history should include our actions
    let (status, body) = req(
        app.clone(),
        "GET",
        &format!("/api/patients/{pid}/audit"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries = body["data"].as_array().expect("array");
    let actions: Vec<&str> = entries
        .iter()
        .filter_map(|e| e["action"].as_str())
        .collect();
    assert!(
        actions.contains(&"create"),
        "audit must include create: {actions:?}"
    );
    assert!(actions.contains(&"update"));
    assert!(actions.contains(&"soft_delete"));
}
