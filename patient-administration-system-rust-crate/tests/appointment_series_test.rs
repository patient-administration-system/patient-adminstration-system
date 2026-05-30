//! Integration test for recurring appointment series (v0.9.0).
//!
//! Walks: preview → create → fetch → cancel → verify state-machine
//! pass-through on the contained appointments, plus an atomic-overlap-
//! reject test that asserts no rows survive when a conflict aborts the
//! create transaction.
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
            family: format!("Series{}", Uuid::new_v4().simple()),
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
async fn appointment_series_preview_then_create_then_cancel() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping appointment_series_preview_then_create_then_cancel"
            );
            return;
        }
    };
    let state = common::build_state(&url).await;
    migration::Migrator::up(&state.db, None)
        .await
        .expect("migrate");
    let patient_id = insert_patient(&state).await;
    let app = router(state.clone());

    // 2026-06-01 is a Monday at noon UTC. Weekly count=4 → four Mondays.
    let req = json!({
        "patient_id": patient_id,
        "service_type": "cardiology",
        "start_datetime": "2026-06-01T12:00:00Z",
        "duration_minutes": 30,
        "rule": {
            "frequency": "weekly",
            "interval": 1,
            "by_weekday": null,
            "end": { "kind": "count", "count": 4 }
        },
        "reason": "weekly cardio follow-up"
    });

    // --- Preview: exactly 4 datetimes, none persisted ---
    let (status, body) = post(app.clone(), "/api/appointment-series/preview", req.clone()).await;
    assert_eq!(status, StatusCode::OK, "preview: {body}");
    assert_eq!(body["data"]["total"].as_u64(), Some(4));
    assert_eq!(body["data"]["occurrences"].as_array().unwrap().len(), 4);
    // Preview must NOT have written a series row.
    let (status, body) = get(
        app.clone(),
        &format!("/api/patients/{patient_id}/appointment-series"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["data"].as_array().unwrap().is_empty(),
        "preview must not persist: {body}"
    );

    // --- Create: 1 series + 4 appointments persisted, all Booked ---
    let (status, body) = post(app.clone(), "/api/appointment-series", req).await;
    assert_eq!(status, StatusCode::OK, "create: {body}");
    let series_id: Uuid =
        serde_json::from_value(body["data"]["series"]["id"].clone()).expect("series id");
    let appointments = body["data"]["appointments"].as_array().expect("appts");
    assert_eq!(appointments.len(), 4);
    for a in appointments {
        assert_eq!(a["status"], "booked");
        assert_eq!(
            a["series_id"].as_str(),
            Some(series_id.to_string().as_str()),
            "series_id must backlink"
        );
    }

    // --- Fetch: series + occurrences round-trip ---
    let (status, body) = get(app.clone(), &format!("/api/appointment-series/{series_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["series"]["status"], "active");
    assert_eq!(body["data"]["appointments"].as_array().unwrap().len(), 4);

    // --- List per patient: 1 row ---
    let (status, body) = get(
        app.clone(),
        &format!("/api/patients/{patient_id}/appointment-series"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"].as_array().unwrap().len(), 1);

    // --- Cancel: series → cancelled, all 4 occurrences → cancelled ---
    let (status, body) = post(
        app.clone(),
        &format!("/api/appointment-series/{series_id}/cancel"),
        json!({ "reason": "rescheduled" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "cancel: {body}");
    assert_eq!(body["data"]["series"]["status"], "cancelled");
    for a in body["data"]["appointments"].as_array().unwrap() {
        assert_eq!(a["status"], "cancelled");
        assert_eq!(a["cancellation_reason"], "rescheduled");
    }
}

#[tokio::test]
async fn appointment_series_create_atomically_rejects_on_overlap() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping appointment_series_create_atomically_rejects_on_overlap"
            );
            return;
        }
    };
    let state = common::build_state(&url).await;
    migration::Migrator::up(&state.db, None)
        .await
        .expect("migrate");
    let patient_id = insert_patient(&state).await;
    let app = router(state.clone());

    // Seed one singleton appointment on 2026-06-08 12:00–12:30 — the 2nd
    // occurrence of our weekly series will collide with it.
    use patient_administration_system::db::repositories::appointment::AppointmentRepository;
    use patient_administration_system::models::appointment::Appointment;
    let mut blocker = Appointment::new(
        patient_id,
        chrono::DateTime::parse_from_rfc3339("2026-06-08T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        chrono::DateTime::parse_from_rfc3339("2026-06-08T12:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
    );
    blocker.status = patient_administration_system::models::appointment::AppointmentStatus::Booked;
    AppointmentRepository::create(&state.db, &blocker)
        .await
        .expect("seed blocker");

    let req = json!({
        "patient_id": patient_id,
        "service_type": "cardiology",
        "start_datetime": "2026-06-01T12:00:00Z",
        "duration_minutes": 30,
        "rule": {
            "frequency": "weekly",
            "interval": 1,
            "by_weekday": null,
            "end": { "kind": "count", "count": 4 }
        }
    });

    let (status, body) = post(app.clone(), "/api/appointment-series", req).await;
    assert_eq!(status, StatusCode::CONFLICT, "should 409: {body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("overlaps"),
        "diagnostic should mention overlap: {body}"
    );

    // Atomic: no series rows were created.
    let (_, body) = get(
        app.clone(),
        &format!("/api/patients/{patient_id}/appointment-series"),
    )
    .await;
    assert!(
        body["data"].as_array().unwrap().is_empty(),
        "no series row should survive an aborted create: {body}"
    );
}
