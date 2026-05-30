//! SeaORM entity for the `payments` table.
//!
//! Note: `amount_value` is stored as `String` at the DB layer. Conversion
//! to/from `crate::models::Money` happens in repositories.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "payments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub amount_value: String,
    pub amount_currency: String,
    pub method: String,
    pub reference: Option<String>,
    pub posted_at: DateTimeWithTimeZone,
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
