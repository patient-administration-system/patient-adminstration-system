//! Integration test for the HL7 v2 ADT ingest endpoints.
//!
//! Gated on `DATABASE_URL`. Skips silently otherwise.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::rest::{AppState, router};
use patient_administration_system::db::connect;
use patient_administration_system::search::SearchEngine;
use patient_administration_system::streaming::InMemoryEventPublisher;
use std::sync::Arc;
use tower::ServiceExt;

async fn post_text(app: &axum::Router, uri: &str, body: String) -> (StatusCode, String, String) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/hl7-v2")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, ct, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn hl7v2_adt_a28_creates_patient_and_returns_aa_ack() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_adt_a28_creates_patient_and_returns_aa_ack"
            );
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

    // Random MRN so reruns don't collide on the unique identifier index.
    let mrn = format!("MRN-T-{}", uuid::Uuid::new_v4().simple());
    let family = format!("Hl7Test{}", uuid::Uuid::new_v4().simple());
    let msg = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A28|MSGTEST|P|2.5\r\
EVN|A28|20260523120000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F|||123 Elm^^Springfield^IL^62701^US||(555)555-0100\r"
    );

    // --- Happy path: AA ACK + patient is searchable by family name ---
    let (status, ct, body) = post_text(&app, "/api/hl7/v2/patient", msg.clone()).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(ct.starts_with("application/hl7-v2"), "ct: {ct}");
    assert!(
        body.contains("MSA|AA|MSGTEST"),
        "expected AA ACK, got: {body}"
    );

    let search_req = Request::builder()
        .method("GET")
        .uri(format!("/api/patients/search?q={family}&limit=10"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(search_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let hits = v["data"].as_array().expect("hits array");
    assert!(
        hits.iter().any(|p| p["name"]["family"] == family),
        "imported patient must be searchable: {v}"
    );

    // --- Non-ADT message → AE ACK ---
    let oru = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ORU^R01|MSGX|P|2.5\r\
PID|1||MRN-X^^^FAC^MR||Wrong^Type||19900115|F\r";
    let (status, _, body) = post_text(&app, "/api/hl7/v2/patient", oru.into()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("MSA|AE|MSGX"), "expected AE ACK, got: {body}");

    // --- Garbage → AR ACK (parse failure) ---
    let (status, _, body) = post_text(&app, "/api/hl7/v2/patient", "not a v2 message".into()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("MSA|AR|"), "expected AR ACK, got: {body}");

    // --- Parse endpoint echoes structured JSON ---
    let (status, _, body) = post_text(&app, "/api/hl7/v2/parse", msg).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("parse JSON");
    let segs = v["data"]["segments"].as_array().expect("segments array");
    let names: Vec<&str> = segs.iter().filter_map(|s| s["name"].as_str()).collect();
    assert_eq!(names, vec!["MSH", "EVN", "PID"]);
}

#[tokio::test]
async fn hl7v2_adt_a01_admits_patient_to_bed() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping hl7v2_adt_a01_admits_patient_to_bed");
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

    // Bootstrap: facility → ward → room → bed. Codes are suffixed with
    // a random UUID so the test is safely re-runnable against a stale DB.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let bed_code = format!("BED-A01-{suffix}");
    let ward_code = format!("WARD-A-HL7-{suffix}");
    let room_code = format!("ROOM-101-HL7-{suffix}");
    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC HL7 A01", "code": format!("FAC-HL7-A01-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"]
        .as_str()
        .expect("facility id")
        .to_string();
    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({
            "facility_id": facility_id,
            "name": "Ward A",
            "code": ward_code.clone(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "ward: {body}");
    let ward_id = body["data"]["id"].as_str().expect("ward id").to_string();
    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({
            "ward_id": ward_id,
            "name": "Room 101",
            "code": room_code.clone(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "room: {body}");
    let room_id = body["data"]["id"].as_str().expect("room id").to_string();
    let (status, body) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({
            "room_id": room_id,
            "name": "Bed A1",
            "code": bed_code,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "bed: {body}");

    let mrn = format!("MRN-A01-{}", uuid::Uuid::new_v4().simple());
    let family = format!("Hl7A01{}", uuid::Uuid::new_v4().simple());
    let msg = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A01|MSG-A01|P|2.5\r\
EVN|A01|20260523120000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F|||123 Elm^^Springfield^IL^62701^US||(555)555-0100\r\
PV1|1|I|{ward_code}^{room_code}^{bed_code}\r"
    );

    // --- Happy path: AA ACK, patient + admission both created ---
    let (status, ct, body) = post_text(&app, "/api/hl7/v2/admit", msg).await;
    assert_eq!(status, StatusCode::OK, "admit body: {body}");
    assert!(ct.starts_with("application/hl7-v2"));
    assert!(
        body.contains("MSA|AA|MSG-A01"),
        "expected AA ACK, got: {body}"
    );

    // Confirm ward occupancy includes our new admission.
    let occupancy_req = Request::builder()
        .method("GET")
        .uri(format!("/api/wards/{ward_id}/occupancy"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(occupancy_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let assignments = v["data"]["bed_assignments"]
        .as_array()
        .unwrap_or(&Vec::new())
        .clone();
    let occupied: usize = v["data"]["occupied"].as_u64().unwrap_or(0) as usize;
    assert!(
        occupied >= 1 || !assignments.is_empty(),
        "expected at least one occupied bed in ward: {v}"
    );

    // --- Bad bed code: AE ACK ---
    let bad_msg = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A01|MSG-A01-BAD|P|2.5\r\
EVN|A01|20260523120000\r\
PID|1||MRN-BAD^^^FAC^MR||BadFamily^A||19900101|F\r\
PV1|1|I|WARD-A^ROOM-101^NO-SUCH-BED\r"
        .to_string();
    let (status, _, body) = post_text(&app, "/api/hl7/v2/admit", bad_msg).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("MSA|AE|MSG-A01-BAD"),
        "expected AE ACK, got: {body}"
    );
    assert!(
        body.contains("not found"),
        "expected 'not found' diagnostic, got: {body}"
    );
}

#[tokio::test]
async fn hl7v2_adt_a02_transfer_and_a03_discharge_walk_an_admission() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_adt_a02_transfer_and_a03_discharge_walk_an_admission"
            );
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

    // Bootstrap a facility/ward/room with two beds (origin + destination).
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let bed_origin = format!("BED-ORIGIN-{suffix}");
    let bed_dest = format!("BED-DEST-{suffix}");

    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC HL7 A02/A03", "code": format!("FAC-A23-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"].as_str().unwrap().to_string();

    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({
            "facility_id": facility_id,
            "name": "Ward A23",
            "code": format!("WARD-A23-{suffix}")
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "ward: {body}");
    let ward_id = body["data"]["id"].as_str().unwrap().to_string();

    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({
            "ward_id": ward_id,
            "name": "Room A23",
            "code": format!("ROOM-A23-{suffix}")
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "room: {body}");
    let room_id = body["data"]["id"].as_str().unwrap().to_string();

    let (status, body) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Origin", "code": bed_origin }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "origin bed: {body}");
    let (status, body) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Dest", "code": bed_dest }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "dest bed: {body}");

    let mrn = format!("MRN-A23-{suffix}");
    let family = format!("Hl7A23{suffix}");

    // --- Step 1: ADT^A01 to set up the open admission ---
    let admit_msg = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A01|MSG-A01-{suffix}|P|2.5\r\
EVN|A01|20260523120000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r\
PV1|1|I|WARD-A23^ROOM-A23^{bed_origin}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/admit", admit_msg).await;
    assert_eq!(status, StatusCode::OK, "admit ack: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A01-{suffix}")));

    // --- Step 2: ADT^A02 transfer to the destination bed ---
    let transfer_msg = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523130000||ADT^A02|MSG-A02-{suffix}|P|2.5\r\
EVN|A02|20260523130000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r\
PV1|1|I|WARD-A23^ROOM-A23^{bed_dest}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/transfer", transfer_msg).await;
    assert_eq!(status, StatusCode::OK, "transfer ack: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A02-{suffix}")));

    // --- Step 3: ADT^A03 discharge ---
    let discharge_msg = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523180000||ADT^A03|MSG-A03-{suffix}|P|2.5\r\
EVN|A03|20260523180000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/discharge", discharge_msg).await;
    assert_eq!(status, StatusCode::OK, "discharge ack: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A03-{suffix}")));

    // --- Step 4: Re-discharge must fail — no open admission anymore ---
    let dup_discharge = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523181000||ADT^A03|MSG-A03-DUP-{suffix}|P|2.5\r\
EVN|A03|20260523181000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/discharge", dup_discharge).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "dup discharge: {body}");
    assert!(body.contains(&format!("MSA|AE|MSG-A03-DUP-{suffix}")));
    assert!(
        body.contains("no currently-open admission"),
        "expected 'no currently-open admission' diagnostic, got: {body}"
    );

    // --- Step 5: Unknown MRN → AE ---
    let bad_mrn_msg = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523182000||ADT^A03|MSG-A03-NOMRN-{suffix}|P|2.5\r\
EVN|A03|20260523182000\r\
PID|1||MRN-DOES-NOT-EXIST-{suffix}^^^FAC^MR||Nobody^A||19900101|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/discharge", bad_mrn_msg).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A03-NOMRN-{suffix}")));
    assert!(body.contains("no patient found"));
}

