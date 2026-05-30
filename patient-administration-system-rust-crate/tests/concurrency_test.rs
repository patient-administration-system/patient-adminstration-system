//! Concurrency integration tests.
//!
//! Asserts the `SELECT … FOR UPDATE` machinery actually prevents double-
//! booking on shared resources (beds, slots). Gated on `DATABASE_URL`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::rest::router;
use patient_administration_system::db::entities::{
    bed, facility, patient, room, schedule, slot, ward,
};
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
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

async fn post_json(
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
        .unwrap();
    let status = resp.status();
    (status, body_json(resp).await)
}

async fn insert_patient(db: &sea_orm::DatabaseConnection, family: &str, given: &str) -> Uuid {
    let now = chrono::Utc::now().fixed_offset();
    let p = Patient::new(
        HumanName {
            use_type: None,
            family: family.into(),
            given: vec![given.into()],
            prefix: vec![],
            suffix: vec![],
        },
        Gender::Unknown,
    );
    let pid = p.id;
    patient::ActiveModel {
        id: Set(pid),
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
    .insert(db)
    .await
    .expect("insert patient");
    pid
}

#[tokio::test]
async fn concurrent_admit_to_same_bed_only_one_succeeds() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping concurrent_admit_to_same_bed_only_one_succeeds"
            );
            return;
        }
    };
    let state = common::build_state(&url).await;
    migration::Migrator::up(&state.db, None)
        .await
        .expect("migrate");
    let app = router(state.clone());

    // Setup: facility/ward/room/single bed
    let now = chrono::Utc::now().fixed_offset();
    let fid = Uuid::new_v4();
    facility::ActiveModel {
        id: Set(fid),
        name: Set("Conc Hospital".into()),
        code: Set(format!("CH-{}", Uuid::new_v4().simple())),
        address: Set(serde_json::json!({"city": "TC"})),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .unwrap();
    let wid = Uuid::new_v4();
    ward::ActiveModel {
        id: Set(wid),
        facility_id: Set(fid),
        name: Set("Ward".into()),
        code: Set(format!("W-{}", Uuid::new_v4().simple())),
        capacity: Set(1),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .unwrap();
    let rid = Uuid::new_v4();
    room::ActiveModel {
        id: Set(rid),
        ward_id: Set(wid),
        name: Set("R".into()),
        code: Set(format!("R-{}", Uuid::new_v4().simple())),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .unwrap();
    let bed_id = Uuid::new_v4();
    bed::ActiveModel {
        id: Set(bed_id),
        room_id: Set(rid),
        name: Set("B".into()),
        code: Set(format!("B-{}", Uuid::new_v4().simple())),
        status: Set("available".into()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .unwrap();

    let p1 = insert_patient(&state.db, "ConA", "One").await;
    let p2 = insert_patient(&state.db, "ConB", "Two").await;

    // Two tokio tasks race for the same bed.
    let app1 = app.clone();
    let app2 = app.clone();
    let t1 = tokio::spawn(async move {
        post_json(
            app1,
            "/api/admissions",
            json!({"patient_id": p1, "bed_id": bed_id}),
        )
        .await
    });
    let t2 = tokio::spawn(async move {
        post_json(
            app2,
            "/api/admissions",
            json!({"patient_id": p2, "bed_id": bed_id}),
        )
        .await
    });
    let (r1, r2) = tokio::join!(t1, t2);
    let (s1, _) = r1.unwrap();
    let (s2, _) = r2.unwrap();

    // Exactly one OK, the other 409 Conflict (bed not available).
    let oks = [s1, s2].iter().filter(|s| **s == StatusCode::OK).count();
    let conflicts = [s1, s2]
        .iter()
        .filter(|s| **s == StatusCode::CONFLICT)
        .count();
    assert_eq!(
        oks, 1,
        "exactly one admit should succeed; got s1={s1} s2={s2}"
    );
    assert_eq!(
        conflicts, 1,
        "the other must be 409 Conflict; got s1={s1} s2={s2}"
    );
}

#[tokio::test]
async fn concurrent_slot_book_only_one_succeeds() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping concurrent_slot_book_only_one_succeeds");
            return;
        }
    };
    let state = common::build_state(&url).await;
    migration::Migrator::up(&state.db, None)
        .await
        .expect("migrate");
    let app = router(state.clone());

    let now = chrono::Utc::now().fixed_offset();
    let practitioner_id = Uuid::new_v4();
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
    .unwrap();

    let slot_id = Uuid::new_v4();
    let start = chrono::Utc::now() + chrono::Duration::hours(2);
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
    .unwrap();

    let p1 = insert_patient(&state.db, "BookA", "One").await;
    let p2 = insert_patient(&state.db, "BookB", "Two").await;

    let app1 = app.clone();
    let app2 = app.clone();
    let t1 = tokio::spawn(async move {
        post_json(
            app1,
            &format!("/api/slots/{slot_id}/book"),
            json!({"patient_id": p1}),
        )
        .await
    });
    let t2 = tokio::spawn(async move {
        post_json(
            app2,
            &format!("/api/slots/{slot_id}/book"),
            json!({"patient_id": p2}),
        )
        .await
    });
    let (r1, r2) = tokio::join!(t1, t2);
    let (s1, _) = r1.unwrap();
    let (s2, _) = r2.unwrap();

    let oks = [s1, s2].iter().filter(|s| **s == StatusCode::OK).count();
    let conflicts = [s1, s2]
        .iter()
        .filter(|s| **s == StatusCode::CONFLICT)
        .count();
    assert_eq!(
        oks, 1,
        "exactly one booking should succeed; got s1={s1} s2={s2}"
    );
    assert_eq!(conflicts, 1, "the other must be 409; got s1={s1} s2={s2}");
}
