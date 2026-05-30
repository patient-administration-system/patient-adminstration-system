//! encounter repository

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::encounter;
use crate::models::encounter::{Encounter, EncounterClass, EncounterStatus};
use crate::{Error, Result};

pub struct EncounterRepository;

impl EncounterRepository {
    pub async fn create<C: ConnectionTrait>(conn: &C, e: &Encounter) -> Result<Encounter> {
        let am = to_active_model(e);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(e.clone())
    }

    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Encounter>> {
        let m = encounter::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    pub async fn update<C: ConnectionTrait>(conn: &C, e: &Encounter) -> Result<Encounter> {
        let mut am = to_active_model(e);
        am.created_at = NotSet;
        am.update(conn).await.map_err(Error::Database)?;
        Ok(e.clone())
    }

    /// Find the most-recent **active ambulatory** encounter for a
    /// patient — i.e. `class IN ('outpatient', 'emergency')` and
    /// `status IN ('arrived', 'in_progress')`. Used by ADT^A06
    /// (change outpatient to inpatient) to locate the existing
    /// encounter that should be reclassified + bedded.
    pub async fn find_latest_active_ambulatory_for_patient<C: ConnectionTrait>(
        conn: &C,
        patient_id: Uuid,
    ) -> Result<Option<Encounter>> {
        let m = encounter::Entity::find()
            .filter(encounter::Column::PatientId.eq(patient_id))
            .filter(encounter::Column::DeletedAt.is_null())
            .filter(
                encounter::Column::Class
                    .eq("outpatient")
                    .or(encounter::Column::Class.eq("emergency")),
            )
            .filter(
                encounter::Column::Status
                    .eq("arrived")
                    .or(encounter::Column::Status.eq("in_progress")),
            )
            .order_by_desc(encounter::Column::CreatedAt)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Find the most-recent **planned inpatient** encounter for a patient,
    /// if any. Used by ADT^A38 cancel-pre-admit to locate the encounter
    /// that should be cancelled when a pre-admission is undone.
    pub async fn find_latest_planned_inpatient_for_patient<C: ConnectionTrait>(
        conn: &C,
        patient_id: Uuid,
    ) -> Result<Option<Encounter>> {
        let m = encounter::Entity::find()
            .filter(encounter::Column::PatientId.eq(patient_id))
            .filter(encounter::Column::DeletedAt.is_null())
            .filter(encounter::Column::Status.eq("planned"))
            .filter(encounter::Column::Class.eq("inpatient"))
            .order_by_desc(encounter::Column::CreatedAt)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    pub async fn list_by_patient<C: ConnectionTrait>(
        conn: &C,
        patient_id: Uuid,
    ) -> Result<Vec<Encounter>> {
        let rows = encounter::Entity::find()
            .filter(encounter::Column::PatientId.eq(patient_id))
            .filter(encounter::Column::DeletedAt.is_null())
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Validate `current -> new_status` against the `EncounterStatus` state
    /// machine, then persist. Returns the updated `Encounter`.
    pub async fn set_status<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        new_status: EncounterStatus,
    ) -> Result<Encounter> {
        let m = encounter::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("encounter {id}")))?;
        let current = encounter_status_from_str(&m.status)?;
        current.try_transition_to(new_status)?;
        let mut am: encounter::ActiveModel = m.into();
        am.status = Set(encounter_status_to_str(new_status).to_string());
        am.updated_at = Set(chrono::Utc::now().fixed_offset());
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
    }

    /// Reclassify an encounter (e.g. Outpatient → Inpatient for
    /// ADT^A06, or Inpatient → Outpatient for ADT^A07). The
    /// EncounterClass enum has no state-machine constraint — any
    /// class change is administratively legal — so this is a plain
    /// field update.
    pub async fn set_class<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        new_class: EncounterClass,
    ) -> Result<Encounter> {
        let m = encounter::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("encounter {id}")))?;
        let mut am: encounter::ActiveModel = m.into();
        am.class = Set(encounter_class_to_str(new_class).to_string());
        am.updated_at = Set(chrono::Utc::now().fixed_offset());
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
    }

    /// Force the encounter to a target status, bypassing the
    /// [`EncounterStatus`] state machine. **Use only for cancellation
    /// flows** (HL7 v2 ADT^A13 cancel-discharge), which by construction
    /// need to move out of the otherwise-terminal `Finished` state. Every
    /// other write path must go through [`Self::set_status`].
    pub async fn set_status_unchecked<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        new_status: EncounterStatus,
    ) -> Result<Encounter> {
        let m = encounter::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("encounter {id}")))?;
        let mut am: encounter::ActiveModel = m.into();
        am.status = Set(encounter_status_to_str(new_status).to_string());
        am.updated_at = Set(chrono::Utc::now().fixed_offset());
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
    }
}