#[tokio::test]
async fn hl7v2_adt_a28_dedups_existing_mrn() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping hl7v2_adt_a28_dedups_existing_mrn");
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

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let mrn = format!("MRN-DEDUP-{suffix}");
    let family = format!("Dedup{suffix}");
    let msg_template = |msg_id: &str| {
        format!(
            "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A28|{msg_id}|P|2.5\r\
EVN|A28|20260523120000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F|||123 Elm^^Springfield^IL^62701^US||(555)555-0100\r"
        )
    };

    // --- First A28: creates a fresh patient. AA ACK without "matched". ---
    let (status, _, body) =
        post_text(&app, "/api/hl7/v2/patient", msg_template("MSG-DEDUP-1")).await;
    assert_eq!(status, StatusCode::OK, "first a28: {body}");
    assert!(
        body.contains("MSA|AA|MSG-DEDUP-1"),
        "expected AA, got: {body}"
    );
    assert!(
        !body.contains("matched existing patient"),
        "first a28 must NOT report a match: {body}"
    );

    // Capture the patient id so we can prove dedup picks the same row.
    let (status, payload) =
        get_json(&app, &format!("/api/patients/search?q={family}&limit=10")).await;
    assert_eq!(status, StatusCode::OK);
    let hits = payload["data"].as_array().expect("search hits");
    assert_eq!(hits.len(), 1, "expected exactly one match: {payload}");
    let first_id = hits[0]["id"].as_str().expect("patient id").to_string();

    // --- Second A28 with the SAME MRN: AA ACK that reports the match. ---
    let (status, _, body) =
        post_text(&app, "/api/hl7/v2/patient", msg_template("MSG-DEDUP-2")).await;
    assert_eq!(status, StatusCode::OK, "second a28: {body}");
    assert!(
        body.contains("MSA|AA|MSG-DEDUP-2"),
        "expected AA, got: {body}"
    );
    assert!(
        body.contains(&format!("matched existing patient {first_id}")),
        "second a28 must report the match by id: {body}"
    );

    // --- Confirm no duplicate row was created (search still returns 1 hit). ---
    let (status, payload) =
        get_json(&app, &format!("/api/patients/search?q={family}&limit=10")).await;
    assert_eq!(status, StatusCode::OK);
    let hits = payload["data"].as_array().expect("search hits");
    assert_eq!(
        hits.len(),
        1,
        "dedup must not create a second patient row: {payload}"
    );
    assert_eq!(hits[0]["id"].as_str(), Some(first_id.as_str()));
}

#[tokio::test]
async fn hl7v2_adt_a08_updates_existing_patient() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping hl7v2_adt_a08_updates_existing_patient");
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

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let mrn = format!("MRN-A08-{suffix}");
    let family_v1 = format!("A08Old{suffix}");
    let family_v2 = format!("A08New{suffix}");

    // --- Seed: create the patient via ADT^A28 ---
    let create_msg = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A28|MSG-A28-{suffix}|P|2.5\r\
EVN|A28|20260523120000\r\
PID|1||{mrn}^^^FAC^MR||{family_v1}^Jane^Marie||19900115|F|||123 Elm^^Springfield^IL^62701^US||(555)555-0100\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/patient", create_msg).await;
    assert_eq!(status, StatusCode::OK, "seed: {body}");

    // Grab the original id so we can prove A08 updates in place.
    let (status, payload) = get_json(
        &app,
        &format!("/api/patients/search?q={family_v1}&limit=10"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let original_id = payload["data"][0]["id"]
        .as_str()
        .expect("seeded patient id")
        .to_string();

    // --- A08 with new family name + new phone + new gender ---
    let update_msg = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523130000||ADT^A08|MSG-A08-{suffix}|P|2.5\r\
EVN|A08|20260523130000\r\
PID|1||{mrn}^^^FAC^MR||{family_v2}^Jane^Marie||19900115|M|||999 Maple^^Boston^MA^02108^US||(555)555-0999\r"
    );
    let (status, ct, body) = post_text(&app, "/api/hl7/v2/update", update_msg).await;
    assert_eq!(status, StatusCode::OK, "update body: {body}");
    assert!(ct.starts_with("application/hl7-v2"));
    assert!(
        body.contains(&format!("MSA|AA|MSG-A08-{suffix}")),
        "expected AA ACK, got: {body}"
    );
    assert!(
        body.contains(&format!("updated patient {original_id}")),
        "ACK MSA-3 should name the patient id: {body}"
    );

    // --- The updated row keeps the same id, has the new family ---
    let (status, payload) = get_json(&app, &format!("/api/patients/{original_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["data"]["id"].as_str(), Some(original_id.as_str()));
    assert_eq!(
        payload["data"]["name"]["family"].as_str(),
        Some(family_v2.as_str())
    );
    assert_eq!(payload["data"]["gender"].as_str(), Some("male"));
    // Phone overwritten to the new value.
    let phones: Vec<&str> = payload["data"]["telecom"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["system"].as_str() == Some("phone"))
        .filter_map(|c| c["value"].as_str())
        .collect();
    assert_eq!(phones, vec!["(555)555-0999"]);

    // --- The original family name is no longer searchable ---
    let (status, payload) = get_json(
        &app,
        &format!("/api/patients/search?q={family_v1}&limit=10"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // After Tantivy reindex the old token should not surface the patient.
    let hits = payload["data"].as_array().expect("hits");
    assert!(
        hits.iter()
            .all(|p| p["id"].as_str() != Some(original_id.as_str())),
        "old family name must not surface the updated patient: {payload}"
    );

    // --- A08 for an unknown MRN → AE ---
    let bad_msg = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523140000||ADT^A08|MSG-A08-BAD-{suffix}|P|2.5\r\
EVN|A08|20260523140000\r\
PID|1||MRN-NOPE-{suffix}^^^FAC^MR||Nobody^A||19900101|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/update", bad_msg).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A08-BAD-{suffix}")));
    assert!(
        body.contains("no patient found"),
        "expected 'no patient found' diagnostic, got: {body}"
    );
}

#[tokio::test]
async fn hl7v2_adt_a11_cancels_admit() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping hl7v2_adt_a11_cancels_admit");
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

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let bed_code = format!("BED-A11-{suffix}");
    let mrn = format!("MRN-A11-{suffix}");
    let family = format!("A11{suffix}");

    // Bootstrap facility/ward/room/bed.
    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC A11", "code": format!("FAC-A11-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({ "facility_id": facility_id, "name": "Ward A11", "code": format!("WARD-A11-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ward_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({ "ward_id": ward_id, "name": "Room A11", "code": format!("ROOM-A11-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let room_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, _) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Bed A11", "code": bed_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // --- Step 1: admit ---
    let admit = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A01|MSG-ADM-{suffix}|P|2.5\r\
EVN|A01|20260523120000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|WARD-A11^ROOM-A11^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/admit", admit).await;
    assert_eq!(status, StatusCode::OK, "admit: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-ADM-{suffix}")));

    // Ward occupancy should now show one occupied bed.
    let (status, payload) = get_json(&app, &format!("/api/wards/{ward_id}/occupancy")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(payload["data"]["occupied"].as_u64().unwrap_or(0) >= 1);

    // --- Step 2: A11 cancel-admit ---
    let cancel = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523121000||ADT^A11|MSG-A11-{suffix}|P|2.5\r\
EVN|A11|20260523121000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-admit", cancel).await;
    assert_eq!(status, StatusCode::OK, "cancel-admit: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A11-{suffix}")));

    // After cancel: ward occupancy drops; the bed flips to Cleaning (the
    // BedStatus::Occupied → Cleaning transition is what `cancel_admission`
    // applies — same as discharge).
    let (status, payload) = get_json(&app, &format!("/api/wards/{ward_id}/occupancy")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        payload["data"]["occupied"].as_u64().unwrap_or(99),
        0,
        "expected no occupied beds after cancel: {payload}"
    );

    // --- Step 3: re-cancelling fails — no open admission now ---
    let cancel_again = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523122000||ADT^A11|MSG-A11-DUP-{suffix}|P|2.5\r\
EVN|A11|20260523122000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-admit", cancel_again).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A11-DUP-{suffix}")));
    assert!(body.contains("no currently-open admission"));
}

#[tokio::test]
async fn hl7v2_adt_a13_cancels_discharge() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping hl7v2_adt_a13_cancels_discharge");
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

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let bed_code = format!("BED-A13-{suffix}");
    let mrn = format!("MRN-A13-{suffix}");
    let family = format!("A13{suffix}");

    // Bootstrap facility/ward/room/bed.
    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC A13", "code": format!("FAC-A13-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({ "facility_id": facility_id, "name": "Ward A13", "code": format!("WARD-A13-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ward_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({ "ward_id": ward_id, "name": "Room A13", "code": format!("ROOM-A13-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let room_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, _) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Bed A13", "code": bed_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // --- Step 1: admit then discharge ---
    let admit = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A01|MSG-ADM-{suffix}|P|2.5\r\
EVN|A01|20260523120000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|WARD-A13^ROOM-A13^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/admit", admit).await;
    assert_eq!(status, StatusCode::OK, "admit: {body}");

    let discharge = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523180000||ADT^A03|MSG-DSC-{suffix}|P|2.5\r\
EVN|A03|20260523180000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/discharge", discharge).await;
    assert_eq!(status, StatusCode::OK, "discharge: {body}");

    // --- Step 2: A13 cancel-discharge → patient reinstated, bed Occupied ---
    let cancel = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523181000||ADT^A13|MSG-A13-{suffix}|P|2.5\r\
EVN|A13|20260523181000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-discharge", cancel).await;
    assert_eq!(status, StatusCode::OK, "cancel-discharge: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A13-{suffix}")));

    // Ward occupancy should show one occupied bed again.
    let (status, payload) = get_json(&app, &format!("/api/wards/{ward_id}/occupancy")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        payload["data"]["occupied"].as_u64().unwrap_or(0) >= 1,
        "expected reinstated admission: {payload}"
    );

    // --- Step 3: re-cancelling fails — no discharge to cancel ---
    let cancel_again = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523182000||ADT^A13|MSG-A13-DUP-{suffix}|P|2.5\r\
EVN|A13|20260523182000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-discharge", cancel_again).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A13-DUP-{suffix}")));
    assert!(
        body.contains("no discharge to cancel"),
        "expected 'no discharge to cancel' diagnostic, got: {body}"
    );

    // --- Step 4: unknown MRN → AE ---
    let bad = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523183000||ADT^A13|MSG-A13-NOMRN-{suffix}|P|2.5\r\
EVN|A13|20260523183000\r\
PID|1||MRN-NOPE-{suffix}^^^FAC^MR||Nobody^A||19900101|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-discharge", bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A13-NOMRN-{suffix}")));
    assert!(body.contains("no patient found"));
}

