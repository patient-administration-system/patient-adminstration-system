//! Integration test for `POST /fhir` — FHIR R5 batch / transaction Bundle.
//!
//! Gated on `DATABASE_URL`. Skips silently otherwise.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::fhir::fhir_router;
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::connect;
use patient_administration_system::db::entities::patient;
use patient_administration_system::models::Gender;
use patient_administration_system::models::patient::{HumanName, Patient};
use patient_administration_system::streaming::InMemoryEventPublisher;
use sea_orm::{ActiveModelTrait, Set};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn post_bundle(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/fhir")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "expected JSON, status={status} body={}, err={e}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, value)
}

#[tokio::test]
async fn fhir_bundle_transaction_creates_patients() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping fhir_bundle_transaction_creates_patients");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let state = AppState::new(db, publisher);
    let app = fhir_router(state);

    // --- transaction bundle of two patient creates ---
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            {
                "fullUrl": "urn:uuid:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                "resource": {
                    "resourceType": "Patient",
                    "active": true,
                    "name": [{ "family": "BundleAlpha", "given": ["Alpha"] }],
                    "gender": "female"
                },
                "request": { "method": "POST", "url": "Patient" }
            },
            {
                "fullUrl": "urn:uuid:bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
                "resource": {
                    "resourceType": "Patient",
                    "active": true,
                    "name": [{ "family": "BundleBravo", "given": ["Bravo"] }],
                    "gender": "male"
                },
                "request": { "method": "POST", "url": "Patient" }
            }
        ]
    });
    let (status, payload) = post_bundle(&app, bundle).await;
    assert_eq!(status, StatusCode::OK, "transaction: {payload}");
    assert_eq!(payload["resourceType"], "Bundle");
    assert_eq!(payload["type"], "transaction-response");
    let entries = payload["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 2);
    for e in entries {
        let resp = &e["response"];
        assert_eq!(resp["status"], "201 Created", "entry response: {resp}");
        let location = resp["location"].as_str().expect("location");
        assert!(location.starts_with("Patient/"), "location: {location}");
    }

    // --- batch bundle: an invalid entry alongside a valid one ---
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "batch",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "active": true,
                    "name": [{ "family": "BundleCharlie", "given": ["Charlie"] }],
                    "gender": "other"
                },
                "request": { "method": "POST", "url": "Patient" }
            },
            {
                "resource": {
                    "resourceType": "Patient",
                    "active": true,
                    "name": [],
                    "gender": "unknown"
                },
                "request": { "method": "POST", "url": "Patient" }
            }
        ]
    });
    let (status, payload) = post_bundle(&app, bundle).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["type"], "batch-response");
    let entries = payload["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["response"]["status"], "201 Created");
    let second_status = entries[1]["response"]["status"].as_str().expect("status");
    assert!(
        second_status.starts_with("400"),
        "expected 4xx for empty-name patient, got {second_status:?}"
    );

    // --- unsupported bundle type → 400 with OperationOutcome ---
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "history",
        "entry": []
    });
    let (status, payload) = post_bundle(&app, bundle).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(payload["resourceType"], "OperationOutcome");
}