// --- conversion helpers ---

pub(crate) fn encounter_class_to_str(c: EncounterClass) -> &'static str {
    match c {
        EncounterClass::Outpatient => "outpatient",
        EncounterClass::Inpatient => "inpatient",
        EncounterClass::Emergency => "emergency",
        EncounterClass::DayCase => "day_case",
        EncounterClass::HomeCare => "home_care",
        EncounterClass::Virtual => "virtual",
    }
}

pub(crate) fn encounter_class_from_str(s: &str) -> Result<EncounterClass> {
    match s {
        "outpatient" => Ok(EncounterClass::Outpatient),
        "inpatient" => Ok(EncounterClass::Inpatient),
        "emergency" => Ok(EncounterClass::Emergency),
        "day_case" => Ok(EncounterClass::DayCase),
        "home_care" => Ok(EncounterClass::HomeCare),
        "virtual" => Ok(EncounterClass::Virtual),
        other => Err(Error::internal(format!("unknown encounter class: {other}"))),
    }
}

pub(crate) fn encounter_status_to_str(s: EncounterStatus) -> &'static str {
    match s {
        EncounterStatus::Planned => "planned",
        EncounterStatus::Arrived => "arrived",
        EncounterStatus::InProgress => "in_progress",
        EncounterStatus::OnLeave => "on_leave",
        EncounterStatus::Finished => "finished",
        EncounterStatus::Cancelled => "cancelled",
    }
}

pub(crate) fn encounter_status_from_str(s: &str) -> Result<EncounterStatus> {
    match s {
        "planned" => Ok(EncounterStatus::Planned),
        "arrived" => Ok(EncounterStatus::Arrived),
        "in_progress" => Ok(EncounterStatus::InProgress),
        "on_leave" => Ok(EncounterStatus::OnLeave),
        "finished" => Ok(EncounterStatus::Finished),
        "cancelled" => Ok(EncounterStatus::Cancelled),
        other => Err(Error::internal(format!(
            "unknown encounter status: {other}"
        ))),
    }
}

fn to_active_model(e: &Encounter) -> encounter::ActiveModel {
    encounter::ActiveModel {
        id: Set(e.id),
        patient_id: Set(e.patient_id),
        class: Set(encounter_class_to_str(e.class).to_string()),
        status: Set(encounter_status_to_str(e.status).to_string()),
        period_start: Set(e.period_start.fixed_offset()),
        period_end: Set(e.period_end.map(|t| t.fixed_offset())),
        practitioner_id: Set(e.practitioner_id),
        department_id: Set(e.department_id),
        reason: Set(e.reason.clone()),
        deleted_at: Set(None),
        created_at: Set(e.created_at.fixed_offset()),
        updated_at: Set(e.updated_at.fixed_offset()),
    }
}

fn from_model(m: encounter::Model) -> Result<Encounter> {
    Ok(Encounter {
        id: m.id,
        patient_id: m.patient_id,
        class: encounter_class_from_str(&m.class)?,
        status: encounter_status_from_str(&m.status)?,
        period_start: m.period_start.with_timezone(&chrono::Utc),
        period_end: m.period_end.map(|t| t.with_timezone(&chrono::Utc)),
        practitioner_id: m.practitioner_id,
        department_id: m.department_id,
        reason: m.reason,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encounter_roundtrip_via_active_model() {
        let e = Encounter::new(Uuid::new_v4(), EncounterClass::Inpatient);
        let am = to_active_model(&e);
        let m = encounter::Model {
            id: am.id.clone().unwrap(),
            patient_id: am.patient_id.clone().unwrap(),
            class: am.class.clone().unwrap(),
            status: am.status.clone().unwrap(),
            period_start: am.period_start.clone().unwrap(),
            period_end: am.period_end.clone().unwrap(),
            practitioner_id: am.practitioner_id.clone().unwrap(),
            department_id: am.department_id.clone().unwrap(),
            reason: am.reason.clone().unwrap(),
            deleted_at: am.deleted_at.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, e.id);
        assert_eq!(back.class, EncounterClass::Inpatient);
        assert_eq!(back.status, EncounterStatus::Planned);
    }
}
