//! Integration test for practitioner CRUD:
//! create → get → list → update → soft-delete (active=false) → list excludes.
//!
//! Gated on `DATABASE_URL`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::rest::router;
use serde_json::json;
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
async fn practitioner_full_flow() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping practitioner_full_flow");
            return;
        }
    };
    let state = common::build_state(&url).await;
    migration::Migrator::up(&state.db, None)
        .await
        .expect("migrate");
    let app = router(state.clone());

    // Create
    let (status, body) = req(
        app.clone(),
        "POST",
        "/api/practitioners",
        Some(json!({
            "family": "Curie",
            "given": ["Marie"],
            "gender": "Female",
            "birth_date": "1867-11-07"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create: {body}");
    let id: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("id");
    assert_eq!(body["data"]["active"], true);

    // Get
    let (status, body) = req(
        app.clone(),
        "GET",
        &format!("/api/practitioners/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["gender"], "female");
    assert_eq!(body["data"]["birth_date"], "1867-11-07");

    // List includes
    let (status, body) = req(app.clone(), "GET", "/api/practitioners", None).await;
    assert_eq!(status, StatusCode::OK);
    let ps = body["data"].as_array().unwrap();
    assert!(ps.iter().any(|p| p["id"] == id.to_string()));

    // Update: change given names + gender
    let (status, body) = req(
        app.clone(),
        "PUT",
        &format!("/api/practitioners/{id}"),
        Some(json!({
            "given": ["Maria", "Sklodowska"],
            "gender": "Other"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update: {body}");
    let name: serde_json::Value = body["data"]["name"].clone();
    let parsed: serde_json::Value =
        serde_json::from_str(name.as_str().unwrap_or("{}")).unwrap_or_else(|_| name.clone());
    // SeaORM stores it as a JSON value; serde_json::Value handles either shape.
    let given = match parsed.get("given") {
        Some(g) => g.clone(),
        None => name
            .get("given")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    };
    let given_arr = given.as_array().expect("given array");
    assert_eq!(given_arr.len(), 2, "two given names after update");
    assert_eq!(body["data"]["gender"], "other");

    // Delete (deactivate)
    let (status, body) = req(
        app.clone(),
        "DELETE",
        &format!("/api/practitioners/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete: {body}");
    assert_eq!(body["data"]["deactivated"], true);

    // List should now exclude (active=false filter)
    let (status, body) = req(app.clone(), "GET", "/api/practitioners", None).await;
    assert_eq!(status, StatusCode::OK);
    let ps = body["data"].as_array().unwrap();
    assert!(
        !ps.iter().any(|p| p["id"] == id.to_string()),
        "deactivated practitioner should not appear in list"
    );

    // GET by id still works — soft delete preserves the row
    let (status, body) = req(
        app.clone(),
        "GET",
        &format!("/api/practitioners/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["active"], false);
}
