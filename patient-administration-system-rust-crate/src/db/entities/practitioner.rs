//! SeaORM entity for the `practitioners` table.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "practitioners")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub active: bool,
    pub name: serde_json::Value,
    pub identifiers: serde_json::Value,
    pub telecom: serde_json::Value,
    pub addresses: serde_json::Value,
    pub gender: String,
    pub birth_date: Option<chrono::NaiveDate>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
