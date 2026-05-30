//! `schedules` + `slots` tables — read-only for the appointment booker.

use chrono::{DateTime, Duration, FixedOffset, Utc};
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use uuid::Uuid;

mod schedule_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "schedules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub owner_kind: String,
        pub owner_id: Uuid,
        pub service_type: String,
        pub active: bool,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod slot_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "slots")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub schedule_id: Uuid,
        pub start_datetime: chrono::DateTime<chrono::FixedOffset>,
        pub end_datetime: chrono::DateTime<chrono::FixedOffset>,
        pub status: String,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduleRow {
    pub id: Uuid,
    pub service_type: String,
}

pub async fn list_active_schedules(db: &DatabaseConnection) -> Result<Vec<ScheduleRow>, DbErr> {
    let rows = schedule_entity::Entity::find()
        .filter(schedule_entity::Column::Active.eq(true))
        .order_by_asc(schedule_entity::Column::ServiceType)
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|m| ScheduleRow {
            id: m.id,
            service_type: m.service_type,
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct FreeSlotRow {
    pub id: Uuid,
    pub schedule_id: Uuid,
    /// ISO 8601 with no fractional seconds — easy to display + parse.
    pub start_iso: String,
    pub end_iso: String,
    /// Human-friendly label for the picker, e.g. `"Mon 2026-05-25 09:00 → 09:30"`.
    pub label: String,
}

/// Free slots for a given schedule starting between `now` and `now + days`.
pub async fn list_free_slots(
    db: &DatabaseConnection,
    schedule_id: Uuid,
    days: i64,
) -> Result<Vec<FreeSlotRow>, DbErr> {
    let now: DateTime<FixedOffset> = Utc::now().with_timezone(&FixedOffset::east_opt(0).unwrap());
    let horizon = now + Duration::days(days);
    let rows = slot_entity::Entity::find()
        .filter(slot_entity::Column::ScheduleId.eq(schedule_id))
        .filter(slot_entity::Column::Status.eq("free"))
        .filter(slot_entity::Column::StartDatetime.gte(now))
        .filter(slot_entity::Column::StartDatetime.lte(horizon))
        .order_by_asc(slot_entity::Column::StartDatetime)
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|m| FreeSlotRow {
            id: m.id,
            schedule_id: m.schedule_id,
            label: format!(
                "{} {} → {}",
                m.start_datetime.format("%a %Y-%m-%d"),
                m.start_datetime.format("%H:%M"),
                m.end_datetime.format("%H:%M"),
            ),
            start_iso: m.start_datetime.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            end_iso: m.end_datetime.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        })
        .collect())
}
