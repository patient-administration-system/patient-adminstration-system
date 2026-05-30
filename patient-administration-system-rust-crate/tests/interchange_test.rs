//! Integration test for the v0.2 bulk interchange endpoints
//! (`/api/patients/export.{json,xml,tsv}` and `/api/patients/import`).
//!
//! Gated on `DATABASE_URL`. Skips silently otherwise.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::rest::{AppState, router};
use patient_administration_system::db::connect;
use patient_administration_system::interchange::{
    PatientRow, csv::patients_from_csv, tsv::patients_from_tsv, xml::patients_from_xml,
};
use patient_administration_system::search::SearchEngine;
use patient_administration_system::streaming::InMemoryEventPublisher;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn read_text(resp: axum::response::Response) -> (StatusCode, String, String) {
    let status = resp.status();
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, ct, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn read_json(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "expected JSON, got status={status} body={}, err={e}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, value)
}

async fn post(
    app: &axum::Router,
    uri: &str,
    content_type: &str,
    body: String,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    read_json(resp).await
}

async fn get_text(app: &axum::Router, uri: &str) -> (StatusCode, String, String) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    read_text(resp).await
}

async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    read_json(resp).await
}

fn row(id: Uuid, mrn: &str, family: &str, given: &str, gender: &str) -> PatientRow {
    let mut r = PatientRow::empty();
    r.id = id.to_string();
    r.mrn = mrn.into();
    r.family_name = family.into();
    r.given_names = given.into();
    r.gender = gender.into();
    r.birth_date = "1990-01-15".into();
    r.phone = "+1-555-0100".into();
    r.email = format!("{}@example.com", family.to_lowercase());
    r.line1 = "123 Elm Street".into();
    r.city = "Springfield".into();
    r.postal_code = "62701".into();
    r.country = "US".into();
    r
}

