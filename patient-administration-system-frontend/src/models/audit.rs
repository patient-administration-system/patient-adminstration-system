//! `audit_log` table — read-only.

use sea_orm::{DatabaseConnection, DbErr, EntityTrait, QueryOrder, QuerySelect};

mod entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "audit_log")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub entity_type: String,
        pub entity_id: Uuid,
        pub action: String,
        pub old_value: Option<serde_json::Value>,
        pub new_value: Option<serde_json::Value>,
        pub user_id: Option<String>,
        pub user_ip: Option<String>,
        pub user_agent: Option<String>,
        pub at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[derive(Debug)]
pub struct AuditSummary {
    pub entity_type: String,
    pub action: String,
    pub user_id: Option<String>,
    pub at: chrono::DateTime<chrono::FixedOffset>,
}

pub async fn list_recent_audit(
    db: &DatabaseConnection,
    limit: u64,
) -> Result<Vec<AuditSummary>, DbErr> {
    let rows = entity::Entity::find()
        .order_by_desc(entity::Column::At)
        .limit(limit)
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|m| AuditSummary {
            entity_type: m.entity_type,
            action: m.action,
            user_id: m.user_id,
            at: m.at,
        })
        .collect())
}
