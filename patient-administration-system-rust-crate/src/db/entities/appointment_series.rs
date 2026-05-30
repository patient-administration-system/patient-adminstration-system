//! SeaORM entity for the `appointment_series` table (v0.9.0).

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "appointment_series")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub patient_id: Uuid,
    pub practitioner_id: Option<Uuid>,
    pub service_type: String,
    pub start_datetime: DateTimeWithTimeZone,
    pub duration_minutes: i32,
    /// Encoded [`crate::models::appointment_series::RecurrenceRule`].
    /// Stored as a JSONB blob; never queried into directly — the
    /// service deserialises before use.
    pub rule: serde_json::Value,
    pub status: String,
    pub reason: Option<String>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
