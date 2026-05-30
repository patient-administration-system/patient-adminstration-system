//! `pas-migrate` — minimal CLI wrapper around the PAS migrator.
//!
//! Reads `DATABASE_URL` from the environment (or a `.env` file via `dotenvy`)
//! and dispatches to the standard SeaORM migrator commands:
//!
//! - `up`     — apply all pending migrations (default)
//! - `down`   — revert all applied migrations
//! - `fresh`  — drop everything and re-apply
//! - `status` — print migration status

use migration::Migrator;
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL is required");
    let db: DatabaseConnection = Database::connect(&url).await?;
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "up".to_string());
    match cmd.as_str() {
        "up" => Migrator::up(&db, None).await?,
        "down" => Migrator::down(&db, None).await?,
        "fresh" => Migrator::fresh(&db).await?,
        "status" => Migrator::status(&db).await?,
        other => return Err(format!("unknown command: {other}").into()),
    }
    println!("ok");
    Ok(())
}
