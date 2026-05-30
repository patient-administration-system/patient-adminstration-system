//! Integration test for the HL7 v2 MLLP TCP listener.
//!
//! Gated on `DATABASE_URL`. Skips silently otherwise. Binds the MLLP
//! listener to an ephemeral port (0) so the test runner can race-free with
//! other tests, then sends a real MLLP-framed ADT^A28 down a TCP socket and
//! asserts the AA ACK comes back framed correctly.

mod common;

use migration::MigratorTrait;
use patient_administration_system::api::rest::{AppState, router};
use patient_administration_system::db::connect;
use patient_administration_system::hl7v2::{MllpServer, mllp};
use patient_administration_system::search::SearchEngine;
use patient_administration_system::streaming::InMemoryEventPublisher;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn mllp_listener_accepts_a28_and_returns_aa_ack() {
    let url = match common::database_url() {
        Some(u) => u,
        None => {
            eprintln!(
                "DATABASE_URL not set; skipping mllp_listener_accepts_a28_and_returns_aa_ack"
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

    // Bind to an ephemeral port so concurrent test runs don't collide.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener); // release the port so MllpServer can re-bind it
    let bind_str = addr.to_string();

    let server = MllpServer::new(app);
    let bind_for_task = bind_str.clone();
    let server_task = tokio::spawn(async move {
        let _ = server.serve(&bind_for_task).await;
    });

    // Tiny retry so the listener has a chance to bind before we connect.
    let mut sock = None;
    for _ in 0..50 {
        match TcpStream::connect(&bind_str).await {
            Ok(s) => {
                sock = Some(s);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
        }
    }
    let mut sock = sock.expect("connect to MLLP listener");

    // Random MRN per run so reruns don't collide on the unique identifier.
    let mrn = format!("MRN-MLLP-{}", uuid::Uuid::new_v4().simple());
    let family = format!("Mllp{}", uuid::Uuid::new_v4().simple());
    let msg = format!(
        "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A28|MSG-MLLP-1|P|2.5\r\
EVN|A28|20260523120000\r\
PID|1||{mrn}^^^FAC^MR||{family}^Jane^Marie||19900115|F\r"
    );

    // Send the frame.
    mllp::write_frame(&mut sock, msg.as_bytes())
        .await
        .expect("write MLLP frame");

    // Read the ACK frame.
    let payload = mllp::read_frame(&mut sock)
        .await
        .expect("read ack")
        .expect("Some(ack)");
    let ack = String::from_utf8(payload).expect("utf8 ack");
    assert!(
        ack.contains("MSA|AA|MSG-MLLP-1"),
        "expected AA ACK referring to MSG-MLLP-1, got: {ack:?}"
    );
    assert!(ack.starts_with("MSH|^~\\&|PAS|FAC|EMR|FAC|"));

    // Garbage frame → AR ACK
    sock.write_all(b"\x0bnot a valid v2 message\x1c\x0d")
        .await
        .expect("write garbage");
    let payload = mllp::read_frame(&mut sock)
        .await
        .expect("read AR ack")
        .expect("Some(ack)");
    let ack = String::from_utf8(payload).expect("utf8 ack");
    assert!(
        ack.contains("MSA|AR|"),
        "expected AR ACK for garbage frame, got: {ack:?}"
    );

    drop(sock);
    server_task.abort();
}
