//! Loco-rs application hooks.
//!
//! Implements the [`loco_rs::app::Hooks`] trait — Loco calls into this on
//! boot, on every request (via the wired routes), and on `loco-cli db …`.

use std::path::Path;

use async_trait::async_trait;
use loco_rs::{
    Result,
    app::{AppContext, Hooks},
    bgworker::Queue,
    boot::{BootResult, StartMode, create_app},
    config::Config,
    controller::AppRoutes,
    environment::Environment,
    task::Tasks,
};
use migration::Migrator;

use crate::controllers;

pub struct App;

#[async_trait]
impl Hooks for App {
    fn app_name() -> &'static str {
        env!("CARGO_PKG_NAME")
    }

    fn app_version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    async fn boot(
        mode: StartMode,
        environment: &Environment,
        config: Config,
    ) -> Result<BootResult> {
        create_app::<Self, Migrator>(mode, environment, config).await
    }

    fn routes(_ctx: &AppContext) -> AppRoutes {
        AppRoutes::with_default_routes()
            .add_route(controllers::dashboard::routes())
            .add_route(controllers::patients::routes())
            .add_route(controllers::wards::routes())
            .add_route(controllers::rtt::routes())
            .add_route(controllers::admissions::routes())
            .add_route(controllers::appointments::routes())
            .add_route(controllers::letters::routes())
            .add_route(controllers::health::routes())
    }

    fn register_tasks(_tasks: &mut Tasks) {
        // No background tasks yet. The PAS Axum binary owns the outbox
        // dispatcher; patient-administration-system-frontend is read-mostly.
    }

    async fn connect_workers(_ctx: &AppContext, _queue: &Queue) -> Result<()> {
        // No workers registered — patient-administration-system-frontend is HTTP-only.
        Ok(())
    }

    async fn truncate(_ctx: &AppContext) -> Result<()> {
        // patient-administration-system-frontend never truncates the shared PAS database; truncation
        // would corrupt the system of record. Test-mode truncation goes
        // through the PAS Axum binary's own test harness.
        Ok(())
    }

    async fn seed(_ctx: &AppContext, _path: &Path) -> Result<()> {
        // Seeding lives in the PAS Axum binary (`pas-seed`). patient-administration-system-frontend
        // is a read-mostly view layer and intentionally cannot mutate
        // the shared schema.
        Ok(())
    }
}