#[tokio::test]
async fn hl7v2_batch_dispatches_each_message_independently() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_batch_dispatches_each_message_independently"
            );
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

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let mrn_ok_1 = format!("MRN-BATCH-OK1-{suffix}");
    let mrn_ok_2 = format!("MRN-BATCH-OK2-{suffix}");
    let family_1 = format!("BatchOne{suffix}");
    let family_2 = format!("BatchTwo{suffix}");

    // A batch with three messages:
    //   #1: well-formed ADT^A28 → AA, creates a patient
    //   #2: well-formed ADT^A28 → AA, creates another patient
    //   #3: ADT^A28 with an empty family name → AE (rejected by mapping)
    let batch = format!(
        "BHS|^~\\&|EMR|FAC|PAS|FAC|20260523120000||BATCH-{suffix}|P|2.5\r\
MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A28|MSG-OK1-{suffix}|P|2.5\r\
EVN|A28|20260523120000\r\
PID|1||{mrn_ok_1}^^^FAC^MR||{family_1}^Jane||19900101|F\r\
MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120001||ADT^A28|MSG-OK2-{suffix}|P|2.5\r\
EVN|A28|20260523120001\r\
PID|1||{mrn_ok_2}^^^FAC^MR||{family_2}^John||19850515|M\r\
MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120002||ADT^A28|MSG-BAD-{suffix}|P|2.5\r\
EVN|A28|20260523120002\r\
PID|1||MRN-BAD-{suffix}^^^FAC^MR||||19800101|F\r\
BTS|3\r"
    );

    let (status, ct, body) = post_text(&app, "/api/hl7/v2/batch", batch).await;
    assert_eq!(status, StatusCode::OK, "batch body: {body}");
    assert!(ct.starts_with("application/hl7-v2"), "ct: {ct}");

    // Envelope: starts with BHS, ends with BTS|3.
    assert!(
        body.starts_with("BHS|^~\\&|PAS|FAC|EMR|FAC|"),
        "expected PAS→EMR BHS envelope, got: {body}"
    );
    assert!(
        body.contains(&format!("|ACK-BATCH-{suffix}|P|2.5\r")),
        "batch control id should echo as ACK-BATCH-...: {body}"
    );
    assert!(
        body.contains("\rBTS|3\r"),
        "BTS must report 3 contained ACKs: {body}"
    );

    // Per-message ACKs: two AA, one AE.
    assert!(
        body.contains(&format!("MSA|AA|MSG-OK1-{suffix}")),
        "missing AA for first message: {body}"
    );
    assert!(
        body.contains(&format!("MSA|AA|MSG-OK2-{suffix}")),
        "missing AA for second message: {body}"
    );
    assert!(
        body.contains(&format!("MSA|AE|MSG-BAD-{suffix}")),
        "missing AE for bad message: {body}"
    );

    // Search must surface the two created patients but not the bad one.
    let (status, payload) =
        get_json(&app, &format!("/api/patients/search?q={family_1}&limit=10")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        payload["data"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "first batch patient should be searchable: {payload}"
    );
    let (status, payload) =
        get_json(&app, &format!("/api/patients/search?q={family_2}&limit=10")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        payload["data"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "second batch patient should be searchable: {payload}"
    );

    // Empty input → AR ACK envelope at the batch level (not wrapped).
    let (status, _, body) = post_text(&app, "/api/hl7/v2/batch", "".into()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("MSA|AR|"),
        "expected AR ACK for empty batch, got: {body}"
    );
}

// ----- SIU (v0.16) ---------------------------------------------------------

#[tokio::test]
async fn hl7v2_siu_s12_books_appointment_and_s15_cancels_it() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_siu_s12_books_appointment_and_s15_cancels_it"
            );
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

    // Random MRN so reruns don't collide.
    let mrn = format!("MRN-SIU-{}", uuid::Uuid::new_v4().simple());
    let family = format!("SiuTest{}", uuid::Uuid::new_v4().simple());

    // S12: PID arrives unknown to PAS; dedup-or-create kicks in and we
    // create the patient on the fly, then the appointment.
    let s12 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S12|MSG-S12-TEST|P|2.5\r\
SCH|PLACER-001||||||routine follow-up||30|min|20270605143000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, ct, body) = post_text(&app, "/api/hl7/v2/schedule-book", s12).await;
    assert_eq!(status, StatusCode::OK, "S12 must AA: {body}");
    assert!(ct.starts_with("application/hl7-v2"));
    assert!(
        body.contains("MSA|AA|MSG-S12-TEST"),
        "expected AA ACK with MSG-S12-TEST control id, got: {body}"
    );
    // Diagnostic carries the assigned filler id (PAS appointment UUID).
    let filler_uuid = body
        .split("filler=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("diagnostic should contain filler=<uuid>")
        .to_string();
    assert_eq!(filler_uuid.len(), 36, "filler must be a 36-char UUID");

    // Overlap protection: a second S12 for the same patient + same time
    // window must be rejected 409 + AE.
    let s12_overlap = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S12|MSG-S12-DUP|P|2.5\r\
SCH|PLACER-002||||||conflict||30|min|20270605143000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-book", s12_overlap).await;
    assert_eq!(status, StatusCode::CONFLICT, "overlap must 409: {body}");
    assert!(
        body.contains("MSA|AE|MSG-S12-DUP"),
        "expected AE ACK for overlap, got: {body}"
    );

    // S15 with the filler id from the AA above: cancels the appointment.
    let s15 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S15|MSG-S15-TEST|P|2.5\r\
SCH|PLACER-001|{filler_uuid}|||||patient request\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-cancel", s15).await;
    assert_eq!(status, StatusCode::OK, "S15 must AA: {body}");
    assert!(
        body.contains("MSA|AA|MSG-S15-TEST"),
        "expected AA ACK for cancel, got: {body}"
    );

    // Re-cancel must 409 + AE — appointment is now terminal.
    let s15_again = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S15|MSG-S15-DUP|P|2.5\r\
SCH|PLACER-001|{filler_uuid}\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-cancel", s15_again).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "double-cancel must 409: {body}"
    );
    assert!(
        body.contains("MSA|AE|MSG-S15-DUP"),
        "expected AE ACK for already-cancelled, got: {body}"
    );

    // Unknown filler uuid → 404 + AE.
    let bogus = uuid::Uuid::new_v4();
    let s15_bogus = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S15|MSG-S15-404|P|2.5\r\
SCH|PLACER-X|{bogus}\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-cancel", s15_bogus).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unknown filler must 404: {body}"
    );
    assert!(
        body.contains("MSA|AE|MSG-S15-404"),
        "expected AE ACK for unknown filler, got: {body}"
    );

    // Non-UUID filler → 400 + AE.
    let s15_bad = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S15|MSG-S15-BAD|P|2.5\r\
SCH|PLACER-X|not-a-uuid\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-cancel", s15_bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("MSA|AE|MSG-S15-BAD"));

    // S15 with no SCH-2 → 400 + AE.
    let s15_no_filler = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S15|MSG-S15-NO|P|2.5\r\
SCH|PLACER-X\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-cancel", s15_no_filler).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("MSA|AE|MSG-S15-NO"),
        "expected AE ACK for missing filler, got: {body}"
    );

    // Wrong message type at the book endpoint → 400 + AE.
    let s15_at_book = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S15|MSG-WRONG|P|2.5\r\
SCH|PLACER-X|{filler_uuid}\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-book", s15_at_book).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("MSA|AE|MSG-WRONG"),
        "expected AE ACK for SIU^S15 at /schedule-book, got: {body}"
    );
}

#[tokio::test]
async fn hl7v2_siu_s13_reschedules_and_s14_modifies() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping hl7v2_siu_s13_reschedules_and_s14_modifies");
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

    let mrn = format!("MRN-RM-{}", uuid::Uuid::new_v4().simple());
    let family = format!("RmTest{}", uuid::Uuid::new_v4().simple());

    // S12: book a 30-minute appointment at 14:30 on 2027-06-05.
    let s12 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S12|MSG-S12-RM|P|2.5\r\
SCH|PLACER-RM||||||initial visit||30|min|20270605143000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-book", s12).await;
    assert_eq!(status, StatusCode::OK, "S12 must AA: {body}");
    let filler_uuid = body
        .split("filler=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("filler= in MSA-3")
        .to_string();

    // Book a second appointment (different patient, same family-prefix MRN
    // pattern) at 16:00 so we can prove S13 overlap-protection works:
    // rescheduling the first appointment to 16:00 must conflict only with
    // *another patient's* appointment via the patient itself — not via
    // its own existing row. We test overlap with the *same* patient by
    // booking a second appt for the same patient at 18:00, then trying
    // to reschedule the first into 18:00.
    let s12b = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S12|MSG-S12-RM2|P|2.5\r\
SCH|PLACER-RM2||||||second visit||30|min|20270605180000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-book", s12b).await;
    assert_eq!(status, StatusCode::OK, "second S12 must AA: {body}");

    // S13: reschedule first appointment to 17:00 (no overlap with the
    // 18:00 row). Must AA.
    let s13 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S13|MSG-S13-RM|P|2.5\r\
SCH|PLACER-RM|{filler_uuid}|||||rescheduled||45|min|20270605170000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-reschedule", s13).await;
    assert_eq!(status, StatusCode::OK, "S13 must AA: {body}");
    assert!(body.contains("MSA|AA|MSG-S13-RM"));

    // S13 conflict: try to reschedule into 18:00 — collides with the
    // second appointment. Must 409 + AE.
    let s13_conflict = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S13|MSG-S13-CONF|P|2.5\r\
SCH|PLACER-RM|{filler_uuid}|||||conflict||30|min|20270605181500\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-reschedule", s13_conflict).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "S13 conflict must 409: {body}"
    );
    assert!(body.contains("MSA|AE|MSG-S13-CONF"));

    // S14: modify the appointment's reason (no time changes).
    let s14 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S14|MSG-S14-RM|P|2.5\r\
SCH|PLACER-RM|{filler_uuid}|||||revised reason text\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-modify", s14).await;
    assert_eq!(status, StatusCode::OK, "S14 must AA: {body}");
    assert!(body.contains("MSA|AA|MSG-S14-RM"));

    // S13 against an unknown filler → 404.
    let bogus = uuid::Uuid::new_v4();
    let s13_404 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S13|MSG-S13-404|P|2.5\r\
SCH|PLACER-RM|{bogus}|||||x||30|min|20270605200000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-reschedule", s13_404).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "S13 404: {body}");
    assert!(body.contains("MSA|AE|MSG-S13-404"));

    // S14 missing SCH-2 → 400.
    let s14_bad = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S14|MSG-S14-BAD|P|2.5\r\
