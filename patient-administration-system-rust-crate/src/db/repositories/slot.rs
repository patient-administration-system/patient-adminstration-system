//! slot repository

use chrono::{DateTime, Utc};
use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::slot;
use crate::models::schedule::{Slot, SlotStatus};
use crate::{Error, Result};

pub struct SlotRepository;

impl SlotRepository {
    pub async fn create<C: ConnectionTrait>(conn: &C, s: &Slot) -> Result<Slot> {
        let am = to_active_model(s);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(s.clone())
    }

    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Slot>> {
        let m = slot::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Free slots in a schedule that fall fully within `[start, end)`.
    ///
    /// "Fully within" means `start_datetime >= start AND end_datetime <= end`.
    pub async fn find_free_in_range<C: ConnectionTrait>(
        conn: &C,
        schedule_id: Uuid,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Slot>> {
        let rows = slot::Entity::find()
            .filter(slot::Column::ScheduleId.eq(schedule_id))
            .filter(slot::Column::Status.eq(slot_status_to_str(SlotStatus::Free)))
            .filter(slot::Column::StartDatetime.gte(start.fixed_offset()))
            .filter(slot::Column::EndDatetime.lte(end.fixed_offset()))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Validate the slot status transition and persist.
    pub async fn update_status<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        new_status: SlotStatus,
    ) -> Result<Slot> {
        let m = slot::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("slot {id}")))?;
        let current = slot_status_from_str(&m.status)?;
        current.try_transition_to(new_status)?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am: slot::ActiveModel = m.into();
        am.status = Set(slot_status_to_str(new_status).to_string());
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
    }

    /// `SELECT … FOR UPDATE` for a single slot. Use inside a transaction.
    pub async fn select_for_update<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Slot>> {
        let m = slot::Entity::find_by_id(id)
            .lock_exclusive()
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Full replace by id. Used by `PUT /fhir/Slot/{id}` (v0.21). The
    /// `status` column is the only thing protected by the state-
    /// machine validation in `update_status`; full replace skips that
    /// validation, so callers should treat it as an operator-driven
    /// override (e.g. correcting a row that was created with the
    /// wrong start time).
    pub async fn update<C: ConnectionTrait>(conn: &C, s: &Slot) -> Result<Slot> {
        let existing = slot::Entity::find_by_id(s.id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("slot {}", s.id)))?;
        let mut am: slot::ActiveModel = existing.into();
        am.schedule_id = Set(s.schedule_id);
        am.start_datetime = Set(s.start_datetime.fixed_offset());
        am.end_datetime = Set(s.end_datetime.fixed_offset());
        am.status = Set(slot_status_to_str(s.status).to_string());
        am.updated_at = Set(chrono::Utc::now().fixed_offset());
        am.update(conn).await.map_err(Error::Database)?;
        Ok(s.clone())
    }

    /// Hard delete by id. Slots are not soft-deleted (invariant §5.3).
    /// Used by `DELETE /fhir/Slot/{id}` (v0.21). Returns
    /// `Error::NotFound` when the row is missing.
    pub async fn delete<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<()> {
        let r = slot::Entity::delete_by_id(id)
            .exec(conn)
            .await
            .map_err(Error::Database)?;
        if r.rows_affected == 0 {
            return Err(Error::not_found(format!("slot {id}")));
        }
        Ok(())
    }
}

// --- conversion helpers ---

pub(crate) fn slot_status_to_str(s: SlotStatus) -> &'static str {
    match s {
        SlotStatus::Free => "free",
        SlotStatus::Busy => "busy",
        SlotStatus::BlockedOut => "blocked_out",
    }
}

pub(crate) fn slot_status_from_str(s: &str) -> Result<SlotStatus> {
    match s {
        "free" => Ok(SlotStatus::Free),
        "busy" => Ok(SlotStatus::Busy),
        "blocked_out" => Ok(SlotStatus::BlockedOut),
        other => Err(Error::internal(format!("unknown slot status: {other}"))),
    }
}

fn to_active_model(s: &Slot) -> slot::ActiveModel {
    slot::ActiveModel {
        id: Set(s.id),
        schedule_id: Set(s.schedule_id),
        start_datetime: Set(s.start_datetime.fixed_offset()),
        end_datetime: Set(s.end_datetime.fixed_offset()),
        status: Set(slot_status_to_str(s.status).to_string()),
        created_at: Set(s.created_at.fixed_offset()),
        updated_at: Set(s.updated_at.fixed_offset()),
    }
}

fn from_model(m: slot::Model) -> Result<Slot> {
    Ok(Slot {
        id: m.id,
        schedule_id: m.schedule_id,
        start_datetime: m.start_datetime.with_timezone(&chrono::Utc),
        end_datetime: m.end_datetime.with_timezone(&chrono::Utc),
        status: slot_status_from_str(&m.status)?,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_slot_roundtrip_via_active_model() {
        let start = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap();
        let s = Slot::new(Uuid::new_v4(), start, end);
        let am = to_active_model(&s);
        let m = slot::Model {
            id: am.id.clone().unwrap(),
            schedule_id: am.schedule_id.clone().unwrap(),
            start_datetime: am.start_datetime.clone().unwrap(),
            end_datetime: am.end_datetime.clone().unwrap(),
            status: am.status.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, s.id);
        assert_eq!(back.status, SlotStatus::Free);
        assert_eq!(back.start_datetime, start);
        assert_eq!(back.end_datetime, end);
    }
}
