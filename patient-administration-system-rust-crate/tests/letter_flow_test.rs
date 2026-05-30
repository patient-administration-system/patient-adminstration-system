//! Integration test for the letter generation flow:
//! create template → generate letter → fetch generated letter.
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
    (status, body_json(resp).await)
}

async fn get(app: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("response");
    let status = resp.status();
    (status, body_json(resp).await)
}

#[tokio::test]
async fn letter_generation_full_flow() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping letter_generation_full_flow");
            return;
        }
    };
    let state = common::build_state(&url).await;
    migration::Migrator::up(&state.db, None)
        .await
        .expect("migrations up");

    let now = chrono::Utc::now().fixed_offset();
    let p = Patient::new(
        HumanName {
            use_type: None,
            family: "Quinn".into(),
            given: vec!["Lee".into()],
            prefix: vec![],
            suffix: vec![],
        },
        Gender::Female,
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
        gender: Set("female".into()),
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

    // Create a template that references the patient name and an extra var
    let (status, body) = post(
        app.clone(),
        "/api/letter-templates",
        json!({
            "name": "appointment-reminder",
            "subject": "Reminder for {{ patient.name.family }}",
            "body_tera": "Dear {{ patient.name.given.0 }} {{ patient.name.family }}, your appointment is on {{ appointment_date }}.",
            "required_variables": ["appointment_date"],
            "channels": ["email"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create template: {body}");
    let template_id: Uuid =
        serde_json::from_value(body["data"]["id"].clone()).expect("template id");

    // Generate a letter
    let (status, body) = post(
        app.clone(),
        "/api/letters/generate",
        json!({
            "template_id": template_id,
            "patient_id": patient_id,
            "channel": "email",
            "extra": { "appointment_date": "2026-05-30" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "generate: {body}");
    let letter_id: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("letter id");
    assert_eq!(body["data"]["rendered_subject"], "Reminder for Quinn");
    assert_eq!(
        body["data"]["rendered_body"],
        "Dear Lee Quinn, your appointment is on 2026-05-30."
    );
    assert_eq!(body["data"]["status"], "pending");

    // Fetch it back
    let (status, body) = get(app.clone(), &format!("/api/letters/{letter_id}")).await;
    assert_eq!(status, StatusCode::OK, "get letter: {body}");
    assert_eq!(body["data"]["id"], letter_id.to_string());

    // Missing required var should fail validation
    let (status, body) = post(
        app.clone(),
        "/api/letters/generate",
        json!({
            "template_id": template_id,
            "patient_id": patient_id,
            "channel": "email",
            "extra": {}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "missing-var should be 400: {body}"
    );

    // Listing templates should include ours
    let (status, body) = get(app.clone(), "/api/letter-templates").await;
    assert_eq!(status, StatusCode::OK);
    let templates = body["data"].as_array().expect("array");
    assert!(templates.iter().any(|t| t["id"] == template_id.to_string()));
}

// ---- v0.8 SMS auto-send -------------------------------------------------

use patient_administration_system::communication::LogSmsProvider;
use std::sync::Arc as StdArc;

#[tokio::test]
async fn letter_sms_auto_sends_when_log_provider_is_wired() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping letter_sms_auto_sends_when_log_provider_is_wired"
            );
            return;
        }
    };
    // Build state with LogSmsProvider so the auto-send path actually fires.
    let base = common::build_state(&url).await;
    migration::Migrator::up(&base.db, None)
        .await
        .expect("migrate");
    let state = base.with_sms_provider(StdArc::new(LogSmsProvider));

    // Insert a patient with a Phone telecom so the auto-send path has a
    // recipient to address.
    let now = chrono::Utc::now().fixed_offset();
    let p = Patient::new(
        HumanName {
            use_type: None,
            family: format!("Sms{}", Uuid::new_v4().simple()),
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
        additional_names: Set(serde_json::json!([])),
        identifiers: Set(serde_json::json!([])),
        telecom: Set(serde_json::json!([{
            "system": "phone",
            "value": "+15555550199",
            "use_type": "mobile"
        }])),
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

    // Template that allows SMS as a channel.
    let (status, body) = post(
        app.clone(),
        "/api/letter-templates",
        json!({
            "name": "sms-reminder",
            "subject": "Reminder",
            "body_tera": "Hi {{ patient.name.given.0 }}, appt at {{ when }}.",
            "required_variables": ["when"],
            "channels": ["sms"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create template: {body}");
    let template_id: Uuid =
        serde_json::from_value(body["data"]["id"].clone()).expect("template id");

    // --- Happy path: SMS letter with a phone-equipped patient. The
    //     LogSmsProvider returns Ok, so the service flips status to Sent
    //     and stamps sent_at. ---
    let (status, body) = post(
        app.clone(),
        "/api/letters/generate",
        json!({
            "template_id": template_id,
            "patient_id": patient_id,
            "channel": "sms",
            "extra": { "when": "14:30" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "generate sms: {body}");
    assert_eq!(
        body["data"]["status"], "sent",
        "auto-send should flip status to sent: {body}"
    );
    assert!(
        body["data"]["sent_at"].is_string(),
        "sent_at must be stamped: {body}"
    );
    assert_eq!(body["data"]["channel"], "sms");

    // --- Patient with no Phone telecom: letter should be generated but
    //     stay pending (auto-send is skipped). Used to exercise the
    //     "no recipient" branch. ---
    let p2 = Patient::new(
        HumanName {
            use_type: None,
            family: format!("NoPhone{}", Uuid::new_v4().simple()),
            given: vec!["Test".into()],
            prefix: vec![],
            suffix: vec![],
        },
        Gender::Other,
    );
    let patient2_id = p2.id;
    patient::ActiveModel {
        id: Set(patient2_id),
        mpi_id: Set(None),
        active: Set(true),
        name: Set(serde_json::to_value(&p2.name).unwrap()),
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
    .expect("insert patient2");

    let (status, body) = post(
        app.clone(),
        "/api/letters/generate",
        json!({
            "template_id": template_id,
            "patient_id": patient2_id,
            "channel": "sms",
            "extra": { "when": "15:00" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "generate sms no-phone: {body}");
    assert_eq!(
        body["data"]["status"], "pending",
        "no-phone patient must leave letter pending: {body}"
    );
    assert!(
        body["data"]["sent_at"].is_null(),
        "no sent_at when nothing was sent: {body}"
    );
}