#[tokio::test]
async fn fhir_bundle_transaction_rolls_back_atomically_on_failure() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping fhir_bundle_transaction_rolls_back_atomically_on_failure"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let state = AppState::new(db, publisher);
    let app = fhir_router(state);

    // Pick a probe family name that no prior test row can have. We will
    // search for it after the (rolled-back) transaction to assert no row
    // survived.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let probe_family = format!("TxnProbe{suffix}");

    // Transaction whose second entry is invalid (empty `name`). The first
    // entry (with probe_family) must be rolled back atomically.
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            {
                "fullUrl": "urn:uuid:cccccccc-cccc-4ccc-8ccc-cccccccccccc",
                "resource": {
                    "resourceType": "Patient",
                    "active": true,
                    "name": [{ "family": probe_family, "given": ["First"] }],
                    "gender": "female"
                },
                "request": { "method": "POST", "url": "Patient" }
            },
            {
                "fullUrl": "urn:uuid:dddddddd-dddd-4ddd-8ddd-dddddddddddd",
                "resource": {
                    "resourceType": "Patient",
                    "active": true,
                    "name": [],
                    "gender": "unknown"
                },
                "request": { "method": "POST", "url": "Patient" }
            }
        ]
    });
    let (status, payload) = post_bundle(&app, bundle).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "transaction must 400: {payload}"
    );
    assert_eq!(payload["resourceType"], "OperationOutcome");
    let diag = payload["issue"][0]["diagnostics"]
        .as_str()
        .expect("diagnostics");
    assert!(
        diag.contains("entry 1"),
        "expected rollback diagnostic naming offending entry index, got: {diag:?}"
    );

    // Verify the rollback: probe family must not be findable via the FHIR
    // Patient Bundle.
    let req = Request::builder()
        .method("GET")
        .uri("/fhir/Patient?_count=500")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let collection: Value = serde_json::from_slice(&bytes).unwrap();
    let entries = collection["entry"].as_array().cloned().unwrap_or_default();
    let found = entries.iter().any(|e| {
        e["resource"]["name"][0]["family"]
            .as_str()
            .map(|s| s == probe_family)
            .unwrap_or(false)
    });
    assert!(
        !found,
        "rolled-back transaction must not leave the first entry visible; found {probe_family} in {} entries",
        entries.len()
    );

    // --- Happy-path transaction (all entries valid) still commits. ---
    let happy_family = format!("TxnHappy{suffix}");
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "active": true,
                    "name": [{ "family": happy_family, "given": ["A"] }],
                    "gender": "female"
                },
                "request": { "method": "POST", "url": "Patient" }
            },
            {
                "resource": {
                    "resourceType": "Patient",
                    "active": true,
                    "name": [{ "family": happy_family, "given": ["B"] }],
                    "gender": "male"
                },
                "request": { "method": "POST", "url": "Patient" }
            }
        ]
    });
    let (status, payload) = post_bundle(&app, bundle).await;
    assert_eq!(status, StatusCode::OK, "happy txn: {payload}");
    assert_eq!(payload["type"], "transaction-response");
    let entries = payload["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 2);
    for e in entries {
        assert_eq!(e["response"]["status"], "201 Created");
    }
}

