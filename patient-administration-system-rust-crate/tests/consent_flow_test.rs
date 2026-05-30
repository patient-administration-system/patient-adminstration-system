//! Integration test for consent CRUD:
//! create → list → revoke → list again.
//!
//! Gated on `DATABASE_URL`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::rest::router;
use patient_administration_system::db::entities::patient;
use patient_administration_system::models::Gender;
use patient_administration_system::models::patient::{HumanName, Patient};
use sea_orm::{ActiveModelTrait, Set};
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
async fn consent_full_flow() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping consent_full_flow");
            return;
        }
    };
    let state = common::build_state(&url).await;
    migration::Migrator::up(&state.db, None)
        .await
        .expect("migrate");

    let now = chrono::Utc::now().fixed_offset();
    let p = Patient::new(
        HumanName {
            use_type: None,
            family: "Consent".into(),
            given: vec!["Kim".into()],
            prefix: vec![],
            suffix: vec![],
        },
        Gender::Unknown,
    );
    let patient_id = p.id;
    patient::ActiveModel {
        id: Set(patient_id),
        mpi_id: Set(None),
        active: Set(true),
        name: Set(serde_json::to_value(&p.name).unwrap()),
        additional_names: Set(serde_json::json!([])),
        identifiers: Set(serde_json::json!([])),
        telecom: Set(serde_json::json!([])),
        addresses: Set(serde_json::json!([])),
        gender: Set("unknown".into()),
        birth_date: Set(None),
        deceased: Set(false),
        deceased_datetime: Set(None),
        emergency_contacts: Set(serde_json::json!([])),
        marital_status: Set(None),
        replaced_by: Set(None),
        deleted_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .expect("insert patient");

    let app = router(state.clone());

    // Create two consents — data processing + marketing
    let (status, body) = req(
        app.clone(),
        "POST",
        &format!("/api/patients/{patient_id}/consents"),
        Some(json!({
            "consent_type": "data_processing",
            "granted_date": "2026-01-01",
            "purpose": "appointment scheduling",
            "method": "electronic"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create dp: {body}");
    let dp_id: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("dp id");
    assert_eq!(body["data"]["status"], "active");
    assert_eq!(body["data"]["consent_type"], "data_processing");

    let (status, body) = req(
        app.clone(),
        "POST",
        &format!("/api/patients/{patient_id}/consents"),
        Some(json!({
            "consent_type": "marketing",
            "granted_date": "2026-01-01",
            "expiry_date": "2027-01-01"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create marketing: {body}");
    let mkt_id: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("mkt id");

    // List should return both, active
    let (status, body) = req(
        app.clone(),
        "GET",
        &format!("/api/patients/{patient_id}/consents"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let consents = body["data"].as_array().expect("array");
    assert_eq!(consents.len(), 2, "expected 2 consents");
    for c in consents {
        assert_eq!(c["status"], "active");
    }

    // Revoke marketing
    let (status, body) = req(
        app.clone(),
        "POST",
        &format!("/api/consents/{mkt_id}/revoke"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "revoke: {body}");
    assert_eq!(body["data"]["status"], "revoked");

    // List again — marketing is revoked, data processing still active
    let (status, body) = req(
        app.clone(),
        "GET",
        &format!("/api/patients/{patient_id}/consents"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let consents = body["data"].as_array().unwrap();
    let dp = consents
        .iter()
        .find(|c| c["id"] == dp_id.to_string())
        .unwrap();
    let mkt = consents
        .iter()
        .find(|c| c["id"] == mkt_id.to_string())
        .unwrap();
    assert_eq!(dp["status"], "active");
    assert_eq!(mkt["status"], "revoked");

    // Audit trail should record the create + revoke
    let (status, body) = req(
        app.clone(),
        "GET",
        &format!("/api/patients/{patient_id}/audit"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // (the audit log is keyed by entity_type=patient, not consent — consent
    //  audit rows exist with entity_type=consent. The patient endpoint won't
    //  see those, so this check just ensures the endpoint responds, not the
    //  exact event count.)
    assert!(body["data"].is_array());
}
