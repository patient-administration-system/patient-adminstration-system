//! SeaORM entity for the `coverages` table (v0.10.0).

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "coverages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub patient_id: Uuid,
    pub account_id: Option<Uuid>,
    pub status: String,
    pub kind: String,
    pub subscriber_id: Option<Uuid>,
    pub payor_name: String,
    pub payor_identifier: Option<String>,
    pub policy_number: String,
    pub group_number: Option<String>,
    pub relationship: String,
    pub start_date: chrono::NaiveDate,
    pub end_date: Option<chrono::NaiveDate>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
