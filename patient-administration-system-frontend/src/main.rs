#![allow(clippy::result_large_err)]

//! patient-administration-system-frontend binary entry point.
//!
//! Boots the Loco-rs app. All wiring (routes, hooks, AppContext) lives in
//! `app.rs`; this file is intentionally tiny so the CLI surface stays
//! debuggable.

use loco_rs::cli;
use migration::Migrator;
use patient_administration_system_frontend::app::App;

#[tokio::main]
async fn main() -> loco_rs::Result<()> {
    cli::main::<App, Migrator>().await
}
