//! Integration test for the waitlist + RTT flow:
//! add → start clock → pause → resume → stop → assert weeks waiting.
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

async fn json_body(resp: axum::response::Response) -> serde_json::Value {
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
    (status, json_body(resp).await)
}

async fn get(app: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("response");
    let status = resp.status();
    (status, json_body(resp).await)
}

#[tokio::test]
async fn waitlist_rtt_full_flow() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping waitlist_rtt_full_flow");
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
            family: "Wait".into(),
            given: vec!["Pat".into()],
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

    // Add to waitlist
    let (status, body) = post(
        app.clone(),
        "/api/waitlist",
        json!({
            "patient_id": patient_id,
            "target_service": "orthopedics",
            "priority": "urgent",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "waitlist add: {body}");
    let entry_id: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("entry id");

    // List by service should include our entry
    let (status, body) = get(app.clone(), "/api/waitlist?service=orthopedics").await;
    assert_eq!(status, StatusCode::OK);
    let entries = body["data"].as_array().expect("array");
    assert!(entries.iter().any(|e| e["id"] == entry_id.to_string()));

    // Start RTT clock
    let (status, body) = post(
        app.clone(),
        "/api/rtt/start",
        json!({
            "patient_id": patient_id,
            "target_service": "orthopedics",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rtt start: {body}");
    let pathway_id: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("pathway id");

    // Pause
    let (status, _) = post(
        app.clone(),
        &format!("/api/rtt/{pathway_id}/pause"),
        json!({ "reason": "patient travelling" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Resume
    let (status, _) = post(
        app.clone(),
        &format!("/api/rtt/{pathway_id}/resume"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Stop
    let (status, _) = post(
        app.clone(),
        &format!("/api/rtt/{pathway_id}/stop"),
        json!({ "reason": "first treatment" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // weeks_waiting should be 0 (events all just now) but the endpoint should respond
    let (status, body) = get(app.clone(), &format!("/api/rtt/{pathway_id}/weeks-waiting")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"], 0);

    // List pathways for the patient — should include this one
    let (status, body) = get(app.clone(), &format!("/api/patients/{patient_id}/rtt")).await;
    assert_eq!(status, StatusCode::OK);
    let pathways = body["data"].as_array().expect("array");
    assert!(pathways.iter().any(|p| p["id"] == pathway_id.to_string()));
}
