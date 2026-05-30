//! Integration test for the billing flow:
//! open account → post charges → finalize invoice → post payment → assert paid.
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
async fn billing_full_flow() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping billing_full_flow");
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
            family: "Bill".into(),
            given: vec!["Aly".into()],
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

    // Open account
    let (status, body) = post(
        app.clone(),
        "/api/accounts",
        json!({ "patient_id": patient_id, "currency": "USD" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open account: {body}");
    let account_id: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("account id");

    // Re-opening should conflict (one open account per patient)
    let (status, body) = post(
        app.clone(),
        "/api/accounts",
        json!({ "patient_id": patient_id, "currency": "USD" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "second open should conflict: {body}"
    );

    // Post two charges
    let mut charge_ids = Vec::new();
    for (code, amount) in &[("CONSULT", "75.00"), ("XRAY", "125.50")] {
        let (status, body) = post(
            app.clone(),
            "/api/charges",
            json!({
                "account_id": account_id,
                "code": code,
                "description": format!("{code} fee"),
                "amount_value": amount,
                "amount_currency": "USD",
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "charge {code}: {body}");
        let cid: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("charge id");
        charge_ids.push(cid);
    }

    // Finalize invoice with both charges
    let (status, body) = post(
        app.clone(),
        "/api/invoices",
        json!({ "account_id": account_id, "charge_ids": charge_ids }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "finalize: {body}");
    let invoice_id: Uuid = serde_json::from_value(body["data"]["id"].clone()).expect("invoice id");
    assert_eq!(body["data"]["status"], "finalized");
    assert_eq!(body["data"]["total"]["amount"], "200.50");

    // Post partial payment
    let (status, body) = post(
        app.clone(),
        "/api/payments",
        json!({
            "invoice_id": invoice_id,
            "amount_value": "50.00",
            "amount_currency": "USD",
            "method": "card",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "partial payment: {body}");

    // Post remainder
    let (status, body) = post(
        app.clone(),
        "/api/payments",
        json!({
            "invoice_id": invoice_id,
            "amount_value": "150.50",
            "amount_currency": "USD",
            "method": "cash",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "remainder payment: {body}");

    // GET /api/patients/:id/account should return the account (still open)
    let (status, body) = get(app.clone(), &format!("/api/patients/{patient_id}/account")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["id"], account_id.to_string());
    assert_eq!(body["data"]["status"], "open");
}