SCH|PLACER-RM||||||reason without filler\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-modify", s14_bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "S14 bad: {body}");
    assert!(body.contains("MSA|AE|MSG-S14-BAD"));

    // S13 missing SCH-11 → 400.
    let s13_no_time = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S13|MSG-S13-NT|P|2.5\r\
SCH|PLACER-RM|{filler_uuid}|||||no-time||30|min|\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-reschedule", s13_no_time).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("MSA|AE|MSG-S13-NT"));

    // Wrong message type at /schedule-modify → 400.
    let s13_at_modify = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S13|MSG-WRONG-MOD|P|2.5\r\
SCH|PLACER-RM|{filler_uuid}|||||x||30|min|20270605210000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/schedule-modify", s13_at_modify).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("MSA|AE|MSG-WRONG-MOD"));

    // Cancel + then reschedule → 409 because terminal.
    let s15_cancel = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S15|MSG-S15-RM|P|2.5\r\
SCH|PLACER-RM|{filler_uuid}\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, _) = post_text(&app, "/api/hl7/v2/schedule-cancel", s15_cancel).await;
    assert_eq!(status, StatusCode::OK);

    let s13_after_cancel = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S13|MSG-S13-TERM|P|2.5\r\
SCH|PLACER-RM|{filler_uuid}|||||late||30|min|20270605220000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );
    let (status, _, body) =
        post_text(&app, "/api/hl7/v2/schedule-reschedule", s13_after_cancel).await;
    assert_eq!(status, StatusCode::CONFLICT, "S13 on terminal: {body}");
    assert!(body.contains("MSA|AE|MSG-S13-TERM"));
}

#[tokio::test]
async fn hl7v2_adt_a40_merges_source_into_survivor() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping hl7v2_adt_a40_merges_source_into_survivor");
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

    // Bootstrap: create two patients via A28 (the simplest HL7 v2
    // patient-creation path). Use distinct MRNs and family names so we
    // can identify them later.
    let survivor_mrn = format!("MRN-SUR-{}", uuid::Uuid::new_v4().simple());
    let source_mrn = format!("MRN-SRC-{}", uuid::Uuid::new_v4().simple());
    let survivor_family = format!("Survivor{}", uuid::Uuid::new_v4().simple());
    let source_family = format!("Source{}", uuid::Uuid::new_v4().simple());

    let a28_survivor = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A28|MSG-SURV|P|2.5\r\
EVN|A28|20260601090000\r\
PID|1||{survivor_mrn}^^^FAC^MR||{survivor_family}^Jane||19900115|F\r"
    );
    let (status, _, _) = post_text(&app, "/api/hl7/v2/patient", a28_survivor).await;
    assert_eq!(status, StatusCode::OK);
    let a28_source = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A28|MSG-SRC|P|2.5\r\
EVN|A28|20260601090000\r\
PID|1||{source_mrn}^^^FAC^MR||{source_family}^Jane||19900115|F\r"
    );
    let (status, _, _) = post_text(&app, "/api/hl7/v2/patient", a28_source).await;
    assert_eq!(status, StatusCode::OK);

    // A40: merge source into survivor.
    let a40 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A40|MSG-A40-OK|P|2.5\r\
EVN|A40|20260601090000\r\
PID|1||{survivor_mrn}^^^FAC^MR||{survivor_family}^Jane||19900115|F\r\
MRG|{source_mrn}^^^FAC^MR\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/merge", a40).await;
    assert_eq!(status, StatusCode::OK, "A40 must AA: {body}");
    assert!(body.contains("MSA|AA|MSG-A40-OK"));
    assert!(
        body.contains("merged source=") && body.contains("into target="),
        "diagnostic must report the linkage, got: {body}"
    );

    // Merge correctness is proven by the re-merge → 409 path below
    // (the source row must now carry `replaced_by`). We deliberately
    // don't assert Tantivy state here: the search reader uses a
    // `OnCommitWithDelay` reload policy and a tight test sequence can
    // race that reload. The Tantivy-drop side-effect is exercised by
    // the v0.11 patient-merge integration test against the REST
    // endpoint, which uses the same code path.

    // Re-merge the same source → 409 + AE (already a tombstone).
    let a40_again = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A40|MSG-A40-DUP|P|2.5\r\
EVN|A40|20260601090000\r\
PID|1||{survivor_mrn}^^^FAC^MR||{survivor_family}^Jane||19900115|F\r\
MRG|{source_mrn}^^^FAC^MR\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/merge", a40_again).await;
    assert_eq!(status, StatusCode::CONFLICT, "re-merge must 409: {body}");
    assert!(body.contains("MSA|AE|MSG-A40-DUP"));

    // Unknown source MRN → 404 + AE.
    let bogus_mrn = format!("MRN-UNKNOWN-{}", uuid::Uuid::new_v4().simple());
    let a40_404 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A40|MSG-A40-404|P|2.5\r\
EVN|A40|20260601090000\r\
PID|1||{survivor_mrn}^^^FAC^MR||{survivor_family}^Jane||19900115|F\r\
MRG|{bogus_mrn}^^^FAC^MR\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/merge", a40_404).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown source: {body}");
    assert!(body.contains("MSA|AE|MSG-A40-404"));

    // Missing MRG segment → 400 + AE.
    let a40_no_mrg = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A40|MSG-A40-NO-MRG|P|2.5\r\
