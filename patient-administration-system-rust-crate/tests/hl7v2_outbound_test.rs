//! Integration test for the outbound HL7 v2 MLLP publisher.
//!
//! Stands up a fake EMR (a tiny TCP listener that responds with AA ACK to
//! every frame and records what it received), bootstraps a patient + bed
//! via the REST API, then calls `Hl7v2MllpPublisher::publish` directly with
//! a hand-built `EncounterAdmitted` payload and asserts the fake EMR saw a
//! well-formed ADT^A01.
//!
//! Gated on `DATABASE_URL`. Skips silently otherwise.

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use migration::MigratorTrait;
use patient_administration_system::api::rest::{AppState, router};
use patient_administration_system::db::connect;
use patient_administration_system::hl7v2::{AckCode, ack, mllp};
use patient_administration_system::streaming::{DomainEvent, EventPublisher, Hl7v2MllpPublisher};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

#[derive(Default)]
struct FakeEmr {
    frames: Vec<Vec<u8>>,
}

async fn json_post(app: &axum::Router, uri: &str, body: serde_json::Value) -> serde_json::Value {
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
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "expected JSON, status={status} body={}, err={e}",
            String::from_utf8_lossy(&bytes)
        )
    });
    assert_eq!(status, StatusCode::OK, "{uri} returned non-200: {v}");
    v
}

#[tokio::test]
async fn outbound_publisher_emits_adt_a01_to_fake_emr() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping outbound_publisher_emits_adt_a01_to_fake_emr"
            );
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    // --- Stand up the fake EMR on an ephemeral port. ---
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let peer = listener.local_addr().expect("local_addr").to_string();
    let received: Arc<Mutex<FakeEmr>> = Arc::new(Mutex::new(FakeEmr::default()));
    let received_for_task = received.clone();
    let emr_task = tokio::spawn(async move {
        loop {
            let (sock, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => return,
            };
            let recv = received_for_task.clone();
            tokio::spawn(async move {
                let (mut rd, mut wr) = sock.into_split();
                // We only handle the first frame on each accepted connection;
                // the publisher opens a fresh TCP connection per event.
                let frame = match mllp::read_frame(&mut rd).await {
                    Ok(Some(f)) => f,
                    _ => return,
                };
                // Pull MSH-10 (message control id) so the ACK refers to it.
                let body_str = String::from_utf8_lossy(&frame).to_string();
                let mcid = body_str
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split('|')
                    .nth(9)
                    .unwrap_or("UNKNOWN")
                    .to_string();
                recv.lock().await.frames.push(frame);
                let ack_body = ack("EMR", "PAS", &mcid, AckCode::Accept, None);
                let _ = mllp::write_frame(&mut wr, ack_body.as_bytes()).await;
                let _ = wr.shutdown().await;
            });
        }
    });

    // --- Bootstrap a real patient + bed via REST so the publisher's repo
    //     lookups will hit live rows. ---
    let publisher =
        Arc::new(patient_administration_system::streaming::InMemoryEventPublisher::new());
    let state = AppState::new(db.clone(), publisher);
    let app = router(state);

    let suffix = Uuid::new_v4().simple().to_string();
    let bed_code = format!("BED-OUT-{suffix}");
    let facility = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC OUT", "code": format!("FAC-OUT-{suffix}") }),
    )
    .await;
    let facility_id = facility["data"]["id"].as_str().unwrap().to_string();
    let ward = json_post(
        &app,
        "/api/wards",
        serde_json::json!({
            "facility_id": facility_id,
            "name": "Ward OUT",
            "code": format!("WARD-OUT-{suffix}")
        }),
    )
    .await;
    let ward_id = ward["data"]["id"].as_str().unwrap().to_string();
    let room = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({
            "ward_id": ward_id,
            "name": "Room OUT",
            "code": format!("ROOM-OUT-{suffix}")
        }),
    )
    .await;
    let room_id = room["data"]["id"].as_str().unwrap().to_string();
    let bed = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "BedOut", "code": bed_code.clone() }),
    )
    .await;
    let bed_id = bed["data"]["id"].as_str().unwrap().to_string();
    let bed_uuid: Uuid = bed_id.parse().expect("bed uuid");

    let patient = json_post(
        &app,
        "/api/patients",
        serde_json::json!({
            "family": format!("OutTest{suffix}"),
            "given": ["Outbound"],
            "gender": "female",
            "birth_date": "1990-01-15"
        }),
    )
    .await;
    let patient_id: Uuid = patient["data"]["id"]
        .as_str()
        .expect("patient id")
        .parse()
        .expect("uuid");

    // --- Build the outbound publisher and publish an EncounterAdmitted event. ---
    let publisher = Hl7v2MllpPublisher::new(db.clone(), &peer, "PAS", "EMR");
    let event = DomainEvent::new(
        "EncounterAdmitted",
        serde_json::json!({
            "patient_id": patient_id,
            "bed_id": bed_uuid,
            "encounter_id": Uuid::new_v4(),
            "admission_id": Uuid::new_v4(),
        }),
    );
    publisher
        .publish(event.clone())
        .await
        .expect("publish should succeed because fake EMR returns AA");

    // --- Inspect the captured frames. ---
    let frames = received.lock().await.frames.clone();
    assert_eq!(frames.len(), 1, "expected exactly one frame received");
    let body = String::from_utf8(frames[0].clone()).expect("utf8");
    assert!(body.contains("|ADT^A01|"), "no ADT^A01: {body:?}");
    assert!(body.contains("\rPV1|1|I|^^"), "no PV1: {body:?}");
    assert!(
        body.contains(&format!("^^{bed_code}")),
        "PV1 missing bed code {bed_code:?}: {body:?}"
    );
    let mrn_segment = format!("OutTest{suffix}");
    assert!(
        body.contains(&mrn_segment),
        "PID missing family name {mrn_segment:?}: {body:?}"
    );

    // --- Non-ADT event should be silently accepted (no frame sent). ---
    let irrelevant = DomainEvent::new(
        "PatientCreated",
        serde_json::json!({ "patient_id": patient_id }),
    );
    publisher
        .publish(irrelevant)
        .await
        .expect("non-ADT event drops silently");
    let frames_after = received.lock().await.frames.clone();
    assert_eq!(
        frames_after.len(),
        1,
        "non-ADT event must not produce a new frame"
    );

    // --- Bad peer host returns Err (publisher reports failure so dispatcher
    //     can retry). ---
    let bad = Hl7v2MllpPublisher::new(db.clone(), "127.0.0.1:1", "PAS", "EMR");
    let err = bad
        .publish(DomainEvent::new(
            "EncounterAdmitted",
            serde_json::json!({ "patient_id": patient_id, "bed_id": bed_uuid }),
        ))
        .await
        .expect_err("bad peer must error");
    let _ = err;

    emr_task.abort();
}