#[tokio::test]
async fn interchange_import_export_flow() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping interchange_import_export_flow");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db, publisher).with_search(search);
    let app = router(state);

    // Use fresh UUIDs so reruns don't collide with prior test rows.
    let id_json_a = Uuid::new_v4();
    let id_json_b = Uuid::new_v4();
    let id_xml = Uuid::new_v4();
    let id_tsv = Uuid::new_v4();
    let id_csv = Uuid::new_v4();

    let json_rows = vec![
        row(id_json_a, "MRN-J-A", "JsonOne", "Alice", "female"),
        row(id_json_b, "MRN-J-B", "JsonTwo", "Bob", "male"),
    ];
    let xml_rows = vec![row(id_xml, "MRN-X-A", "XmlOne", "Xavier", "male")];
    let tsv_rows = vec![row(id_tsv, "MRN-T-A", "TsvOne", "Tina", "female")];
    let csv_rows = vec![row(id_csv, "MRN-C-A", "CsvOne", "Cara", "female")];

    // --- JSON import: 2 inserted, 0 skipped ---
    let body = serde_json::to_string(&json_rows).expect("json encode");
    let (status, payload) = post(&app, "/api/patients/import", "application/json", body).await;
    assert_eq!(status, StatusCode::OK, "json import: {payload}");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["data"]["inserted"], 2);
    assert_eq!(payload["data"]["skipped"], 0);
    assert_eq!(payload["data"]["failed"], 0);

    // --- Re-import same JSON: 0 inserted, 2 skipped (idempotent) ---
    let body = serde_json::to_string(&json_rows).expect("json encode");
    let (status, payload) = post(&app, "/api/patients/import", "application/json", body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["data"]["inserted"], 0);
    assert_eq!(payload["data"]["skipped"], 2);

    // --- XML import: 1 inserted ---
    let body = patient_administration_system::interchange::xml::patients_to_xml(&xml_rows)
        .expect("xml encode");
    let (status, payload) = post(&app, "/api/patients/import", "application/xml", body).await;
    assert_eq!(status, StatusCode::OK, "xml import: {payload}");
    assert_eq!(payload["data"]["inserted"], 1);

    // --- TSV import: 1 inserted ---
    let body = patient_administration_system::interchange::tsv::patients_to_tsv(&tsv_rows)
        .expect("tsv encode");
    let (status, payload) = post(
        &app,
        "/api/patients/import",
        "text/tab-separated-values",
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "tsv import: {payload}");
    assert_eq!(payload["data"]["inserted"], 1);

    // --- CSV import: 1 inserted ---
    let body = patient_administration_system::interchange::csv::patients_to_csv(&csv_rows)
        .expect("csv encode");
    let (status, payload) = post(&app, "/api/patients/import", "text/csv", body).await;
    assert_eq!(status, StatusCode::OK, "csv import: {payload}");
    assert_eq!(payload["data"]["inserted"], 1);

    // --- Export JSON: every imported id must be present ---
    let (status, payload) = get_json(&app, "/api/patients/export.json").await;
    assert_eq!(status, StatusCode::OK);
    let arr = payload["data"].as_array().expect("json array");
    let id_set: std::collections::HashSet<String> = arr
        .iter()
        .filter_map(|r| r["id"].as_str().map(String::from))
        .collect();
    for id in [id_json_a, id_json_b, id_xml, id_tsv, id_csv] {
        assert!(id_set.contains(&id.to_string()), "JSON export missing {id}");
    }

    // --- Export XML: parses back to PatientRow and includes our ids ---
    let (status, ct, body) = get_text(&app, "/api/patients/export.xml").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.starts_with("application/xml"), "xml content-type: {ct}");
    let xml_rows_back = patients_from_xml(&body).expect("parse xml export");
    let xml_ids: std::collections::HashSet<String> =
        xml_rows_back.iter().map(|r| r.id.clone()).collect();
    for id in [id_json_a, id_json_b, id_xml, id_tsv, id_csv] {
        assert!(xml_ids.contains(&id.to_string()), "XML export missing {id}");
    }

    // --- Export TSV: parses back and includes our ids ---
    let (status, ct, body) = get_text(&app, "/api/patients/export.tsv").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        ct.starts_with("text/tab-separated-values"),
        "tsv content-type: {ct}"
    );
    let tsv_rows_back = patients_from_tsv(&body).expect("parse tsv export");
    let tsv_ids: std::collections::HashSet<String> =
        tsv_rows_back.iter().map(|r| r.id.clone()).collect();
    for id in [id_json_a, id_json_b, id_xml, id_tsv, id_csv] {
        assert!(tsv_ids.contains(&id.to_string()), "TSV export missing {id}");
    }

    // --- Export CSV: parses back and includes our ids ---
    let (status, ct, body) = get_text(&app, "/api/patients/export.csv").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.starts_with("text/csv"), "csv content-type: {ct}");
    let csv_rows_back = patients_from_csv(&body).expect("parse csv export");
    let csv_ids: std::collections::HashSet<String> =
        csv_rows_back.iter().map(|r| r.id.clone()).collect();
    for id in [id_json_a, id_json_b, id_xml, id_tsv, id_csv] {
        assert!(csv_ids.contains(&id.to_string()), "CSV export missing {id}");
    }

    // --- Imported rows are indexed in Tantivy: search hits the JSON-imported family name ---
    let (status, payload) = get_json(&app, "/api/patients/search?q=JsonOne&limit=10").await;
    assert_eq!(status, StatusCode::OK);
    let hits = payload["data"].as_array().expect("search array");
    assert!(
        hits.iter().any(|p| p["id"] == id_json_a.to_string()),
        "imported patient should be searchable: {payload}"
    );

    // --- Garbled body → 400 with Validation error ---
    let (status, payload) = post(
        &app,
        "/api/patients/import",
        "application/json",
        "not-json".to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{payload}");
    assert_eq!(payload["success"], false);
}