/// Insert a bare patient straight via SeaORM so the Coverage tests can
/// point at a real UUID without going through `POST /api/patients`.
async fn insert_patient_for_coverage(state: &AppState) -> Uuid {
    let now = chrono::Utc::now().fixed_offset();
    let p = Patient::new(
        HumanName {
            use_type: None,
            family: format!("CovBundle{}", Uuid::new_v4().simple()),
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

/// v0.13: `POST /fhir` batch + transaction bundles now accept
/// `Coverage` resource entries alongside Patient/Encounter/Appointment.
#[tokio::test]
async fn fhir_bundle_creates_coverage_entries() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping fhir_bundle_creates_coverage_entries");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let state = AppState::new(db, publisher);
    let app = fhir_router(state.clone());

    let patient_id = insert_patient_for_coverage(&state).await;

    // Batch bundle: two Coverage entries — primary insurance + secondary
    // self-pay — both reference the same patient.
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "batch",
        "entry": [
            {
                "resource": {
                    "resourceType": "Coverage",
                    "status": "active",
                    "type": { "text": "insurance" },
                    "beneficiary": { "reference": format!("Patient/{patient_id}") },
                    "subscriberId": "POL-PRIMARY-001",
                    "relationship": { "text": "self" },
                    "period": { "start": "2026-01-01" },
                    "payor": [{ "display": "Aetna", "identifier": { "value": "AET-EIN-1" } }]
                },
                "request": { "method": "POST", "url": "Coverage" }
            },
            {
                "resource": {
                    "resourceType": "Coverage",
                    "status": "active",
                    "type": { "text": "self_pay" },
                    "beneficiary": { "reference": format!("Patient/{patient_id}") },
                    "subscriberId": "SELF-PAY",
                    "relationship": { "text": "self" },
                    "period": { "start": "2026-01-01" },
                    "payor": [{ "display": "Self-pay" }]
                },
                "request": { "method": "POST", "url": "Coverage" }
            }
        ]
    });
    let (status, payload) = post_bundle(&app, bundle).await;
    assert_eq!(status, StatusCode::OK, "batch: {payload}");
    assert_eq!(payload["type"], "batch-response");
    let entries = payload["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 2);
    for e in entries {
        assert_eq!(e["response"]["status"], "201 Created");
        let location = e["response"]["location"].as_str().expect("location");
        assert!(
            location.starts_with("Coverage/"),
            "location must be Coverage/{{id}}; got {location}"
        );
    }

    // Coverage rows are visible to the read endpoint.
    let first_location = entries[0]["response"]["location"].as_str().unwrap();
    let coverage_id = first_location.strip_prefix("Coverage/").unwrap();
    let req = Request::builder()
        .method("GET")
        .uri(format!("/fhir/Coverage/{coverage_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let read_back: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(read_back["resourceType"], "Coverage");
    assert_eq!(read_back["status"], "active");
    assert_eq!(read_back["subscriberId"], "POL-PRIMARY-001");
}

/// v0.13: a Coverage entry inside a `type: transaction` bundle rolls
/// back the whole bundle when validation fails. Companion to the
/// existing Patient-rollback test above.
#[tokio::test]
async fn fhir_bundle_transaction_rolls_back_when_coverage_invalid() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping fhir_bundle_transaction_rolls_back_when_coverage_invalid"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let state = AppState::new(db, publisher);
    let app = fhir_router(state.clone());

    let patient_id = insert_patient_for_coverage(&state).await;
    let probe_policy = format!("ROLLBACK-{}", Uuid::new_v4().simple());

    // Transaction: first entry is a valid Coverage with `probe_policy`;
    // the second entry is malformed (missing required payor[0].display).
    // The whole transaction must roll back, so `probe_policy` must not
    // surface afterwards.
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            {
                "resource": {
                    "resourceType": "Coverage",
                    "status": "active",
                    "type": { "text": "insurance" },
                    "beneficiary": { "reference": format!("Patient/{patient_id}") },
                    "subscriberId": probe_policy,
                    "relationship": { "text": "self" },
                    "period": { "start": "2026-02-01" },
                    "payor": [{ "display": "BCBS" }]
                },
                "request": { "method": "POST", "url": "Coverage" }
            },
            {
                "resource": {
                    "resourceType": "Coverage",
                    "status": "active",
                    "type": { "text": "insurance" },
                    "beneficiary": { "reference": format!("Patient/{patient_id}") },
                    "subscriberId": "WILL-NOT-PERSIST",
                    "relationship": { "text": "self" },
                    "period": { "start": "2026-02-01" },
                    "payor": [{ }]
                },
                "request": { "method": "POST", "url": "Coverage" }
            }
        ]
    });
    let (status, payload) = post_bundle(&app, bundle).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "transaction must 400: {payload}"
    );
    assert_eq!(payload["resourceType"], "OperationOutcome");
    let diag = payload["issue"][0]["diagnostics"]
        .as_str()
        .expect("diagnostics");
    assert!(
        diag.contains("entry 1"),
        "expected rollback diagnostic naming offending entry index, got: {diag:?}"
    );

    // Verify the rollback: no Coverage row exists with our probe policy.
    use patient_administration_system::db::repositories::coverage::CoverageRepository;
    let rows = CoverageRepository::list_by_patient(&state.db, patient_id)
        .await
        .expect("list_by_patient");
    assert!(
        rows.iter().all(|r| r.policy_number != probe_policy),
        "rolled-back transaction must not leave the first coverage entry visible"
    );
}

