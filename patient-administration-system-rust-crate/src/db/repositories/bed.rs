//! bed repository
//!
//! Owns the `beds` table. Provides `select_for_update` for the ADT service
//! to lock a bed inside an admit/transfer transaction.

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::{bed, room};
use crate::models::facility::{Bed, BedStatus};
use crate::{Error, Result};

pub struct BedRepository;

impl BedRepository {
    pub async fn create<C: ConnectionTrait>(conn: &C, b: &Bed) -> Result<Bed> {
        let am = to_active_model(b);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(b.clone())
    }

    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Bed>> {
        let m = bed::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Find the first bed whose `code` matches. Bed codes are administered
    /// per-facility and typically unique within a facility, but this
    /// repository does not enforce uniqueness across facilities — callers
    /// that need a global lookup should use [`Self::find_by_id`].
    pub async fn find_by_code<C: ConnectionTrait>(conn: &C, code: &str) -> Result<Option<Bed>> {
        let m = bed::Entity::find()
            .filter(bed::Column::Code.eq(code))
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Full replace by id (name + room + code). Status updates stay
    /// on the state-machine-protected `update_status` path; this one
    /// is for MFN^M05 master-file changes that touch the location
    /// metadata but not the operational state. Returns `Error::
    /// NotFound` when the row is missing.
    pub async fn update<C: ConnectionTrait>(conn: &C, b: &Bed) -> Result<Bed> {
        let existing = bed::Entity::find_by_id(b.id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("bed {}", b.id)))?;
        let mut am: bed::ActiveModel = existing.into();
        am.room_id = Set(b.room_id);
        am.name = Set(b.name.clone());
        am.code = Set(b.code.clone());
        am.updated_at = Set(chrono::Utc::now().fixed_offset());
        am.update(conn).await.map_err(Error::Database)?;
        Ok(b.clone())
    }

    /// Validate the bed status transition and persist.
    pub async fn update_status<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        new_status: BedStatus,
    ) -> Result<Bed> {
        let m = bed::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("bed {id}")))?;
        let current = bed_status_from_str(&m.status)?;
        current.try_transition_to(new_status)?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am: bed::ActiveModel = m.into();
        am.status = Set(bed_status_to_str(new_status).to_string());
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
    }

    /// List all beds in a ward. Walks rooms → beds.
    pub async fn list_by_ward<C: ConnectionTrait>(conn: &C, ward_id: Uuid) -> Result<Vec<Bed>> {
        let room_ids: Vec<Uuid> = room::Entity::find()
            .filter(room::Column::WardId.eq(ward_id))
            .all(conn)
            .await
            .map_err(Error::Database)?
            .into_iter()
            .map(|r| r.id)
            .collect();
        if room_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = bed::Entity::find()
            .filter(bed::Column::RoomId.is_in(room_ids))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// `SELECT … FOR UPDATE` on a single bed row. Use inside a
    /// `DatabaseTransaction` so the lock is held until commit.
    pub async fn select_for_update<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Bed>> {
        let m = bed::Entity::find_by_id(id)
            .lock_exclusive()
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Force the bed to a target status, bypassing the [`BedStatus`]
    /// state machine. **Use only for cancellation flows** (HL7 v2 ADT^A11,
    /// ADT^A13) where the regular `Occupied → Cleaning → Available` /
    /// `Cleaning → Available` lifecycle does not apply because the event
    /// being cancelled never properly happened. Every other write path must
    /// go through [`Self::update_status`].
    pub async fn set_status_unchecked<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        new_status: BedStatus,
    ) -> Result<Bed> {
        let m = bed::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("bed {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am: bed::ActiveModel = m.into();
        am.status = Set(bed_status_to_str(new_status).to_string());
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
    }
}

// --- conversion helpers ---

pub(crate) fn bed_status_to_str(s: BedStatus) -> &'static str {
    match s {
        BedStatus::Available => "available",
        BedStatus::Occupied => "occupied",
        BedStatus::Reserved => "reserved",
        BedStatus::OutOfService => "out_of_service",
        BedStatus::Cleaning => "cleaning",
    }
}

pub(crate) fn bed_status_from_str(s: &str) -> Result<BedStatus> {
    match s {
        "available" => Ok(BedStatus::Available),
        "occupied" => Ok(BedStatus::Occupied),
        "reserved" => Ok(BedStatus::Reserved),
        "out_of_service" => Ok(BedStatus::OutOfService),
        "cleaning" => Ok(BedStatus::Cleaning),
        other => Err(Error::internal(format!("unknown bed status: {other}"))),
    }
}

fn to_active_model(b: &Bed) -> bed::ActiveModel {
    bed::ActiveModel {
        id: Set(b.id),
        room_id: Set(b.room_id),
        name: Set(b.name.clone()),
        code: Set(b.code.clone()),
        status: Set(bed_status_to_str(b.status).to_string()),
        created_at: Set(b.created_at.fixed_offset()),
        updated_at: Set(b.updated_at.fixed_offset()),
    }
}

fn from_model(m: bed::Model) -> Result<Bed> {
    Ok(Bed {
        id: m.id,
        room_id: m.room_id,
        name: m.name,
        code: m.code,
        status: bed_status_from_str(&m.status)?,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bed_roundtrip_via_active_model() {
        let b = Bed::new(Uuid::new_v4(), "Bed 1".into(), "B1".into());
        let am = to_active_model(&b);
        let m = bed::Model {
            id: am.id.clone().unwrap(),
            room_id: am.room_id.clone().unwrap(),
            name: am.name.clone().unwrap(),
            code: am.code.clone().unwrap(),
            status: am.status.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, b.id);
        assert_eq!(back.status, BedStatus::Available);
        assert_eq!(back.name, "Bed 1");
    }
}
