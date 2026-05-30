//! `outbox_events` table — read-only count for the dashboard.

use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, PaginatorTrait, QueryFilter};

mod entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "outbox_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub event_type: String,
        pub payload: serde_json::Value,
        pub published: bool,
        pub at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub async fn count_unpublished(db: &DatabaseConnection) -> Result<usize, DbErr> {
    let n = entity::Entity::find()
        .filter(entity::Column::Published.eq(false))
        .count(db)
        .await?;
    Ok(n as usize)
}