/// Sanity: a fake EMR that returns AE → publisher reports Err.
#[tokio::test]
async fn outbound_publisher_reports_err_on_ae_ack() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!("DATABASE_URL not set; skipping outbound_publisher_reports_err_on_ae_ack");
            return;
        }
    };
    let db = connect(&url).await.expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let peer = listener.local_addr().expect("local_addr").to_string();
    let emr_task = tokio::spawn(async move {
        let (sock, _) = match listener.accept().await {
            Ok(v) => v,
            Err(_) => return,
        };
        let (mut rd, mut wr) = sock.into_split();
        let _frame = match mllp::read_frame(&mut rd).await {
            Ok(Some(f)) => f,
            _ => return,
        };
        let ack_body = ack("EMR", "PAS", "X", AckCode::AppError, Some("simulated"));
        let _ = mllp::write_frame(&mut wr, ack_body.as_bytes()).await;
    });

    // Use any valid patient/bed — we need them to exist so the publisher
    // gets past the lookup step.
    let publisher_in_mem =
        Arc::new(patient_administration_system::streaming::InMemoryEventPublisher::new());
    let state = AppState::new(db.clone(), publisher_in_mem);
    let app = router(state);
    let suffix = Uuid::new_v4().simple().to_string();
    let bed_code = format!("BED-OUT2-{suffix}");

    let facility = json_post(
        &app,
        "/api/facilities",
        serde_json::json!({ "name": "FAC OUT2", "code": format!("FAC-OUT2-{suffix}") }),
    )
    .await;
    let facility_id = facility["data"]["id"].as_str().unwrap().to_string();
    let ward = json_post(
        &app,
        "/api/wards",
        serde_json::json!({
            "facility_id": facility_id,
            "name": "Ward OUT2",
            "code": format!("WARD-OUT2-{suffix}")
        }),
    )
    .await;
    let ward_id = ward["data"]["id"].as_str().unwrap().to_string();
    let room = json_post(
        &app,
        "/api/rooms",
        serde_json::json!({
            "ward_id": ward_id,
            "name": "Room OUT2",
            "code": format!("ROOM-OUT2-{suffix}")
        }),
    )
    .await;
    let room_id = room["data"]["id"].as_str().unwrap().to_string();
    let bed = json_post(
        &app,
        "/api/beds",
        serde_json::json!({ "room_id": room_id, "name": "BedOut2", "code": bed_code }),
    )
    .await;
    let bed_id: Uuid = bed["data"]["id"].as_str().unwrap().parse().unwrap();
    let patient = json_post(
        &app,
        "/api/patients",
        serde_json::json!({ "family": format!("AeTest{suffix}"), "given": ["X"], "gender": "other" }),
    )
    .await;
    let patient_id: Uuid = patient["data"]["id"].as_str().unwrap().parse().unwrap();

    let publisher = Hl7v2MllpPublisher::new(db, &peer, "PAS", "EMR");
    let err = publisher
        .publish(DomainEvent::new(
            "EncounterAdmitted",
            serde_json::json!({ "patient_id": patient_id, "bed_id": bed_id }),
        ))
        .await
        .expect_err("AE ACK must surface as Err so dispatcher retries");
    let _ = err;

    emr_task.abort();
}
