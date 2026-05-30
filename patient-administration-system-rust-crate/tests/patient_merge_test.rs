//! Integration test for the v0.11 Patient merge / tombstone flow.
//!
//! Walks: create A + B → merge A → B → fetch A (tombstoned) →
//! fetch B (untouched) → GET /api/patients/B/replaces returns A →
//! FHIR /fhir/Patient/A carries Patient.link[replaced-by] → search for
//! A's family name returns 0 hits → reject self-merge → reject
//! double-merge.
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

async fn post_empty(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .body(Body::empty())
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

async fn insert_patient(
    state: &patient_administration_system::api::rest::AppState,
    family: &str,
) -> Uuid {
    let now = chrono::Utc::now().fixed_offset();
    let p = Patient::new(
        HumanName {
            use_type: None,
            family: family.into(),
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
async fn patient_merge_full_lifecycle_with_fhir_link_and_search_drop() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping patient_merge_full_lifecycle_with_fhir_link_and_search_drop"
            );
            return;
        }
    };
    // build_state by itself omits the search engine. Wire a real Tantivy
    // index so the test can assert the source row is dropped on merge.
    use std::sync::Arc;
    let base = common::build_state(&url).await;
    migration::Migrator::up(&base.db, None)
        .await
        .expect("migrate");
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(
        patient_administration_system::search::SearchEngine::new(tmp.path().to_str().unwrap())
            .expect("search"),
    );
    let state = base.with_search(search);
    let app = router(state.clone());
    let fhir = fhir_router(state.clone());

    let suffix = Uuid::new_v4().simple().to_string();
    let family_a = format!("Merge{suffix}A");
    let family_b = format!("Merge{suffix}B");
    let a_id = insert_patient(&state, &family_a).await;
    let b_id = insert_patient(&state, &family_b).await;

    // Index both into Tantivy so we can assert the source is dropped
    // after the merge.
    let pa =
        patient_administration_system::db::repositories::patient::PatientRepository::find_by_id(
            &state.db, a_id,
        )
        .await
        .unwrap()
        .unwrap();
    let pb =
        patient_administration_system::db::repositories::patient::PatientRepository::find_by_id(
            &state.db, b_id,
        )
        .await
        .unwrap()
        .unwrap();
    state
        .search
        .as_ref()
        .unwrap()
        .index_patient(&pa)
        .expect("index a");
    state
        .search
        .as_ref()
        .unwrap()
        .index_patient(&pb)
        .expect("index b");

    // --- Happy-path merge A → B ---
    let (status, body) = post_empty(
        app.clone(),
        &format!("/api/patients/{a_id}/merge-into/{b_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "merge: {body}");
    assert_eq!(
        body["data"]["replaced_by"].as_str(),
        Some(b_id.to_string().as_str())
    );
    assert_eq!(body["data"]["active"], false);

    // --- Fetch A: tombstone with replaced_by set ---
    let (status, body) = get(app.clone(), &format!("/api/patients/{a_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["data"]["replaced_by"].as_str(),
        Some(b_id.to_string().as_str())
    );
    assert_eq!(body["data"]["active"], false);

    // --- Fetch B: unchanged, no replaced_by ---
    let (status, body) = get(app.clone(), &format!("/api/patients/{b_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["data"]["replaced_by"].is_null());
    assert_eq!(body["data"]["active"], true);

    // --- B.replaces lists A ---
    let (status, body) = get(app.clone(), &format!("/api/patients/{b_id}/replaces")).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().expect("array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"].as_str(), Some(a_id.to_string().as_str()));

    // --- FHIR Patient/A carries Patient.link[replaced-by] ---
    let (status, body) = get(fhir.clone(), &format!("/fhir/Patient/{a_id}")).await;
    assert_eq!(status, StatusCode::OK, "fhir read: {body}");
    assert_eq!(body["resourceType"], "Patient");
    assert_eq!(body["link"][0]["type"], "replaced-by");
    assert_eq!(
        body["link"][0]["other"]["reference"].as_str(),
        Some(format!("Patient/{b_id}").as_str())
    );

    // --- FHIR Patient/B: no link emitted ---
    let (status, body) = get(fhir.clone(), &format!("/fhir/Patient/{b_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["link"].is_null()
            || body["link"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(false),
        "survivor must not emit a link: {body}"
    );

    // --- Search for A's family name returns 0 hits (Tantivy dropped) ---
    let (status, body) = get(
        app.clone(),
        &format!("/api/patients/search?q={family_a}&limit=10"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let hits = body["data"].as_array().expect("hits");
    assert!(
        hits.iter()
            .all(|p| p["id"].as_str() != Some(a_id.to_string().as_str())),
        "tombstone must not appear in search results: {body}"
    );

    // --- Self-merge is rejected with 400 ---
    let (status, body) = post_empty(
        app.clone(),
        &format!("/api/patients/{b_id}/merge-into/{b_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("themselves"),
        "diagnostic should mention self-merge: {body}"
    );

    // --- Re-merging A (already a tombstone) is rejected with 409 ---
    let (status, body) = post_empty(
        app.clone(),
        &format!("/api/patients/{a_id}/merge-into/{b_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("already merged"),
        "diagnostic should mention prior merge: {body}"
    );

    // --- Merging into a non-existent target is rejected with 404 ---
    let c_id = insert_patient(&state, "ToBeMergedC").await;
    let nonexistent = Uuid::new_v4();
    let (status, body) = post_empty(
        app.clone(),
        &format!("/api/patients/{c_id}/merge-into/{nonexistent}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "missing target: {body}");
}
