//! SeaORM entity for the `patients` table.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "patients")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub mpi_id: Option<Uuid>,
    pub active: bool,
    pub name: serde_json::Value,
    pub additional_names: serde_json::Value,
    pub identifiers: serde_json::Value,
    pub telecom: serde_json::Value,
    pub addresses: serde_json::Value,
    pub gender: String,
    pub birth_date: Option<chrono::NaiveDate>,
    pub deceased: bool,
    pub deceased_datetime: Option<DateTimeWithTimeZone>,
    pub emergency_contacts: serde_json::Value,
    pub marital_status: Option<String>,
    /// When set, this row is a merge tombstone — its identity has been
    /// merged into the patient with `id = replaced_by`. Tombstones are
    /// excluded from the default list / search paths (v0.11).
    pub replaced_by: Option<Uuid>,
    pub deleted_at: Option<DateTimeWithTimeZone>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
