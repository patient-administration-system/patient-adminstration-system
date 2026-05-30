//! schedule repository

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::schedule;
use crate::models::schedule::{Schedule, ScheduleOwner};
use crate::{Error, Result};

pub struct ScheduleRepository;

/// String constants used for the polymorphic `owner_kind` column.
pub(crate) mod owner_kind {
    pub const PRACTITIONER: &str = "practitioner";
    pub const BED: &str = "bed";
    pub const ROOM: &str = "room";
}

impl ScheduleRepository {
    pub async fn create<C: ConnectionTrait>(conn: &C, s: &Schedule) -> Result<Schedule> {
        let am = to_active_model(s);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(s.clone())
    }

    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Schedule>> {
        let m = schedule::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// List schedules whose `(owner_kind, owner_id)` matches the given owner.
    pub async fn list_by_owner<C: ConnectionTrait>(
        conn: &C,
        owner_kind: &str,
        owner_id: Uuid,
    ) -> Result<Vec<Schedule>> {
        let rows = schedule::Entity::find()
            .filter(schedule::Column::OwnerKind.eq(owner_kind))
            .filter(schedule::Column::OwnerId.eq(owner_id))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Full replace by id. Returns `Error::NotFound` when the row is
    /// missing. Used by the FHIR R5 `PUT /fhir/Schedule/{id}`
    /// surface (v0.21).
    pub async fn update<C: ConnectionTrait>(conn: &C, s: &Schedule) -> Result<Schedule> {
        let existing = schedule::Entity::find_by_id(s.id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("schedule {}", s.id)))?;
        let (kind, owner_id) = owner_to_kind_and_id(&s.owner);
        let mut am: schedule::ActiveModel = existing.into();
        am.owner_kind = Set(kind.to_string());
        am.owner_id = Set(owner_id);
        am.service_type = Set(s.service_type.clone());
        am.active = Set(s.active);
        am.updated_at = Set(chrono::Utc::now().fixed_offset());
        am.update(conn).await.map_err(Error::Database)?;
        Ok(s.clone())
    }

    /// Hard delete by id. Schedules do **not** carry a `deleted_at`
    /// column — invariant §5.3 restricts soft delete to patients,
    /// encounters, and appointments. Returns `Error::NotFound` when
    /// the row is missing. Used by `DELETE /fhir/Schedule/{id}`
    /// (v0.21).
    pub async fn delete<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<()> {
        let r = schedule::Entity::delete_by_id(id)
            .exec(conn)
            .await
            .map_err(Error::Database)?;
        if r.rows_affected == 0 {
            return Err(Error::not_found(format!("schedule {id}")));
        }
        Ok(())
    }
}

// --- conversion helpers ---

fn owner_to_kind_and_id(owner: &ScheduleOwner) -> (&'static str, Uuid) {
    match owner {
        ScheduleOwner::Practitioner(id) => (owner_kind::PRACTITIONER, *id),
        ScheduleOwner::Bed(id) => (owner_kind::BED, *id),
        ScheduleOwner::Room(id) => (owner_kind::ROOM, *id),
    }
}

fn owner_from_kind_and_id(kind: &str, id: Uuid) -> Result<ScheduleOwner> {
    match kind {
        owner_kind::PRACTITIONER => Ok(ScheduleOwner::Practitioner(id)),
        owner_kind::BED => Ok(ScheduleOwner::Bed(id)),
        owner_kind::ROOM => Ok(ScheduleOwner::Room(id)),
        other => Err(Error::internal(format!(
            "unknown schedule owner kind: {other}"
        ))),
    }
}

fn to_active_model(s: &Schedule) -> schedule::ActiveModel {
    let (kind, owner_id) = owner_to_kind_and_id(&s.owner);
    schedule::ActiveModel {
        id: Set(s.id),
        owner_kind: Set(kind.to_string()),
        owner_id: Set(owner_id),
        service_type: Set(s.service_type.clone()),
        active: Set(s.active),
        created_at: Set(s.created_at.fixed_offset()),
        updated_at: Set(s.updated_at.fixed_offset()),
    }
}

fn from_model(m: schedule::Model) -> Result<Schedule> {
    Ok(Schedule {
        id: m.id,
        owner: owner_from_kind_and_id(&m.owner_kind, m.owner_id)?,
        service_type: m.service_type,
        active: m.active,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_roundtrip_via_active_model() {
        let s = Schedule::new(
            ScheduleOwner::Practitioner(Uuid::new_v4()),
            "general".into(),
        );
        let am = to_active_model(&s);
        let m = schedule::Model {
            id: am.id.clone().unwrap(),
            owner_kind: am.owner_kind.clone().unwrap(),
            owner_id: am.owner_id.clone().unwrap(),
            service_type: am.service_type.clone().unwrap(),
            active: am.active.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, s.id);
        assert_eq!(back.owner, s.owner);
        assert_eq!(back.service_type, "general");
        assert!(back.active);
    }
}
