//! Integration test for the scheduling flow: book → check-in → complete.
//!
//! Gated on `DATABASE_URL`. Sets up a schedule with a single free slot,
//! drives the REST API to book it, checks the patient in, and marks it
//! fulfilled. Asserts the appointment status transitions correctly and
//! the slot is busy after booking.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::rest::router;
use patient_administration_system::db::entities::{patient, schedule, slot};
use patient_administration_system::models::Gender;
use patient_administration_system::models::patient::{HumanName, Patient};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

async fn read_body(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes).expect("json body")
}

async fn post(
    app: axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
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
        .expect("response");
    let status = resp.status();
    let json = read_body(resp).await;
    (status, json)
}

#[tokio::test]
async fn book_check_in_complete_flow() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping book_check_in_complete_flow");
            return;
        }
    };
    let state = common::build_state(&url).await;
    migration::Migrator::up(&state.db, None)
        .await
        .expect("migrations up");

    let now = chrono::Utc::now().fixed_offset();
    let practitioner_id = Uuid::new_v4();

    // schedule owned by a Practitioner
    let schedule_id = Uuid::new_v4();
    schedule::ActiveModel {
        id: Set(schedule_id),
        owner_kind: Set("Practitioner".into()),
        owner_id: Set(practitioner_id),
        service_type: Set("cardiology".into()),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .expect("insert schedule");

    // single free slot, starts in 1 hour, 30 min long
    let slot_id = Uuid::new_v4();
    let start = chrono::Utc::now() + chrono::Duration::hours(1);
    let end = start + chrono::Duration::minutes(30);
    slot::ActiveModel {
        id: Set(slot_id),
        schedule_id: Set(schedule_id),
        start_datetime: Set(start.fixed_offset()),
        end_datetime: Set(end.fixed_offset()),
        status: Set("free".into()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .expect("insert slot");

    // patient
    let p = Patient::new(
        HumanName {
            use_type: None,
            family: "Roe".into(),
            given: vec!["Sam".into()],
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
        additional_names: Set(serde_json::json!([])),
        identifiers: Set(serde_json::json!([])),
        telecom: Set(serde_json::json!([])),
        addresses: Set(serde_json::json!([])),
        gender: Set("other".into()),
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

    // Book
    let (status, body) = post(
        app.clone(),
        &format!("/api/slots/{slot_id}/book"),
        json!({ "patient_id": patient_id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "book body={body}");
    let appt_id: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("appointment id");
    assert_eq!(body["data"]["status"], "booked");

    // Slot is now busy
    let s = slot::Entity::find_by_id(slot_id)
        .one(&state.db)
        .await
        .expect("find slot")
        .expect("slot exists");
    assert_eq!(s.status, "busy");

    // Check-in
    let (status, body) = post(
        app.clone(),
        &format!("/api/appointments/{appt_id}/check-in"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "check-in body={body}");
    assert_eq!(body["data"]["status"], "arrived");

    // Complete
    let (status, body) = post(
        app.clone(),
        &format!("/api/appointments/{appt_id}/complete"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "complete body={body}");
    assert_eq!(body["data"]["status"], "fulfilled");
}
