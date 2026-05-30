//! SeaORM entity for the `charges` table.
//!
//! Note: `amount_value` is stored as `String` at the DB layer. Conversion
//! to/from `crate::models::Money` happens in repositories.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "charges")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub account_id: Uuid,
    pub encounter_id: Option<Uuid>,
    pub appointment_id: Option<Uuid>,
    pub code: String,
    pub description: String,
    pub amount_value: String,
    pub amount_currency: String,
    pub posted_at: DateTimeWithTimeZone,
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
