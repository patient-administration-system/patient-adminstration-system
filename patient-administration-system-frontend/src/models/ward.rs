//! `wards` table — read-only.

use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};
use serde::Serialize;
use uuid::Uuid;

mod entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "wards")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub facility_id: Uuid,
        pub name: String,
        pub code: String,
        pub capacity: i32,
        pub active: bool,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[derive(Serialize, Debug, Clone)]
pub struct WardSummary {
    pub id: Uuid,
    pub name: String,
    pub code: String,
}

pub async fn list_active_wards(db: &DatabaseConnection) -> Result<Vec<WardSummary>, DbErr> {
    let rows = entity::Entity::find()
        .filter(entity::Column::Active.eq(true))
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|m| WardSummary {
            id: m.id,
            name: m.name,
            code: m.code,
        })
        .collect())
}

/// Batch-load wards by id. Used to attach ward names to bed-picker rows.
pub async fn find_wards_by_ids(
    db: &DatabaseConnection,
    ids: &[Uuid],
) -> Result<Vec<WardSummary>, DbErr> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = entity::Entity::find()
        .filter(entity::Column::Id.is_in(ids.to_vec()))
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|m| WardSummary {
            id: m.id,
            name: m.name,
            code: m.code,
        })
        .collect())
}

/// Load one ward by its primary-key id.
pub async fn find_ward_by_id(
    db: &DatabaseConnection,
    id: Uuid,
) -> Result<Option<WardSummary>, DbErr> {
    Ok(entity::Entity::find_by_id(id)
        .one(db)
        .await?
        .map(|m| WardSummary {
            id: m.id,
            name: m.name,
            code: m.code,
        }))
}
