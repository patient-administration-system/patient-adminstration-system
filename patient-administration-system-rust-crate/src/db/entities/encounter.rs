//! SeaORM entity for the `encounters` table.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "encounters")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub patient_id: Uuid,
    pub class: String,
    pub status: String,
    pub period_start: DateTimeWithTimeZone,
    pub period_end: Option<DateTimeWithTimeZone>,
    pub practitioner_id: Option<Uuid>,
    pub department_id: Option<Uuid>,
    pub reason: Option<String>,
    pub deleted_at: Option<DateTimeWithTimeZone>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
