//! SeaORM entity for the `outbox_dead_letters` table (v0.5.0).
//!
//! Holds outbox rows that exceeded `PAS_OUTBOX_MAX_RETRIES` failed publish
//! attempts. The dispatcher moves a row here in one transaction (insert
//! dead-letter + delete original), so an operator can review and optionally
//! replay it via `POST /api/admin/outbox/dead-letters/{id}/replay`.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "outbox_dead_letters")]
pub struct Model {
    /// New UUID for the dead-letter row. Not the same as `original_id`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// The `id` the row carried while it was in `outbox_events`. Preserved
    /// so an operator can correlate against application logs that recorded
    /// the original outbox id.
    pub original_id: Uuid,
    pub event_type: String,
    pub payload: serde_json::Value,
    /// Original `outbox_events.at` — the moment the producing transaction
    /// committed the event.
    pub created_at: DateTimeWithTimeZone,
    /// Moment the row was moved into the DLQ. Index-ordered.
    pub dead_lettered_at: DateTimeWithTimeZone,
    /// Final retry_count when the move happened — equal to the configured
    /// `PAS_OUTBOX_MAX_RETRIES` at that time.
    pub retry_count: i32,
    /// Last error the publisher returned before the move. Never empty.
    pub last_error: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