EVN|A40|20260601090000\r\
PID|1||{survivor_mrn}^^^FAC^MR||{survivor_family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/merge", a40_no_mrg).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("MSA|AE|MSG-A40-NO-MRG"));

    // Self-merge (MRG-1 same MRN as PID-3) → 409 + AE.
    let a40_self = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A40|MSG-A40-SELF|P|2.5\r\
EVN|A40|20260601090000\r\
PID|1||{survivor_mrn}^^^FAC^MR||{survivor_family}^Jane||19900115|F\r\
MRG|{survivor_mrn}^^^FAC^MR\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/merge", a40_self).await;
    assert_eq!(status, StatusCode::CONFLICT, "self-merge: {body}");
    assert!(body.contains("MSA|AE|MSG-A40-SELF"));

    // Wrong message type at /merge → 400 + AE.
    let a28_at_merge = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A28|MSG-A40-WRONG|P|2.5\r\
EVN|A28|20260601090000\r\
PID|1||{survivor_mrn}^^^FAC^MR||{survivor_family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/merge", a28_at_merge).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("MSA|AE|MSG-A40-WRONG"));
}

#[tokio::test]
async fn hl7v2_dft_p03_posts_charge_and_creates_account() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_dft_p03_posts_charge_and_creates_account"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    let mrn = format!("MRN-DFT-{}", uuid::Uuid::new_v4().simple());
    let family = format!("DftTest{}", uuid::Uuid::new_v4().simple());

    // Happy path: DFT^P03 for a previously-unknown patient. The handler
    // dedup-or-creates the patient, then auto-creates an open account
    // in the FT1-11.2 currency, then posts the charge.
    let dft = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-DFT-OK|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
FT1|1|EXT-1||20260601100000||CG|97110|Therapeutic exercise|||125.50^USD\r"
    );
    let (status, ct, body) = post_text(&app, "/api/hl7/v2/dft", dft).await;
    assert_eq!(status, StatusCode::OK, "DFT must AA: {body}");
    assert!(ct.starts_with("application/hl7-v2"));
    assert!(body.contains("MSA|AA|MSG-DFT-OK"));
    let charge_uuid = body
        .split("charge=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("MSA-3 should contain charge=<uuid>")
        .to_string();
    let account_uuid = body
        .split("account=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("MSA-3 should contain account=<uuid>")
        .to_string();
    assert_eq!(charge_uuid.len(), 36);
    assert_eq!(account_uuid.len(), 36);

    // Direct DB readback: the charge row exists and points at the new account.
    use patient_administration_system::db::repositories::billing::BillingRepository;
    let charge_id = uuid::Uuid::parse_str(&charge_uuid).unwrap();
    let charge = BillingRepository::find_charge_by_id(&state.db, charge_id)
        .await
        .expect("find_charge")
        .expect("charge must exist");
    assert_eq!(charge.code, "97110");
    assert_eq!(charge.description, "Therapeutic exercise");
    assert_eq!(
        charge.amount.amount,
        rust_decimal::Decimal::from_str_exact("125.50").unwrap()
    );
    assert_eq!(charge.amount.currency.0, "USD");

    // A second DFT for the same patient must reuse the open account
    // (not create a duplicate). MSA-3 carries the same `account=`.
    let dft_2 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-DFT-2|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
FT1|1|EXT-2||20260602100000||CG|99213|Office visit|||80.00^USD\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/dft", dft_2).await;
    assert_eq!(status, StatusCode::OK, "second DFT must AA: {body}");
    assert!(body.contains("MSA|AA|MSG-DFT-2"));
    assert!(
        body.contains(&format!("account={account_uuid}")),
        "open account must be reused, got: {body}"
    );

    // Unsupported transaction type (PY) → 400 + AE.
    let dft_py = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-DFT-PY|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
FT1|1|||||PY|97110|Visit|||50^USD\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/dft", dft_py).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "PY: {body}");
    assert!(body.contains("MSA|AE|MSG-DFT-PY"));

    // Missing FT1 → 400 + AE.
    let dft_no_ft1 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-DFT-NOFT1|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/dft", dft_no_ft1).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("MSA|AE|MSG-DFT-NOFT1"));

    // Bad currency → 400 + AE.
    let dft_bad_curr = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-DFT-CURR|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
FT1|1|||||CG|97110|Visit|||50^bogus\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/dft", dft_bad_curr).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("MSA|AE|MSG-DFT-CURR"));

    // Wrong message type at /dft → 400 + AE.
    let adt_at_dft = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A28|MSG-WRONG-DFT|P|2.5\r\
EVN|A28|20260601090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/dft", adt_at_dft).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("MSA|AE|MSG-WRONG-DFT"));
}

#[tokio::test]
async fn hl7v2_dft_p03_posts_multiple_ft1_in_one_transaction() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_dft_p03_posts_multiple_ft1_in_one_transaction"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    let mrn = format!("MRN-DFT-M-{}", uuid::Uuid::new_v4().simple());
    let family = format!("DftMulti{}", uuid::Uuid::new_v4().simple());

    // Happy path: a single DFT message carrying three FT1 segments.
    // All three charges must persist; MSA-3 reports the count + account.
    let dft = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-DFT-MULTI|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
FT1|1|||20260601100000||CG|97110|Therapeutic exercise|||125.50^USD\r\
FT1|2|||20260601100500||CG|99213|Office visit|||80.00^USD\r\
FT1|3|||20260601100800||CG|J0696|Antibiotic injection|||15.25^USD\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/dft", dft).await;
    assert_eq!(status, StatusCode::OK, "multi-FT1 DFT must AA: {body}");
    assert!(body.contains("MSA|AA|MSG-DFT-MULTI"));
    assert!(
        body.contains("charges_posted=3"),
        "MSA-3 must report count for multi-FT1, got: {body}"
    );
    let account_uuid = body
        .split("account=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("MSA-3 should contain account=<uuid>")
        .to_string();

    // All three charges landed on the same account.
    use patient_administration_system::db::repositories::billing::BillingRepository;
    let account_id = uuid::Uuid::parse_str(&account_uuid).unwrap();
    let charges = BillingRepository::list_charges_for_account(&state.db, account_id)
        .await
        .expect("list charges");
    assert_eq!(charges.len(), 3, "all three charges must persist");
    let mut codes: Vec<String> = charges.iter().map(|c| c.code.clone()).collect();
    codes.sort();
    assert_eq!(codes, vec!["97110", "99213", "J0696"]);
    for c in &charges {
        assert_eq!(c.amount.currency.0, "USD");
    }

    // Mixed currencies in one DFT → 400 + AE.
    let dft_mixed = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-DFT-MIX|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
FT1|1|||||CG|97110|A|||50^USD\r\
FT1|2|||||CG|99213|B|||50^EUR\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/dft", dft_mixed).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "mixed currencies: {body}");
    assert!(body.contains("MSA|AE|MSG-DFT-MIX"));

    // Atomicity: if FT1[2] is malformed, NO charges from the message
    // should land. We post the message, expect AE, then assert the
    // account's charge count is unchanged.
    let charges_before = BillingRepository::list_charges_for_account(&state.db, account_id)
        .await
        .expect("list before")
        .len();
    let dft_partial = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-DFT-PART|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
FT1|1|||||CG|97110|Good|||10^USD\r\
FT1|2|||||CG|99213|Bad|||not-a-number^USD\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/dft", dft_partial).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "partial: {body}");
    assert!(body.contains("MSA|AE|MSG-DFT-PART"));
    let charges_after = BillingRepository::list_charges_for_account(&state.db, account_id)
        .await
        .expect("list after")
        .len();
    assert_eq!(
        charges_after, charges_before,
        "first FT1 must not have landed because second failed; before={charges_before}, after={charges_after}"
    );
}

#[tokio::test]
async fn hl7v2_mfn_m02_walks_add_update_delete() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping hl7v2_mfn_m02_walks_add_update_delete");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    let staff_id = format!("STF-MFN-{}", uuid::Uuid::new_v4().simple());
    let family_v1 = format!("Curie{}", uuid::Uuid::new_v4().simple());
    let family_v2 = format!("Sklodowska{}", uuid::Uuid::new_v4().simple());

    // MAD: create a new practitioner.
    let mfn_add = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-ADD|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MAD||20260601090000|{staff_id}\r\
STF|{staff_id}||{family_v1}^Marie||F|18671107|A\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-staff", mfn_add).await;
    assert_eq!(status, StatusCode::OK, "MAD must AA: {body}");
    assert!(body.contains("MSA|AA|MSG-MFN-ADD"));
    let pract_uuid = body
        .split("practitioner=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("AA must report practitioner=<uuid>")
        .to_string();

    // Duplicate MAD on the same staff id → 409 + AE.
    let mfn_add_again = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-DUP|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MAD||20260601090000|{staff_id}\r\
STF|{staff_id}||{family_v1}^Marie||F|18671107|A\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-staff", mfn_add_again).await;
    assert_eq!(status, StatusCode::CONFLICT, "dup MAD: {body}");
    assert!(body.contains("MSA|AE|MSG-MFN-DUP"));

    // MUP: rename — verify family changes from v1 → v2.
    let mfn_upd = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-UPD|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MUP||20260601090000|{staff_id}\r\
STF|{staff_id}||{family_v2}^Marie||F|18671107|A\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-staff", mfn_upd).await;
    assert_eq!(status, StatusCode::OK, "MUP must AA: {body}");
    assert!(body.contains("MSA|AA|MSG-MFN-UPD"));

    // Direct DB read-back via the new repo.
    use patient_administration_system::db::repositories::practitioner::PractitionerRepository;
    let id = uuid::Uuid::parse_str(&pract_uuid).unwrap();
    let p = PractitionerRepository::find_by_id(&state.db, id)
        .await
        .expect("find")
        .expect("must exist");
    assert_eq!(p.name.family, family_v2);
    assert!(p.active);

    // MDL: soft-delete via active=false.
    let mfn_del = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-DEL|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MDL||20260601090000|{staff_id}\r\
STF|{staff_id}||{family_v2}^Marie||F|18671107|I\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-staff", mfn_del).await;
    assert_eq!(status, StatusCode::OK, "MDL must AA: {body}");
    let p = PractitionerRepository::find_by_id(&state.db, id)
        .await
        .expect("find")
        .expect("row must still exist after MDL");
    assert!(!p.active, "MDL must soft-delete via active=false");

    // MUP / MDL on unknown staff id → 404.
    let bogus = format!("BOGUS-{}", uuid::Uuid::new_v4().simple());
    let mfn_404 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-404|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MUP||20260601090000|{bogus}\r\
STF|{bogus}||Unknown^Practitioner||M||A\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-staff", mfn_404).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "MUP on unknown: {body}");
    assert!(body.contains("MSA|AE|MSG-MFN-404"));

    // Atomicity: a 2-item MFN with a duplicate MAD in the second
    // slot must roll back the first item entirely.
    let dup_staff = staff_id.clone();
    let new_staff = format!("STF-PARTIAL-{}", uuid::Uuid::new_v4().simple());
    let mfn_partial = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-PART|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MAD||20260601090000|{new_staff}\r\
STF|{new_staff}||PartialAdd^Should||M||A\r\
MFE|MAD||20260601090000|{dup_staff}\r\
STF|{dup_staff}||PartialDup^Should||M||A\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-staff", mfn_partial).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.contains("MSA|AE|MSG-MFN-PART"));
    // First item must not have landed.
    let leftover = PractitionerRepository::find_by_identifier_value(&state.db, &new_staff)
        .await
        .expect("lookup");
    assert!(
        leftover.is_none(),
        "partial MFN must not leave the first MAD persisted"
    );
}

#[tokio::test]
async fn hl7v2_mfn_m05_walks_add_update_delete_on_beds() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_mfn_m05_walks_add_update_delete_on_beds"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    // Bootstrap a facility → ward → room via REST so we have a valid
    // parent for the bed MFN messages. Random suffixes keep this
    // test re-runnable without DB cleanup between runs.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let facility_code = format!("FAC-M05-{suffix}");
    let ward_code = format!("WARD-M05-{suffix}");
    let room_code = format!("ROOM-M05-{suffix}");

    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "M05 Fac", "code": facility_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({
            "facility_id": facility_id,
            "name": "M05 Ward",
            "code": ward_code.clone(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "ward: {body}");
    let ward_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({
            "ward_id": ward_id,
            "name": "M05 Room",
            "code": room_code.clone(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "room: {body}");

    let bed_code = format!("BED-M05-{suffix}");

    // MAD: create a new bed under the bootstrapped room.
    let mfn_add = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-M05-ADD|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MAD||20260601090000|{bed_code}\r\
LOC|{room_code}^^{bed_code}|Bed One\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-location", mfn_add).await;
    assert_eq!(status, StatusCode::OK, "MAD must AA: {body}");
    assert!(body.contains("MSA|AA|MSG-M05-ADD"));
    let bed_uuid = body
        .split("bed=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .expect("AA must report bed=<uuid>")
        .to_string();

    // Duplicate MAD on same bed code → 409 + AE.
    let mfn_dup = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-M05-DUP|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MAD||20260601090000|{bed_code}\r\
LOC|{room_code}^^{bed_code}|Bed One\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-location", mfn_dup).await;
    assert_eq!(status, StatusCode::CONFLICT, "dup MAD: {body}");
    assert!(body.contains("MSA|AE|MSG-M05-DUP"));

    // MUP: rename the bed.
    let mfn_upd = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-M05-UPD|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MUP||20260601090000|{bed_code}\r\
LOC|{room_code}^^{bed_code}|Bed One (Renamed)\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-location", mfn_upd).await;
    assert_eq!(status, StatusCode::OK, "MUP must AA: {body}");
    use patient_administration_system::db::repositories::bed::BedRepository;
    use patient_administration_system::models::facility::BedStatus;
    let id = uuid::Uuid::parse_str(&bed_uuid).unwrap();
    let b = BedRepository::find_by_id(&state.db, id)
        .await
        .expect("find")
        .expect("must exist");
    assert_eq!(b.name, "Bed One (Renamed)");
    assert_eq!(b.status, BedStatus::Available);

    // MDL: soft-delete via OutOfService.
    let mfn_del = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-M05-DEL|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MDL||20260601090000|{bed_code}\r\
LOC|^^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-location", mfn_del).await;
    assert_eq!(status, StatusCode::OK, "MDL must AA: {body}");
    let b = BedRepository::find_by_id(&state.db, id)
        .await
        .expect("find")
        .expect("row must still exist after MDL");
    assert_eq!(
        b.status,
        BedStatus::OutOfService,
        "MDL must flip bed to OutOfService"
    );

    // MUP on unknown bed code → 404 + AE.
    let bogus = format!("BED-MISSING-{suffix}");
    let mfn_404 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-M05-404|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MUP||20260601090000|{bogus}\r\
LOC|{room_code}^^{bogus}|Nope\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-location", mfn_404).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "MUP on unknown: {body}");
    assert!(body.contains("MSA|AE|MSG-M05-404"));

    // MAD with an unknown room → 404 + AE (the MAD pre-check resolves
    // the parent room before the transaction opens).
    let bed_orphan = format!("BED-ORPH-{suffix}");
    let mfn_orph = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-M05-ORPH|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MAD||20260601090000|{bed_orphan}\r\
LOC|ROOM-DOES-NOT-EXIST^^{bed_orphan}|Bed Orphan\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-location", mfn_orph).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "orphan room: {body}");
    assert!(body.contains("MSA|AE|MSG-M05-ORPH"));

    // Atomicity: 2-item MFN where the second item duplicates an
    // existing bed code must roll back the first item.
    let new_bed = format!("BED-PARTIAL-{suffix}");
    let mfn_partial = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-M05-PART|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MAD||20260601090000|{new_bed}\r\
LOC|{room_code}^^{new_bed}|Partial Add\r\
MFE|MAD||20260601090000|{bed_code}\r\
LOC|{room_code}^^{bed_code}|Will Fail\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/mfn-location", mfn_partial).await;
    assert_eq!(status, StatusCode::CONFLICT, "partial: {body}");
    let leftover = BedRepository::find_by_code(&state.db, &new_bed)
        .await
        .expect("lookup");
    assert!(
        leftover.is_none(),
        "partial MFN must not leave the first MAD persisted"
    );
}

#[tokio::test]
async fn hl7v2_adt_a04_registers_outpatient_and_emergency() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_adt_a04_registers_outpatient_and_emergency"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    use patient_administration_system::db::repositories::encounter::EncounterRepository;
    use patient_administration_system::models::encounter::{EncounterClass, EncounterStatus};

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let mrn_o = format!("MRN-A04-O-{suffix}");
    let mrn_e = format!("MRN-A04-E-{suffix}");

    // Outpatient: PV1-2 = "O".
    let a04_o = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A04|MSG-A04-O|P|2.5\r\
PID|||{mrn_o}||Smith^John||19700115|M\r\
PV1||O|\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/register", a04_o).await;
    assert_eq!(status, StatusCode::OK, "A04 outpatient must AA: {body}");
    assert!(body.contains("MSA|AA|MSG-A04-O"));
    let enc_o = body
        .split("encounter=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .expect("AA must report encounter=<uuid>")
        .to_string();
    let id_o = uuid::Uuid::parse_str(&enc_o).expect("parse");
    let e = EncounterRepository::find_by_id(&state.db, id_o)
        .await
        .expect("find")
        .expect("encounter exists");
    assert_eq!(e.class, EncounterClass::Outpatient);
    assert_eq!(e.status, EncounterStatus::Arrived);

    // Emergency: PV1-2 = "E".
    let a04_e = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A04|MSG-A04-E|P|2.5\r\
PID|||{mrn_e}||Doe^Jane||19800520|F\r\
PV1||E|\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/register", a04_e).await;
    assert_eq!(status, StatusCode::OK, "A04 emergency must AA: {body}");
    assert!(body.contains("MSA|AA|MSG-A04-E"));
    let enc_e = body
        .split("encounter=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .expect("AA must report encounter=<uuid>")
        .to_string();
    let id_e = uuid::Uuid::parse_str(&enc_e).expect("parse");
    let e = EncounterRepository::find_by_id(&state.db, id_e)
        .await
        .expect("find")
        .expect("encounter exists");
    assert_eq!(e.class, EncounterClass::Emergency);
    assert_eq!(e.status, EncounterStatus::Arrived);

    // Same MRN second visit: dedup the patient but make a fresh encounter.
    let a04_repeat = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A04|MSG-A04-RPT|P|2.5\r\
PID|||{mrn_o}||Smith^John||19700115|M\r\
PV1||O|\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/register", a04_repeat).await;
    assert_eq!(status, StatusCode::OK, "repeat A04 must AA: {body}");
    assert!(body.contains("matched existing patient"));
    let enc_rpt = body
        .split("encounter=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .expect("AA must report encounter=<uuid>")
        .to_string();
    assert_ne!(enc_rpt, enc_o, "repeat visit must create a fresh encounter");

    // Missing PV1 → 400 + AE.
    let a04_nopv1 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A04|MSG-A04-NOPV1|P|2.5\r\
PID|||MRN-NO-PV1-{suffix}||Test^User||19900101|M\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/register", a04_nopv1).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "missing PV1: {body}");
    assert!(body.contains("MSA|AE|MSG-A04-NOPV1"));

    // Wrong message type at /register → 400 + AE.
    let bogus = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A01|MSG-A04-WRONG|P|2.5\r\
PID|||MRN-WRONG-{suffix}||X^Y||19700101|M\r\
PV1||I|WARD^^BED-X\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/register", bogus).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("MSA|AE|MSG-A04-WRONG"));
    assert!(body.contains("expected ADT^A04"));
}

#[tokio::test]
async fn hl7v2_adt_a05_pre_admits_reserves_bed_plans_encounter() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_adt_a05_pre_admits_reserves_bed_plans_encounter"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    use patient_administration_system::db::repositories::bed::BedRepository;
    use patient_administration_system::db::repositories::encounter::EncounterRepository;
    use patient_administration_system::models::encounter::{EncounterClass, EncounterStatus};
    use patient_administration_system::models::facility::BedStatus;

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let bed_code = format!("BED-A05-{suffix}");
    let mrn = format!("MRN-A05-{suffix}");
    let family = format!("A05{suffix}");

    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC A05", "code": format!("FAC-A05-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({ "facility_id": facility_id, "name": "Ward A05", "code": format!("WARD-A05-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ward_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({ "ward_id": ward_id, "name": "Room A05", "code": format!("ROOM-A05-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let room_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, _) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Bed A05", "code": bed_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Step 1: A05 pre-admit reserves the bed and plans the encounter.
    let pre_admit = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530100000||ADT^A05|MSG-A05-{suffix}|P|2.5\r\
EVN|A05|20260530100000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|WARD-A05^ROOM-A05^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/pre-admit", pre_admit).await;
    assert_eq!(status, StatusCode::OK, "pre-admit: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A05-{suffix}")));

    let enc_str = body
        .split("encounter=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .expect("AA must report encounter=<uuid>")
        .to_string();
    let enc_id = uuid::Uuid::parse_str(&enc_str).expect("parse");
    let enc = EncounterRepository::find_by_id(&state.db, enc_id)
        .await
        .expect("find")
        .expect("encounter exists");
    assert_eq!(enc.class, EncounterClass::Inpatient);
    assert_eq!(enc.status, EncounterStatus::Planned);

    let bed = BedRepository::find_by_code(&state.db, &bed_code)
        .await
        .expect("find")
        .expect("bed exists");
    assert_eq!(
        bed.status,
        BedStatus::Reserved,
        "bed should be Reserved after A05"
    );

    // Step 2: A05 on an already-reserved bed → 409 + AE.
    let pre_admit_dup = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530101000||ADT^A05|MSG-A05-DUP-{suffix}|P|2.5\r\
EVN|A05|20260530101000\r\
PID|1||MRN-A05-OTHER-{suffix}^^^FAC^MR||Other^Person||19850101|M\r\
PV1|1|I|WARD-A05^ROOM-A05^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/pre-admit", pre_admit_dup).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "pre-admit on reserved bed: {body}"
    );
    assert!(body.contains(&format!("MSA|AE|MSG-A05-DUP-{suffix}")));

    // Step 3: A05 with unknown bed code → 404 + AE.
    let pre_admit_404 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530102000||ADT^A05|MSG-A05-404-{suffix}|P|2.5\r\
EVN|A05|20260530102000\r\
PID|1||MRN-A05-404-{suffix}^^^FAC^MR||Ghost^Person||19850101|M\r\
PV1|1|I|WARD-A05^ROOM-A05^BED-DOES-NOT-EXIST-{suffix}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/pre-admit", pre_admit_404).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown bed: {body}");
    assert!(body.contains(&format!("MSA|AE|MSG-A05-404-{suffix}")));

    // Step 4: wrong message type at /pre-admit → 400 + AE.
    let bogus = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530103000||ADT^A01|MSG-A05-WRONG-{suffix}|P|2.5\r\
EVN|A01|20260530103000\r\
PID|1||MRN-A05-WRONG-{suffix}^^^FAC^MR||Wrong^Type||19850101|M\r\
PV1|1|I|WARD-A05^ROOM-A05^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/pre-admit", bogus).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A05-WRONG-{suffix}")));
    assert!(body.contains("expected ADT^A05"));
}

#[tokio::test]
async fn hl7v2_adt_a21_a22_leave_of_absence_round_trip() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_adt_a21_a22_leave_of_absence_round_trip"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    use patient_administration_system::db::repositories::admission::AdmissionRepository;
    use patient_administration_system::db::repositories::bed::BedRepository;
    use patient_administration_system::db::repositories::encounter::EncounterRepository;
    use patient_administration_system::db::repositories::patient::PatientRepository;
    use patient_administration_system::models::encounter::EncounterStatus;
    use patient_administration_system::models::facility::BedStatus;

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let bed_code = format!("BED-A21-{suffix}");
    let mrn = format!("MRN-A21-{suffix}");
    let family = format!("A21{suffix}");

    // Bootstrap.
    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC A21", "code": format!("FAC-A21-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({ "facility_id": facility_id, "name": "Ward A21", "code": format!("WARD-A21-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ward_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({ "ward_id": ward_id, "name": "Room A21", "code": format!("ROOM-A21-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let room_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, _) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Bed A21", "code": bed_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Step 1: admit (puts encounter InProgress, bed Occupied).
    let admit = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530090000||ADT^A01|MSG-A21-ADM-{suffix}|P|2.5\r\
EVN|A01|20260530090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|WARD-A21^ROOM-A21^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/admit", admit).await;
    assert_eq!(status, StatusCode::OK, "admit: {body}");

    let patient = PatientRepository::find_by_identifier_value(&state.db, &mrn)
        .await
        .expect("find patient")
        .expect("patient exists");
    let adm = AdmissionRepository::find_open_for_patient(&state.db, patient.id)
        .await
        .expect("find adm")
        .expect("open admission");

    // Step 2: A21 leave-start — encounter InProgress → OnLeave.
    let a21 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530100000||ADT^A21|MSG-A21-{suffix}|P|2.5\r\
EVN|A21|20260530100000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/leave-start", a21).await;
    assert_eq!(status, StatusCode::OK, "leave-start: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A21-{suffix}")));

    let enc = EncounterRepository::find_by_id(&state.db, adm.encounter_id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(enc.status, EncounterStatus::OnLeave);
    let bed = BedRepository::find_by_code(&state.db, &bed_code)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(
        bed.status,
        BedStatus::Occupied,
        "bed should remain Occupied during LOA"
    );

    // Step 3: second A21 → 409 (encounter no longer InProgress).
    let a21_again = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530101000||ADT^A21|MSG-A21-DUP-{suffix}|P|2.5\r\
EVN|A21|20260530101000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/leave-start", a21_again).await;
    assert_eq!(status, StatusCode::CONFLICT, "re-start: {body}");
    assert!(body.contains(&format!("MSA|AE|MSG-A21-DUP-{suffix}")));

    // Step 4: A22 leave-end — encounter OnLeave → InProgress.
    let a22 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530120000||ADT^A22|MSG-A22-{suffix}|P|2.5\r\
EVN|A22|20260530120000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/leave-end", a22).await;
    assert_eq!(status, StatusCode::OK, "leave-end: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A22-{suffix}")));

    let enc = EncounterRepository::find_by_id(&state.db, adm.encounter_id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(enc.status, EncounterStatus::InProgress);

    // Step 5: second A22 → 409 (encounter no longer OnLeave).
    let a22_again = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530121000||ADT^A22|MSG-A22-DUP-{suffix}|P|2.5\r\
EVN|A22|20260530121000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/leave-end", a22_again).await;
    assert_eq!(status, StatusCode::CONFLICT, "re-end: {body}");
    assert!(body.contains(&format!("MSA|AE|MSG-A22-DUP-{suffix}")));

    // Step 6: wrong message type at /leave-start → 400 + AE.
    let bogus = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530130000||ADT^A01|MSG-A21-WRONG-{suffix}|P|2.5\r\
EVN|A01|20260530130000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/leave-start", bogus).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A21-WRONG-{suffix}")));
    assert!(body.contains("expected ADT^A21"));
}

