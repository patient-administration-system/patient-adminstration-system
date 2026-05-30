//! SeaORM entity for the `consents` table.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "consents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub patient_id: Uuid,
    pub consent_type: String,
    pub status: String,
    pub granted_date: chrono::NaiveDate,
    pub expiry_date: Option<chrono::NaiveDate>,
    pub revoked_date: Option<chrono::NaiveDate>,
    pub purpose: Option<String>,
    pub method: Option<String>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
