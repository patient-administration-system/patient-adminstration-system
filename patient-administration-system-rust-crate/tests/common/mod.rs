//! Shared test harness for integration tests.
//!
//! All integration tests skip silently when `DATABASE_URL` is not set, so the
//! default `cargo test` invocation does not require a live Postgres.

use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::connect;
use patient_administration_system::streaming::InMemoryEventPublisher;
use std::sync::Arc;

pub fn database_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

#[allow(dead_code)]
pub async fn build_state(url: &str) -> AppState {
    let db = connect(url).await.expect("connect");
    let publisher = Arc::new(InMemoryEventPublisher::new());
    AppState::new(db, publisher)
}