#[tokio::test]
async fn hl7v2_adt_a06_promotes_outpatient_to_inpatient() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_adt_a06_promotes_outpatient_to_inpatient"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    use patient_administration_system::db::repositories::bed::BedRepository;
    use patient_administration_system::db::repositories::encounter::EncounterRepository;
    use patient_administration_system::models::encounter::{EncounterClass, EncounterStatus};
    use patient_administration_system::models::facility::BedStatus;

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let bed_code = format!("BED-A06-{suffix}");
    let ward_code = format!("WARD-A06-{suffix}");
    let room_code = format!("ROOM-A06-{suffix}");
    let mrn = format!("MRN-A06-{suffix}");
    let family = format!("A06{suffix}");

    // Bootstrap facility/ward/room/bed.
    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC A06", "code": format!("FAC-A06-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({ "facility_id": facility_id, "name": "Ward A06", "code": ward_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ward_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({ "ward_id": ward_id, "name": "Room A06", "code": room_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let room_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, _) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Bed A06", "code": bed_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Step 1: A04 register outpatient.
    let a04 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530090000||ADT^A04|MSG-A06-A04-{suffix}|P|2.5\r\
PID|||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1||O|\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/register", a04).await;
    assert_eq!(status, StatusCode::OK, "register: {body}");

    // Step 2: A06 change-to-inpatient.
    let a06 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530100000||ADT^A06|MSG-A06-{suffix}|P|2.5\r\
EVN|A06|20260530100000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|{ward_code}^{room_code}^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/change-to-inpatient", a06).await;
    assert_eq!(status, StatusCode::OK, "A06: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A06-{suffix}")));

    let enc_str = body
        .split("encounter=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .expect("encounter=<uuid>")
        .to_string();
    let enc_id = uuid::Uuid::parse_str(&enc_str).expect("parse");
    let enc = EncounterRepository::find_by_id(&state.db, enc_id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(enc.class, EncounterClass::Inpatient);
    assert_eq!(enc.status, EncounterStatus::InProgress);

    let bed = BedRepository::find_by_code(&state.db, &bed_code)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(bed.status, BedStatus::Occupied);

    // Step 3: second A06 (no remaining active ambulatory) → 404.
    let a06_dup = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530101000||ADT^A06|MSG-A06-DUP-{suffix}|P|2.5\r\
EVN|A06|20260530101000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|{ward_code}^{room_code}^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/change-to-inpatient", a06_dup).await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::CONFLICT,
        "second A06 should fail: {status} {body}"
    );
    assert!(body.contains(&format!("MSA|AE|MSG-A06-DUP-{suffix}")));

    // Step 4: unknown patient → 404.
    let a06_404 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530102000||ADT^A06|MSG-A06-404-{suffix}|P|2.5\r\
EVN|A06|20260530102000\r\
PID|1||MRN-A06-NO-{suffix}^^^FAC^MR||Ghost^Person||19850101|M\r\
PV1|1|I|{ward_code}^{room_code}^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/change-to-inpatient", a06_404).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown patient: {body}");
    assert!(body.contains(&format!("MSA|AE|MSG-A06-404-{suffix}")));

    // Step 5: wrong message type → 400.
    let bogus = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530103000||ADT^A01|MSG-A06-WRONG-{suffix}|P|2.5\r\
EVN|A01|20260530103000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|{ward_code}^{room_code}^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/change-to-inpatient", bogus).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A06-WRONG-{suffix}")));
    assert!(body.contains("expected ADT^A06"));
}

