//! Integration test for the v0.10 Coverage (insurance) surface.
//!
//! Walks: create patient + account → create coverage → list per patient →
//! link to account → list per account → update → soft-cancel via DELETE →
//! FHIR R5 read.
//!
//! Gated on `DATABASE_URL`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::fhir::fhir_router;
use patient_administration_system::api::rest::router;
use patient_administration_system::db::entities::patient;
use patient_administration_system::models::Gender;
use patient_administration_system::models::patient::{HumanName, Patient};
use sea_orm::{ActiveModelTrait, Set};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

async fn post(app: axum::Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .expect("post");
    let status = resp.status();
    (status, body_json(resp).await)
}

async fn put(app: axum::Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .expect("put");
    let status = resp.status();
    (status, body_json(resp).await)
}

async fn delete(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("delete");
    let status = resp.status();
    (status, body_json(resp).await)
}

async fn get(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("get");
    let status = resp.status();
    (status, body_json(resp).await)
}

async fn insert_patient(state: &patient_administration_system::api::rest::AppState) -> Uuid {
    let now = chrono::Utc::now().fixed_offset();
    let p = Patient::new(
        HumanName {
            use_type: None,
            family: format!("Cov{}", Uuid::new_v4().simple()),
            given: vec!["Test".into()],
            prefix: vec![],
            suffix: vec![],
        },
        Gender::Other,
    );
    let patient_id = p.id;
    patient::ActiveModel {
        id: Set(patient_id),
        mpi_id: Set(None),
        active: Set(true),
        name: Set(serde_json::to_value(&p.name).unwrap()),
        additional_names: Set(json!([])),
        identifiers: Set(json!([])),
        telecom: Set(json!([])),
        addresses: Set(json!([])),
        gender: Set("other".into()),
        birth_date: Set(None),
        deceased: Set(false),
        deceased_datetime: Set(None),
        emergency_contacts: Set(json!([])),
        marital_status: Set(None),
        replaced_by: Set(None),
        deleted_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .expect("insert patient");
    patient_id
}

#[tokio::test]
async fn coverage_full_lifecycle_with_fhir_read() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping coverage_full_lifecycle_with_fhir_read");
            return;
        }
    };
    let state = common::build_state(&url).await;
    migration::Migrator::up(&state.db, None)
        .await
        .expect("migrate");
    let patient_id = insert_patient(&state).await;
    let app = router(state.clone());
    let fhir = fhir_router(state.clone());

    // --- Create coverage ---
    let (status, body) = post(
        app.clone(),
        "/api/coverages",
        json!({
            "patient_id": patient_id,
            "payor_name": "Aetna",
            "policy_number": "POL-12345",
            "group_number": "GRP-9",
            "payor_identifier": "AET-MED-A",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create: {body}");
    let coverage_id: Uuid =
        serde_json::from_value(body["data"]["id"].clone()).expect("coverage id");
    assert_eq!(body["data"]["status"], "active");
    assert_eq!(body["data"]["kind"], "insurance");
    assert_eq!(body["data"]["relationship"], "self");
    assert!(body["data"]["account_id"].is_null());

    // --- List per patient: 1 row ---
    let (status, body) = get(
        app.clone(),
        &format!("/api/patients/{patient_id}/coverages"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"].as_array().unwrap().len(), 1);

    // --- Open an account, then link the coverage to it ---
    let (status, body) = post(
        app.clone(),
        "/api/accounts",
        json!({ "patient_id": patient_id, "currency": "USD" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open account: {body}");
    let account_id: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("account id");

    let (status, body) = put(
        app.clone(),
        &format!("/api/coverages/{coverage_id}"),
        json!({ "account_id": account_id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "link: {body}");
    assert_eq!(
        body["data"]["account_id"].as_str(),
        Some(account_id.to_string().as_str())
    );

    // --- List per account: 1 row ---
    let (status, body) = get(
        app.clone(),
        &format!("/api/accounts/{account_id}/coverages"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"].as_array().unwrap().len(), 1);

    // --- Update kind + group number ---
    let (status, body) = put(
        app.clone(),
        &format!("/api/coverages/{coverage_id}"),
        json!({ "kind": "self_pay", "group_number": null }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update: {body}");
    assert_eq!(body["data"]["kind"], "self_pay");
    assert!(
        body["data"]["group_number"].is_null(),
        "explicit null should clear: {body}"
    );

    // --- FHIR GET: returns a Coverage resource keyed on Patient/{id} ---
    let (status, body) = get(fhir.clone(), &format!("/fhir/Coverage/{coverage_id}")).await;
    assert_eq!(status, StatusCode::OK, "fhir read: {body}");
    assert_eq!(body["resourceType"], "Coverage");
    assert_eq!(body["status"], "active");
    assert_eq!(
        body["beneficiary"]["reference"].as_str(),
        Some(format!("Patient/{patient_id}").as_str())
    );
    // payor[0].display should mirror the payor_name.
    assert_eq!(body["payor"][0]["display"], "Aetna");
    // subscriberId carries the policy number (FHIR convention).
    assert_eq!(body["subscriberId"], "POL-12345");

    // --- Soft-cancel via DELETE: status flips, row remains ---
    let (status, body) = delete(app.clone(), &format!("/api/coverages/{coverage_id}")).await;
    assert_eq!(status, StatusCode::OK, "delete: {body}");
    assert_eq!(body["data"]["status"], "cancelled");

    // FHIR read after cancel: still returns a 200 with status=cancelled
    // (FHIR row is never gone; the audit trail is what matters).
    let (status, body) = get(fhir.clone(), &format!("/fhir/Coverage/{coverage_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "cancelled");

    // --- 404 for an unknown coverage id ---
    let unknown = Uuid::new_v4();
    let (status, body) = get(fhir.clone(), &format!("/fhir/Coverage/{unknown}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "fhir miss: {body}");
}
