//! Integration test for FHIR R5 Patient write endpoints:
//! POST → GET → PUT → DELETE. Gated on `DATABASE_URL`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::fhir::fhir_router;
use patient_administration_system::api::rest::AppState;
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
    if bytes.is_empty() {
        return serde_json::Value::Null;
    }
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
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
async fn fhir_patient_crud() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping fhir_patient_crud");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");
    let tmp = tempfile::tempdir().unwrap();
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).unwrap());
    let state = AppState::new(db, Arc::new(InMemoryEventPublisher::new())).with_search(search);
    let app = fhir_router(state.clone());

    // POST /fhir/Patient
    let (status, body) = req(
        app.clone(),
        "POST",
        "/fhir/Patient",
        Some(json!({
            "resourceType": "Patient",
            "active": true,
            "gender": "female",
            "birthDate": "1990-04-12",
            "name": [{
                "use": "official",
                "family": "Lovelace",
                "given": ["Ada"]
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "POST: {body}");
    assert_eq!(body["resourceType"], "Patient");
    let id: Uuid = body["id"].as_str().unwrap().parse().expect("uuid");

    // GET should round-trip
    let (status, body) = req(app.clone(), "GET", &format!("/fhir/Patient/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"][0]["family"], "Lovelace");
    assert_eq!(body["birthDate"], "1990-04-12");

    // PUT replaces; id is preserved
    let (status, body) = req(
        app.clone(),
        "PUT",
        &format!("/fhir/Patient/{id}"),
        Some(json!({
            "resourceType": "Patient",
            "id": "ignored-client-id",
            "active": true,
            "gender": "female",
            "birthDate": "1990-04-12",
            "name": [{
                "use": "official",
                "family": "Byron",
                "given": ["Ada", "Augusta"]
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT: {body}");
    assert_eq!(body["id"], id.to_string(), "id must be preserved");
    assert_eq!(body["name"][0]["family"], "Byron");

    // GET reflects the update
    let (status, body) = req(app.clone(), "GET", &format!("/fhir/Patient/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"][0]["family"], "Byron");
    assert_eq!(
        body["name"][0]["given"].as_array().unwrap().len(),
        2,
        "two given names"
    );

    // DELETE returns 204
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/fhir/Patient/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET after delete: the soft-delete keeps the row, so the patient is still
    // returned. (PAS treats soft-delete as `deleted_at IS NOT NULL`; we don't
    // hide the resource from FHIR GET in v0.1.) Just ensure it didn't 500.
    let (status, _) = req(app.clone(), "GET", &format!("/fhir/Patient/{id}"), None).await;
    assert!(status == StatusCode::OK || status == StatusCode::NOT_FOUND);

    // Validation: POST without a family name fails with 400 + OperationOutcome
    let (status, body) = req(
        app.clone(),
        "POST",
        "/fhir/Patient",
        Some(json!({
            "resourceType": "Patient",
            "active": true,
            "gender": "male",
            "name": [{"family": "", "given": [""]}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["resourceType"], "OperationOutcome");
    assert_eq!(body["issue"][0]["code"], "invalid");
}

#[tokio::test]
async fn fhir_practitioner_crud() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping fhir_practitioner_crud");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");
    let tmp = tempfile::tempdir().unwrap();
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).unwrap());
    let state = AppState::new(db, Arc::new(InMemoryEventPublisher::new())).with_search(search);
    let app = fhir_router(state.clone());

    // POST
    let (status, body) = req(
        app.clone(),
        "POST",
        "/fhir/Practitioner",
        Some(json!({
            "resourceType": "Practitioner",
            "active": true,
            "gender": "female",
            "name": [{ "family": "Marie", "given": ["Curie"] }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "POST: {body}");
    assert_eq!(body["resourceType"], "Practitioner");
    let id: Uuid = body["id"].as_str().unwrap().parse().expect("uuid");

    // GET round-trips.
    let (status, body) = req(
        app.clone(),
        "GET",
        &format!("/fhir/Practitioner/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["resourceType"], "Practitioner");
    assert_eq!(body["name"][0]["family"], "Marie");

    // PUT replaces; id is preserved.
    let (status, body) = req(
        app.clone(),
        "PUT",
        &format!("/fhir/Practitioner/{id}"),
        Some(json!({
            "resourceType": "Practitioner",
            "id": id.to_string(),
            "active": true,
            "gender": "female",
            "name": [{ "family": "Sklodowska-Curie", "given": ["Marie"] }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT: {body}");
    assert_eq!(body["id"].as_str().unwrap(), id.to_string());
    assert_eq!(body["name"][0]["family"], "Sklodowska-Curie");

    // DELETE flips active = false (soft-delete via the FHIR `active` flag).
    let (status, _) = req(
        app.clone(),
        "DELETE",
        &format!("/fhir/Practitioner/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // GET after DELETE still returns the row, with active=false.
    let (status, body) = req(
        app.clone(),
        "GET",
        &format!("/fhir/Practitioner/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["active"], false);

    // DELETE on unknown id → 404 with OperationOutcome.
    let bogus = Uuid::new_v4();
    let (status, body) = req(
        app.clone(),
        "DELETE",
        &format!("/fhir/Practitioner/{bogus}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["resourceType"], "OperationOutcome");

    // PUT on unknown id → 404.
    let (status, _) = req(
        app.clone(),
        "PUT",
        &format!("/fhir/Practitioner/{bogus}"),
        Some(json!({
            "resourceType": "Practitioner",
            "active": true,
            "gender": "male",
            "name": [{ "family": "Nobody", "given": ["X"] }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn fhir_schedule_and_slot_crud() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping fhir_schedule_and_slot_crud");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");
    let tmp = tempfile::tempdir().unwrap();
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).unwrap());
    let state = AppState::new(db, Arc::new(InMemoryEventPublisher::new())).with_search(search);
    let app = fhir_router(state.clone());

    // First create a Practitioner via FHIR so we have a real actor UUID.
    let (status, body) = req(
        app.clone(),
        "POST",
        "/fhir/Practitioner",
        Some(json!({
            "resourceType": "Practitioner",
            "active": true,
            "gender": "other",
            "name": [{ "family": "ScheduleOwner", "given": ["Dr"] }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let pract_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    // POST /fhir/Schedule
    let (status, body) = req(
        app.clone(),
        "POST",
        "/fhir/Schedule",
        Some(json!({
            "resourceType": "Schedule",
            "active": true,
            "actor": [{ "reference": format!("Practitioner/{pract_id}") }],
            "serviceType": [{ "text": "cardiology" }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "POST Schedule: {body}");
    assert_eq!(body["resourceType"], "Schedule");
    let schedule_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    // GET, then PUT replaces serviceType, then verify.
    let (status, _) = req(
        app.clone(),
        "GET",
        &format!("/fhir/Schedule/{schedule_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = req(
        app.clone(),
        "PUT",
        &format!("/fhir/Schedule/{schedule_id}"),
        Some(json!({
            "resourceType": "Schedule",
            "id": schedule_id.to_string(),
            "active": true,
            "actor": [{ "reference": format!("Practitioner/{pract_id}") }],
            "serviceType": [{ "text": "general" }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT Schedule: {body}");
    assert_eq!(body["serviceType"][0]["text"], "general");

    // POST /fhir/Slot
    let (status, body) = req(
        app.clone(),
        "POST",
        "/fhir/Slot",
        Some(json!({
            "resourceType": "Slot",
            "schedule": { "reference": format!("Schedule/{schedule_id}") },
            "status": "free",
            "start": "2027-06-05T14:30:00Z",
            "end": "2027-06-05T15:00:00Z"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "POST Slot: {body}");
    let slot_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(body["status"], "free");

    // PUT replaces status.
    let (status, body) = req(
        app.clone(),
        "PUT",
        &format!("/fhir/Slot/{slot_id}"),
        Some(json!({
            "resourceType": "Slot",
            "id": slot_id.to_string(),
            "schedule": { "reference": format!("Schedule/{schedule_id}") },
            "status": "busy",
            "start": "2027-06-05T14:30:00Z",
            "end": "2027-06-05T15:00:00Z"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT Slot: {body}");
    assert_eq!(body["status"], "busy");

    // DELETE Slot (hard delete).
    let (status, _) = req(
        app.clone(),
        "DELETE",
        &format!("/fhir/Slot/{slot_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = req(app.clone(), "GET", &format!("/fhir/Slot/{slot_id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // DELETE Slot again → 404.
    let (status, body) = req(
        app.clone(),
        "DELETE",
        &format!("/fhir/Slot/{slot_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["resourceType"], "OperationOutcome");

    // DELETE Schedule (hard delete).
    let (status, _) = req(
        app.clone(),
        "DELETE",
        &format!("/fhir/Schedule/{schedule_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = req(
        app.clone(),
        "GET",
        &format!("/fhir/Schedule/{schedule_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