#[tokio::test]
async fn hl7v2_adt_a07_demotes_inpatient_to_outpatient() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_adt_a07_demotes_inpatient_to_outpatient"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    use patient_administration_system::db::repositories::admission::AdmissionRepository;
    use patient_administration_system::db::repositories::bed::BedRepository;
    use patient_administration_system::db::repositories::encounter::EncounterRepository;
    use patient_administration_system::db::repositories::patient::PatientRepository;
    use patient_administration_system::models::encounter::{EncounterClass, EncounterStatus};
    use patient_administration_system::models::facility::BedStatus;

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let bed_code = format!("BED-A07-{suffix}");
    let ward_code = format!("WARD-A07-{suffix}");
    let room_code = format!("ROOM-A07-{suffix}");
    let mrn = format!("MRN-A07-{suffix}");
    let family = format!("A07{suffix}");

    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC A07", "code": format!("FAC-A07-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({ "facility_id": facility_id, "name": "Ward A07", "code": ward_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ward_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({ "ward_id": ward_id, "name": "Room A07", "code": room_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let room_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, _) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Bed A07", "code": bed_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Step 1: admit (Inpatient + Occupied).
    let admit = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530090000||ADT^A01|MSG-A07-ADM-{suffix}|P|2.5\r\
EVN|A01|20260530090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|{ward_code}^{room_code}^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/admit", admit).await;
    assert_eq!(status, StatusCode::OK, "admit: {body}");

    let patient = PatientRepository::find_by_identifier_value(&state.db, &mrn)
        .await
        .expect("find")
        .expect("exists");
    let adm = AdmissionRepository::find_open_for_patient(&state.db, patient.id)
        .await
        .expect("find adm")
        .expect("open admission");

    // Step 2: A07 demote.
    let a07 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530100000||ADT^A07|MSG-A07-{suffix}|P|2.5\r\
EVN|A07|20260530100000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/change-to-outpatient", a07).await;
    assert_eq!(status, StatusCode::OK, "A07: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A07-{suffix}")));

    let enc = EncounterRepository::find_by_id(&state.db, adm.encounter_id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(enc.class, EncounterClass::Outpatient);
    assert_eq!(enc.status, EncounterStatus::InProgress);

    let bed = BedRepository::find_by_code(&state.db, &bed_code)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(
        bed.status,
        BedStatus::Cleaning,
        "bed should be Cleaning after A07"
    );

    // Step 3: second A07 → no open admission anymore → 400 + AE.
    let a07_again = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530101000||ADT^A07|MSG-A07-DUP-{suffix}|P|2.5\r\
EVN|A07|20260530101000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/change-to-outpatient", a07_again).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "re-A07: {body}");
    assert!(body.contains(&format!("MSA|AE|MSG-A07-DUP-{suffix}")));
    assert!(body.contains("no currently-open admission"));

    // Step 4: wrong message type → 400.
    let bogus = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530103000||ADT^A01|MSG-A07-WRONG-{suffix}|P|2.5\r\
EVN|A01|20260530103000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/change-to-outpatient", bogus).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A07-WRONG-{suffix}")));
    assert!(body.contains("expected ADT^A07"));
}

#[tokio::test]
async fn hl7v2_adt_a23_deletes_patient_soft_with_safety_check() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping hl7v2_adt_a23_deletes_patient_soft_with_safety_check"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    use patient_administration_system::db::repositories::patient::PatientRepository;

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let mrn_uncomplicated = format!("MRN-A23-{suffix}");
    let mrn_admitted = format!("MRN-A23-ADM-{suffix}");
    let bed_code = format!("BED-A23-{suffix}");
    let ward_code = format!("WARD-A23-{suffix}");
    let room_code = format!("ROOM-A23-{suffix}");
    let family = format!("A23{suffix}");

    // Step 1: register an uncomplicated patient via A28 (no admission).
    let a28 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530090000||ADT^A28|MSG-A23-REG-{suffix}|P|2.5\r\
EVN|A28|20260530090000\r\
PID|1||{mrn_uncomplicated}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/patient", a28).await;
    assert_eq!(status, StatusCode::OK, "register: {body}");

    // Step 2: A23 deletes the patient — soft delete + Tantivy drop.
    let a23 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530100000||ADT^A23|MSG-A23-{suffix}|P|2.5\r\
EVN|A23|20260530100000\r\
PID|1||{mrn_uncomplicated}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/delete-patient", a23).await;
    assert_eq!(status, StatusCode::OK, "A23: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A23-{suffix}")));
    // Patient is no longer findable via the active lookup.
    let still_there = PatientRepository::find_by_identifier_value(&state.db, &mrn_uncomplicated)
        .await
        .expect("find");
    assert!(
        still_there.is_none(),
        "soft-deleted patient must not appear in active lookup"
    );

    // Step 3: A23 on unknown MRN → 404 + AE.
    let a23_404 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530101000||ADT^A23|MSG-A23-404-{suffix}|P|2.5\r\