/// v0.23: a single `POST /fhir` Bundle (transaction) can create a
/// Practitioner, a Schedule that references it, and a Slot that
/// references the Schedule — all in one atomic transaction.
#[tokio::test]
async fn fhir_bundle_creates_practitioner_schedule_slot_entries() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping fhir_bundle_creates_practitioner_schedule_slot_entries"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let state = AppState::new(db, publisher);
    let app = fhir_router(state.clone());

    // Step 1: create a Practitioner via a batch bundle (server assigns
    // the id; we capture it from the Location header in the response).
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let family = format!("Bundle{suffix}");
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "batch",
        "entry": [
            {
                "resource": {
                    "resourceType": "Practitioner",
                    "active": true,
                    "gender": "female",
                    "name": [{ "family": family, "given": ["Bundle"] }]
                },
                "request": { "method": "POST", "url": "Practitioner" }
            }
        ]
    });
    let (status, payload) = post_bundle(&app, bundle).await;
    assert_eq!(status, StatusCode::OK, "batch: {payload}");
    let entries = payload["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["response"]["status"], "201 Created");
    let pract_location = entries[0]["response"]["location"].as_str().unwrap();
    let pract_id = pract_location
        .strip_prefix("Practitioner/")
        .expect("Practitioner/<uuid>");

    // Step 2: in a single transaction Bundle, create a Schedule
    // (referencing the Practitioner) and a Slot (referencing the
    // Schedule). The Slot can't reference the Schedule by id unless
    // the Schedule has an id at the time we build the message, so
    // we pre-assign a UUID client-side. The server accepts the body
    // id when it's a valid UUID and uses it for the location header,
    // but our existing handler always overrides — so we capture the
    // returned ids instead.
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            {
                "resource": {
                    "resourceType": "Schedule",
                    "active": true,
                    "actor": [{ "reference": format!("Practitioner/{pract_id}") }],
                    "serviceType": [{ "text": "cardiology" }]
                },
                "request": { "method": "POST", "url": "Schedule" }
            }
        ]
    });
    let (status, payload) = post_bundle(&app, bundle).await;
    assert_eq!(status, StatusCode::OK, "transaction Schedule: {payload}");
    let entries = payload["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 1);
    let sched_location = entries[0]["response"]["location"].as_str().unwrap();
    let sched_id = sched_location
        .strip_prefix("Schedule/")
        .expect("Schedule/<uuid>");

    // Step 3: create a Slot referencing the Schedule, via Bundle.
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            {
                "resource": {
                    "resourceType": "Slot",
                    "schedule": { "reference": format!("Schedule/{sched_id}") },
                    "status": "free",
                    "start": "2027-06-05T14:30:00Z",
                    "end": "2027-06-05T15:00:00Z"
                },
                "request": { "method": "POST", "url": "Slot" }
            }
        ]
    });
    let (status, payload) = post_bundle(&app, bundle).await;
    assert_eq!(status, StatusCode::OK, "transaction Slot: {payload}");
    let entries = payload["entry"].as_array().expect("entry array");
    let slot_location = entries[0]["response"]["location"].as_str().unwrap();
    assert!(slot_location.starts_with("Slot/"));

    // The Slot is read-back-able through the FHIR read surface.
    let slot_id = slot_location.strip_prefix("Slot/").unwrap();
    let req = Request::builder()
        .method("GET")
        .uri(format!("/fhir/Slot/{slot_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["status"], "free");
    assert_eq!(
        v["schedule"]["reference"].as_str().unwrap(),
        format!("Schedule/{sched_id}")
    );

    // Transaction atomicity: a Slot with a malformed status field
    // should roll back the whole bundle. Use the same Schedule id to
    // make the error semantic (parse, not lookup).
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            {
                "resource": {
                    "resourceType": "Schedule",
                    "active": true,
                    "actor": [{ "reference": format!("Practitioner/{pract_id}") }],
                    "serviceType": [{ "text": "willnotpersist" }]
                },
                "request": { "method": "POST", "url": "Schedule" }
            },
            {
                "resource": {
                    "resourceType": "Slot",
                    "schedule": { "reference": format!("Schedule/{sched_id}") },
                    "status": "not-a-real-status",
                    "start": "2027-06-05T14:30:00Z",
                    "end": "2027-06-05T15:00:00Z"
                },
                "request": { "method": "POST", "url": "Slot" }
            }
        ]
    });
    let (status, payload) = post_bundle(&app, bundle).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "transaction must 400: {payload}"
    );
    let diag = payload["issue"][0]["diagnostics"]
        .as_str()
        .expect("diagnostics");
    assert!(
        diag.contains("entry 1"),
        "expected rollback diagnostic naming offending entry index, got: {diag:?}"
    );
}
