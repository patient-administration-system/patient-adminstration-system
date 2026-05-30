//! `beds` table — read-only.

use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};
use uuid::Uuid;

mod room_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "rooms")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub ward_id: Uuid,
        pub name: String,
        pub code: String,
        pub active: bool,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod bed_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "beds")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub room_id: Uuid,
        pub name: String,
        pub code: String,
        pub status: String,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[derive(Debug, serde::Serialize)]
pub struct BedSummary {
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BedRow {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub status: String,
    pub room_code: String,
}

/// Walk rooms → beds for a ward and return the bed status strings only.
/// Used by the dashboard's ward-occupancy aggregate.
pub async fn list_by_ward(
    db: &DatabaseConnection,
    ward_id: Uuid,
) -> Result<Vec<BedSummary>, DbErr> {
    let room_ids: Vec<Uuid> = room_entity::Entity::find()
        .filter(room_entity::Column::WardId.eq(ward_id))
        .all(db)
        .await?
        .into_iter()
        .map(|r| r.id)
        .collect();
    if room_ids.is_empty() {
        return Ok(Vec::new());
    }
    let beds = bed_entity::Entity::find()
        .filter(bed_entity::Column::RoomId.is_in(room_ids))
        .all(db)
        .await?;
    Ok(beds
        .into_iter()
        .map(|b| BedSummary { status: b.status })
        .collect())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AvailableBed {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub room_code: String,
    pub ward_name: String,
    pub ward_code: String,
}

/// Every bed currently in `status = 'available'`, joined to its room +
/// ward for human-readable picker display. Sorted by ward + room + bed
/// code so adjacent beds group together visually.
pub async fn list_available(db: &DatabaseConnection) -> Result<Vec<AvailableBed>, DbErr> {
    let beds = bed_entity::Entity::find()
        .filter(bed_entity::Column::Status.eq("available"))
        .all(db)
        .await?;
    if beds.is_empty() {
        return Ok(Vec::new());
    }
    let room_ids: Vec<Uuid> = beds.iter().map(|b| b.room_id).collect();
    let rooms = room_entity::Entity::find()
        .filter(room_entity::Column::Id.is_in(room_ids))
        .all(db)
        .await?;
    let room_by_id: std::collections::HashMap<Uuid, room_entity::Model> =
        rooms.into_iter().map(|r| (r.id, r)).collect();

    let ward_ids: Vec<Uuid> = room_by_id.values().map(|r| r.ward_id).collect();
    let wards = crate::models::ward::find_wards_by_ids(db, &ward_ids).await?;
    let ward_by_id: std::collections::HashMap<Uuid, crate::models::ward::WardSummary> =
        wards.into_iter().map(|w| (w.id, w)).collect();

    let mut out: Vec<AvailableBed> = beds
        .into_iter()
        .filter_map(|b| {
            let room = room_by_id.get(&b.room_id)?;
            let ward = ward_by_id.get(&room.ward_id)?;
            Some(AvailableBed {
                id: b.id,
                name: b.name,
                code: b.code,
                room_code: room.code.clone(),
                ward_name: ward.name.clone(),
                ward_code: ward.code.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        a.ward_code
            .cmp(&b.ward_code)
            .then_with(|| a.room_code.cmp(&b.room_code))
            .then_with(|| a.code.cmp(&b.code))
    });
    Ok(out)
}

/// Rich projection used by the ward detail page: every bed in a ward
/// with its room code, sorted by `(room_code, bed_code)` for a stable
/// display order.
pub async fn list_by_ward_detailed(
    db: &DatabaseConnection,
    ward_id: Uuid,
) -> Result<Vec<BedRow>, DbErr> {
    let rooms = room_entity::Entity::find()
        .filter(room_entity::Column::WardId.eq(ward_id))
        .all(db)
        .await?;
    if rooms.is_empty() {
        return Ok(Vec::new());
    }
    let room_code_by_id: std::collections::HashMap<Uuid, String> =
        rooms.iter().map(|r| (r.id, r.code.clone())).collect();
    let room_ids: Vec<Uuid> = rooms.into_iter().map(|r| r.id).collect();
    let beds = bed_entity::Entity::find()
        .filter(bed_entity::Column::RoomId.is_in(room_ids))
        .all(db)
        .await?;
    let mut rows: Vec<BedRow> = beds
        .into_iter()
        .map(|b| BedRow {
            id: b.id,
            name: b.name,
            code: b.code,
            status: b.status,
            room_code: room_code_by_id.get(&b.room_id).cloned().unwrap_or_default(),
        })
        .collect();
    rows.sort_by(|a, b| {
        a.room_code
            .cmp(&b.room_code)
            .then_with(|| a.code.cmp(&b.code))
    });
    Ok(rows)
}