EVN|A23|20260530101000\r\
PID|1||MRN-DOES-NOT-EXIST-{suffix}^^^FAC^MR||Ghost^Person||19850101|M\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/delete-patient", a23_404).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "404: {body}");
    assert!(body.contains(&format!("MSA|AE|MSG-A23-404-{suffix}")));

    // Step 4: bootstrap facility/ward/room/bed + admit a second patient,
    // then A23 must refuse (409 + AE) — safety check.
    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC A23", "code": format!("FAC-A23-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({ "facility_id": facility_id, "name": "Ward A23", "code": ward_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ward_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({ "ward_id": ward_id, "name": "Room A23", "code": room_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let room_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, _) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Bed A23", "code": bed_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let admit = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530120000||ADT^A01|MSG-A23-ADM-{suffix}|P|2.5\r\
EVN|A01|20260530120000\r\
PID|1||{mrn_admitted}^^^FAC^MR||AdmFamily{suffix}^Jane||19900115|F\r\
PV1|1|I|{ward_code}^{room_code}^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/admit", admit).await;
    assert_eq!(status, StatusCode::OK, "admit: {body}");

    let a23_busy = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530121000||ADT^A23|MSG-A23-BUSY-{suffix}|P|2.5\r\
EVN|A23|20260530121000\r\
PID|1||{mrn_admitted}^^^FAC^MR||AdmFamily{suffix}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/delete-patient", a23_busy).await;
    assert_eq!(status, StatusCode::CONFLICT, "admitted patient: {body}");
    assert!(body.contains(&format!("MSA|AE|MSG-A23-BUSY-{suffix}")));
    assert!(body.contains("open admission"));

    // Step 5: wrong message type at /delete-patient → 400 + AE.
    let bogus = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530130000||ADT^A28|MSG-A23-WRONG-{suffix}|P|2.5\r\
PID|1||{mrn_admitted}^^^FAC^MR||X^Y||19850101|M\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/delete-patient", bogus).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A23-WRONG-{suffix}")));
    assert!(body.contains("expected ADT^A23"));
}

#[tokio::test]
async fn hl7v2_adt_a38_cancels_pre_admit() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping hl7v2_adt_a38_cancels_pre_admit");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    use patient_administration_system::db::repositories::bed::BedRepository;
    use patient_administration_system::db::repositories::encounter::EncounterRepository;
    use patient_administration_system::models::encounter::EncounterStatus;
    use patient_administration_system::models::facility::BedStatus;

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let bed_code = format!("BED-A38-{suffix}");
    let mrn = format!("MRN-A38-{suffix}");
    let family = format!("A38{suffix}");

    // Bootstrap.
    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC A38", "code": format!("FAC-A38-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({ "facility_id": facility_id, "name": "Ward A38", "code": format!("WARD-A38-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ward_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({ "ward_id": ward_id, "name": "Room A38", "code": format!("ROOM-A38-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let room_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, _) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Bed A38", "code": bed_code.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Step 1: pre-admit (A05) reserves the bed.
    let a05 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530090000||ADT^A05|MSG-A38-A05-{suffix}|P|2.5\r\
EVN|A05|20260530090000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|WARD-A38^ROOM-A38^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/pre-admit", a05).await;
    assert_eq!(status, StatusCode::OK, "pre-admit: {body}");
    let enc_str = body
        .split("encounter=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .expect("encounter=")
        .to_string();
    let enc_id = uuid::Uuid::parse_str(&enc_str).expect("parse");

    // Step 2: A38 cancel-pre-admit — bed released, encounter cancelled.
    let a38 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530100000||ADT^A38|MSG-A38-{suffix}|P|2.5\r\
EVN|A38|20260530100000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|WARD-A38^ROOM-A38^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-pre-admit", a38).await;
    assert_eq!(status, StatusCode::OK, "cancel-pre-admit: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A38-{suffix}")));

    let bed = BedRepository::find_by_code(&state.db, &bed_code)
        .await
        .expect("find")
        .expect("bed exists");
    assert_eq!(
        bed.status,
        BedStatus::Available,
        "bed should be Available after A38"
    );
    let enc = EncounterRepository::find_by_id(&state.db, enc_id)
        .await
        .expect("find")
        .expect("encounter exists");
    assert_eq!(
        enc.status,
        EncounterStatus::Cancelled,
        "encounter should be Cancelled after A38"
    );

    // Step 3: re-cancel — bed is now Available, not Reserved → 409.
    let a38_again = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530101000||ADT^A38|MSG-A38-DUP-{suffix}|P|2.5\r\
EVN|A38|20260530101000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|WARD-A38^ROOM-A38^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-pre-admit", a38_again).await;
    assert_eq!(status, StatusCode::CONFLICT, "re-cancel: {body}");
    assert!(body.contains(&format!("MSA|AE|MSG-A38-DUP-{suffix}")));

    // Step 4: unknown patient MRN → 404 + AE.
    let a38_404 = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530102000||ADT^A38|MSG-A38-404-{suffix}|P|2.5\r\
EVN|A38|20260530102000\r\
PID|1||MRN-DOES-NOT-EXIST-{suffix}^^^FAC^MR||Nobody^Test||19850101|M\r\
PV1|1|I|WARD-A38^ROOM-A38^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-pre-admit", a38_404).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown patient: {body}");
    assert!(body.contains(&format!("MSA|AE|MSG-A38-404-{suffix}")));

    // Step 5: wrong message type → 400 + AE.
    let bogus = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260530103000||ADT^A05|MSG-A38-WRONG-{suffix}|P|2.5\r\
EVN|A05|20260530103000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|WARD-A38^ROOM-A38^{bed_code}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-pre-admit", bogus).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A38-WRONG-{suffix}")));
    assert!(body.contains("expected ADT^A38"));
}

#[tokio::test]
async fn hl7v2_adt_a12_cancels_transfer() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping hl7v2_adt_a12_cancels_transfer");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let publisher = Arc::new(InMemoryEventPublisher::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let search = Arc::new(SearchEngine::new(tmp.path().to_str().unwrap()).expect("search"));
    let state = AppState::new(db.clone(), publisher).with_search(search);
    let app = router(state.clone());

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let bed_a = format!("BED-A12A-{suffix}");
    let bed_b = format!("BED-A12B-{suffix}");
    let mrn = format!("MRN-A12-{suffix}");
    let family = format!("A12{suffix}");

    // Bootstrap facility / ward / room + 2 beds.
    let (status, body) = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC A12", "code": format!("FAC-A12-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "facility: {body}");
    let facility_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/wards",
        serde_json::json!({ "facility_id": facility_id, "name": "Ward A12", "code": format!("WARD-A12-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ward_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({ "ward_id": ward_id, "name": "Room A12", "code": format!("ROOM-A12-{suffix}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let room_id = body["data"]["id"].as_str().unwrap().to_string();
    let (status, _) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Bed A12A", "code": bed_a.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "Bed A12B", "code": bed_b.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Step 1: admit to bed A.
    let admit = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A01|MSG-ADM-{suffix}|P|2.5\r\
EVN|A01|20260523120000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|WARD-A12^ROOM-A12^{bed_a}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/admit", admit).await;
    assert_eq!(status, StatusCode::OK, "admit: {body}");

    // Step 2: transfer to bed B.
    let xfer = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523121000||ADT^A02|MSG-XFER-{suffix}|P|2.5\r\
EVN|A02|20260523121000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r\
PV1|1|I|WARD-A12^ROOM-A12^{bed_b}\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/transfer", xfer).await;
    assert_eq!(status, StatusCode::OK, "transfer: {body}");

    use patient_administration_system::db::repositories::bed::BedRepository;
    use patient_administration_system::models::facility::BedStatus;
    let bed_a_row = BedRepository::find_by_code(&state.db, &bed_a)
        .await
        .expect("find a")
        .expect("bed a exists");
    let bed_b_row = BedRepository::find_by_code(&state.db, &bed_b)
        .await
        .expect("find b")
        .expect("bed b exists");
    assert_eq!(
        bed_a_row.status,
        BedStatus::Cleaning,
        "A should be Cleaning after transfer"
    );
    assert_eq!(
        bed_b_row.status,
        BedStatus::Occupied,
        "B should be Occupied after transfer"
    );

    // Step 3: cancel-transfer — patient restored to A, B flips to Cleaning.
    let cancel = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523122000||ADT^A12|MSG-A12-{suffix}|P|2.5\r\
EVN|A12|20260523122000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-transfer", cancel).await;
    assert_eq!(status, StatusCode::OK, "cancel-transfer: {body}");
    assert!(body.contains(&format!("MSA|AA|MSG-A12-{suffix}")));

    let bed_a_row = BedRepository::find_by_code(&state.db, &bed_a)
        .await
        .expect("find a")
        .expect("bed a exists");
    let bed_b_row = BedRepository::find_by_code(&state.db, &bed_b)
        .await
        .expect("find b")
        .expect("bed b exists");
    assert_eq!(
        bed_a_row.status,
        BedStatus::Occupied,
        "A should be Occupied after cancel"
    );
    assert_eq!(
        bed_b_row.status,
        BedStatus::Cleaning,
        "B should be Cleaning after cancel"
    );

    // Step 4: re-cancelling fails (no more transfer history).
    let cancel_again = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523123000||ADT^A12|MSG-A12-DUP-{suffix}|P|2.5\r\
EVN|A12|20260523123000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-transfer", cancel_again).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "re-cancel: {body}");
    assert!(body.contains(&format!("MSA|AE|MSG-A12-DUP-{suffix}")));

    // Step 5: wrong message type at /cancel-transfer.
    let bogus = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523124000||ADT^A11|MSG-A12-WRONG-{suffix}|P|2.5\r\
EVN|A11|20260523124000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane||19900115|F\r"
    );
    let (status, _, body) = post_text(&app, "/api/hl7/v2/cancel-transfer", bogus).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains(&format!("MSA|AE|MSG-A12-WRONG-{suffix}")));
    assert!(body.contains("expected ADT^A12"));
}

async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    (status, value)
}

async fn json_post(
    app: &axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "expected JSON, status={status} body={}, err={e}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, value)
}
