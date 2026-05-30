//! `letter_templates` table — read-only summary projection for the
//! `/letters/new` composer form.

use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};
use serde::Serialize;
use uuid::Uuid;

mod entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "letter_templates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub name: String,
        pub subject: String,
        pub body_tera: String,
        pub required_variables: serde_json::Value,
        pub channels: serde_json::Value,
        pub active: bool,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[derive(Debug, Clone, Serialize)]
pub struct TemplateRow {
    pub id: Uuid,
    pub name: String,
    pub subject: String,
    pub channels: Vec<String>,
}

pub async fn list_active_templates(db: &DatabaseConnection) -> Result<Vec<TemplateRow>, DbErr> {
    let rows = entity::Entity::find()
        .filter(entity::Column::Active.eq(true))
        .all(db)
        .await?;
    let mut out: Vec<TemplateRow> = rows
        .into_iter()
        .map(|m| TemplateRow {
            id: m.id,
            name: m.name,
            subject: m.subject,
            channels: m
                .channels
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
