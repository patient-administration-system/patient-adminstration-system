//! SeaORM entity for the `outbox_events` table.
//!
//! Schema lives in two migrations:
//! - `m20260520_000001_init` — base columns (`id`, `event_type`, `payload`,
//!   `published`, `at`).
//! - `m20260525_000002_outbox_dlq` — retry-tracking columns (`retry_count`,
//!   `last_attempted_at`, `last_error`).

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "outbox_events")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub published: bool,
    pub at: DateTimeWithTimeZone,
    /// Count of publish attempts that failed. Reset to 0 when the row is
    /// replayed from the dead-letter queue. Compared against
    /// `PAS_OUTBOX_MAX_RETRIES` by the dispatcher.
    pub retry_count: i32,
    /// Timestamp of the most recent publish attempt (success or failure).
    /// `None` until the dispatcher first touches the row.
    pub last_attempted_at: Option<DateTimeWithTimeZone>,
    /// Diagnostic from the most recent failed publish attempt. Cleared when
    /// the row is replayed; never set when the most recent attempt
    /// succeeded.
    pub last_error: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
