//! Full ADT flow integration test: admit → transfer → discharge.
//!
//! Gated on `DATABASE_URL`. Applies migrations before running so the schema
//! is current. Uses the in-process Axum router via `tower::ServiceExt::oneshot`
//! to avoid binding a real socket.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::rest::router;
use patient_administration_system::db::entities::{bed, facility, patient, room, ward};
use patient_administration_system::models::patient::{HumanName, Patient};
use patient_administration_system::models::{Address, Gender};
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
async fn admit_transfer_discharge_full_flow() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping admit_transfer_discharge_full_flow");
            return;
        }
    };
    let state = common::build_state(&url).await;

    migration::Migrator::up(&state.db, None)
        .await
        .expect("migrations up");

    // ----- setup: facility/ward/room/3 beds + patient -----
    let now = chrono::Utc::now().fixed_offset();
    let address_json = serde_json::to_value(Address {
        use_type: None,
        line1: Some("1 Main St".into()),
        line2: None,
        city: Some("Townsville".into()),
        state: Some("TS".into()),
        postal_code: Some("00001".into()),
        country: Some("US".into()),
    })
    .unwrap();

    let facility_id = Uuid::new_v4();
    facility::ActiveModel {
        id: Set(facility_id),
        name: Set("Test Hospital".into()),
        code: Set(format!("TH-{}", Uuid::new_v4().simple())),
        address: Set(address_json.clone()),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .expect("insert facility");

    let ward_id = Uuid::new_v4();
    ward::ActiveModel {
        id: Set(ward_id),
        facility_id: Set(facility_id),
        name: Set("Ward A".into()),
        code: Set(format!("W-{}", Uuid::new_v4().simple())),
        capacity: Set(10),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .expect("insert ward");

    let room_id = Uuid::new_v4();
    room::ActiveModel {
        id: Set(room_id),
        ward_id: Set(ward_id),
        name: Set("Room 1".into()),
        code: Set(format!("R-{}", Uuid::new_v4().simple())),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .expect("insert room");

    let mut bed_ids = Vec::new();
    for n in 1..=3 {
        let bed_id = Uuid::new_v4();
        bed::ActiveModel {
            id: Set(bed_id),
            room_id: Set(room_id),
            name: Set(format!("Bed {n}")),
            code: Set(format!("B-{}", Uuid::new_v4().simple())),
            status: Set("available".into()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&state.db)
        .await
        .expect("insert bed");
        bed_ids.push(bed_id);
    }

    let p = Patient::new(
        HumanName {
            use_type: None,
            family: "Doe".into(),
            given: vec!["Jane".into()],
            prefix: vec![],
            suffix: vec![],
        },
        Gender::Female,
    );
    let patient_id = p.id;
    let patient_json = patient::ActiveModel {
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
    };
    patient_json
        .insert(&state.db)
        .await
        .expect("insert patient");

    // ----- flow: admit, transfer, discharge -----
    let app = router(state.clone());

    let (status, body) = post(
        app.clone(),
        "/api/admissions",
        json!({ "patient_id": patient_id, "bed_id": bed_ids[0] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admit body={body}");
    assert_eq!(body["success"], true);
    let admission_id: Uuid =
        serde_json::from_value(body["data"]["admission"]["id"].clone()).expect("admission id");

    let (status, body) = post(
        app.clone(),
        &format!("/api/admissions/{admission_id}/transfer"),
        json!({ "new_bed_id": bed_ids[1] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "transfer body={body}");
    assert_eq!(body["success"], true);

    let (status, body) = post(
        app.clone(),
        &format!("/api/admissions/{admission_id}/discharge"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "discharge body={body}");
    assert_eq!(body["success"], true);

    // ----- assert final bed states -----
    let beds: Vec<bed::Model> = bed::Entity::find().all(&state.db).await.expect("list beds");
    let by_id: std::collections::HashMap<_, _> =
        beds.into_iter().map(|b| (b.id, b.status)).collect();
    assert_eq!(
        by_id.get(&bed_ids[0]).unwrap(),
        "cleaning",
        "bed0 should be cleaning after transfer",
    );
    assert_eq!(
        by_id.get(&bed_ids[1]).unwrap(),
        "cleaning",
        "bed1 should be cleaning after discharge",
    );
    assert_eq!(
        by_id.get(&bed_ids[2]).unwrap(),
        "available",
        "bed2 untouched",
    );
}
