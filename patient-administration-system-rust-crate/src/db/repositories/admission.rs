//! admission repository
//!
//! Covers `admissions`, `transfers`, `discharges`, and `bed_assignments`
//! tables. The orchestration of ADT (lock bed, write rows, flip statuses)
//! lives in `src/adt/`; this module just persists the records.

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::{admission, bed_assignment, discharge, encounter, transfer};
use crate::models::admission::{Admission, BedAssignment, Discharge, Transfer};
use crate::{Error, Result};

pub struct AdmissionRepository;

impl AdmissionRepository {
    pub async fn create<C: ConnectionTrait>(conn: &C, a: &Admission) -> Result<Admission> {
        let am = admission_to_active_model(a);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(a.clone())
    }

    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Admission>> {
        let m = admission::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(admission_from_model).transpose()
    }

    /// Find the currently-active bed assignment for an encounter, if any.
    ///
    /// "Active" means `released_at IS NULL`. There can be at most one active
    /// row per encounter (enforced by application logic — the schema's
    /// partial unique index is on `bed_id`, not `encounter_id`).
    pub async fn find_active_by_encounter<C: ConnectionTrait>(
        conn: &C,
        encounter_id: Uuid,
    ) -> Result<Option<BedAssignment>> {
        let m = bed_assignment::Entity::find()
            .filter(bed_assignment::Column::EncounterId.eq(encounter_id))
            .filter(bed_assignment::Column::ReleasedAt.is_null())
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(bed_assignment_from_model).transpose()
    }

    pub async fn create_transfer<C: ConnectionTrait>(conn: &C, t: &Transfer) -> Result<Transfer> {
        let am = transfer_to_active_model(t);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(t.clone())
    }

    pub async fn create_discharge<C: ConnectionTrait>(
        conn: &C,
        d: &Discharge,
    ) -> Result<Discharge> {
        let am = discharge_to_active_model(d);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(d.clone())
    }

    /// Find the currently-open admission for a patient: an admission whose
    /// bed assignment has `released_at IS NULL`. Returns at most one row;
    /// the application invariant is that a patient has at most one open
    /// inpatient admission at any time.
    pub async fn find_open_for_patient<C: ConnectionTrait>(
        conn: &C,
        patient_id: Uuid,
    ) -> Result<Option<Admission>> {
        let encounter_ids: Vec<Uuid> = encounter::Entity::find()
            .filter(encounter::Column::PatientId.eq(patient_id))
            .all(conn)
            .await
            .map_err(Error::Database)?
            .into_iter()
            .map(|e| e.id)
            .collect();
        if encounter_ids.is_empty() {
            return Ok(None);
        }
        // Active bed assignment = no released_at. Take the most recently
        // assigned one if more than one matches (shouldn't happen).
        let active = bed_assignment::Entity::find()
            .filter(bed_assignment::Column::EncounterId.is_in(encounter_ids.clone()))
            .filter(bed_assignment::Column::ReleasedAt.is_null())
            .order_by_desc(bed_assignment::Column::AssignedAt)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        let Some(active) = active else {
            return Ok(None);
        };
        let m = admission::Entity::find()
            .filter(admission::Column::EncounterId.eq(active.encounter_id))
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(admission_from_model).transpose()
    }

    /// All admissions for a given patient, joined via `encounters`. Returns
    /// the admission rows only — callers can hydrate the encounter
    /// separately if needed.
    pub async fn list_by_patient<C: ConnectionTrait>(
        conn: &C,
        patient_id: Uuid,
    ) -> Result<Vec<Admission>> {
        // Two queries: fetch the patient's encounter ids, then admissions.
        let encounter_ids: Vec<Uuid> = encounter::Entity::find()
            .filter(encounter::Column::PatientId.eq(patient_id))
            .all(conn)
            .await
            .map_err(Error::Database)?
            .into_iter()
            .map(|e| e.id)
            .collect();
        if encounter_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = admission::Entity::find()
            .filter(admission::Column::EncounterId.is_in(encounter_ids))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(admission_from_model).collect()
    }

    /// Insert a new bed_assignment row. Separate from `create` because
    /// admissions and bed_assignments are independent in v0.1.
    pub async fn create_bed_assignment<C: ConnectionTrait>(
        conn: &C,
        ba: &BedAssignment,
    ) -> Result<BedAssignment> {
        let am = bed_assignment_to_active_model(ba);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(ba.clone())
    }

    /// Mark the given bed_assignment as released (sets `released_at`).
    pub async fn release_bed_assignment<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<BedAssignment> {
        let m = bed_assignment::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("bed_assignment {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am: bed_assignment::ActiveModel = m.into();
        am.released_at = Set(Some(now));
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        bed_assignment_from_model(updated)
    }

    /// Find the patient's most-recently-discharged admission: walk
    /// `encounters` for the patient, join `discharges`, return the
    /// admission row corresponding to the discharge with the largest
    /// `discharged_at`. Returns `Ok(None)` when the patient has never been
    /// discharged. Used by the HL7 v2 ADT^A13 (cancel discharge) handler
    /// to identify which discharge to undo.
    pub async fn find_most_recently_discharged_for_patient<C: ConnectionTrait>(
        conn: &C,
        patient_id: Uuid,
    ) -> Result<Option<(Admission, Discharge)>> {
        let encounter_ids: Vec<Uuid> = encounter::Entity::find()
            .filter(encounter::Column::PatientId.eq(patient_id))
            .all(conn)
            .await
            .map_err(Error::Database)?
            .into_iter()
            .map(|e| e.id)
            .collect();
        if encounter_ids.is_empty() {
            return Ok(None);
        }
        let admissions = admission::Entity::find()
            .filter(admission::Column::EncounterId.is_in(encounter_ids))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        if admissions.is_empty() {
            return Ok(None);
        }
        let admission_ids: Vec<Uuid> = admissions.iter().map(|a| a.id).collect();
        let latest = discharge::Entity::find()
            .filter(discharge::Column::AdmissionId.is_in(admission_ids))
            .order_by_desc(discharge::Column::DischargedAt)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        let Some(latest) = latest else {
            return Ok(None);
        };
        let adm = admissions
            .into_iter()
            .find(|a| a.id == latest.admission_id)
            .ok_or_else(|| Error::not_found(format!("admission {}", latest.admission_id)))?;
        Ok(Some((
            admission_from_model(adm)?,
            discharge_from_model(latest)?,
        )))
    }

    /// Find the most-recent bed_assignment (active or released) for an
    /// encounter. Used by cancel-discharge to identify which bed to
    /// reinstate. Returns `Ok(None)` when the encounter never had a bed.
    pub async fn find_latest_bed_assignment_for_encounter<C: ConnectionTrait>(
        conn: &C,
        encounter_id: Uuid,
    ) -> Result<Option<BedAssignment>> {
        let m = bed_assignment::Entity::find()
            .filter(bed_assignment::Column::EncounterId.eq(encounter_id))
            .order_by_desc(bed_assignment::Column::AssignedAt)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(bed_assignment_from_model).transpose()
    }

    /// Find the most-recent transfer row for an admission, if any.
    /// Used by ADT^A12 (cancel transfer) to identify which transfer
    /// to undo (the latest one).
    pub async fn find_latest_transfer_for_admission<C: ConnectionTrait>(
        conn: &C,
        admission_id: Uuid,
    ) -> Result<Option<Transfer>> {
        let m = transfer::Entity::find()
            .filter(transfer::Column::AdmissionId.eq(admission_id))
            .order_by_desc(transfer::Column::TransferredAt)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(transfer_from_model).transpose()
    }

    /// Delete a transfer row by id. Returns the number of rows
    /// deleted (0 if none matched). Used by ADT^A12 (cancel
    /// transfer) — like `discharges`, the `transfers` table is
    /// operational, not append-only, so a wrong transfer is
    /// physically removed when cancelled.
    pub async fn delete_transfer<C: ConnectionTrait>(conn: &C, transfer_id: Uuid) -> Result<u64> {
        let res = transfer::Entity::delete_by_id(transfer_id)
            .exec(conn)
            .await
            .map_err(Error::Database)?;
        Ok(res.rows_affected)
    }

    /// Delete the discharge row for the given admission. Returns the number
    /// of rows deleted (0 if there was no discharge). Used by ADT^A13
    /// (cancel discharge) — the `discharges` table is operational, not
    /// append-only, so a row that turns out to have been wrong is
    /// physically removed.
    pub async fn delete_discharge<C: ConnectionTrait>(conn: &C, admission_id: Uuid) -> Result<u64> {
        let res = discharge::Entity::delete_many()
            .filter(discharge::Column::AdmissionId.eq(admission_id))
            .exec(conn)
            .await
            .map_err(Error::Database)?;
        Ok(res.rows_affected)
    }
}

fn discharge_from_model(m: discharge::Model) -> Result<Discharge> {
    Ok(Discharge {
        id: m.id,
        admission_id: m.admission_id,
        discharging_practitioner_id: m.discharging_practitioner_id,
        discharged_at: m.discharged_at.with_timezone(&chrono::Utc),
        disposition: m.disposition,
        notes: m.notes,
        created_at: m.created_at.with_timezone(&chrono::Utc),
    })
}

// --- conversion helpers ---

fn admission_to_active_model(a: &Admission) -> admission::ActiveModel {
    admission::ActiveModel {
        id: Set(a.id),
        encounter_id: Set(a.encounter_id),
        bed_id: Set(a.bed_id),
        admitting_practitioner_id: Set(a.admitting_practitioner_id),
        admitted_at: Set(a.admitted_at.fixed_offset()),
        reason: Set(a.reason.clone()),
        created_at: Set(a.created_at.fixed_offset()),
        updated_at: Set(a.updated_at.fixed_offset()),
    }
}

fn admission_from_model(m: admission::Model) -> Result<Admission> {
    Ok(Admission {
        id: m.id,
        encounter_id: m.encounter_id,
        bed_id: m.bed_id,
        admitting_practitioner_id: m.admitting_practitioner_id,
        admitted_at: m.admitted_at.with_timezone(&chrono::Utc),
        reason: m.reason,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

fn transfer_from_model(m: transfer::Model) -> Result<Transfer> {
    Ok(Transfer {
        id: m.id,
        admission_id: m.admission_id,
        from_bed_id: m.from_bed_id,
        to_bed_id: m.to_bed_id,
        reason: m.reason,
        transferred_at: m.transferred_at.with_timezone(&chrono::Utc),
        created_at: m.created_at.with_timezone(&chrono::Utc),
    })
}

fn transfer_to_active_model(t: &Transfer) -> transfer::ActiveModel {
    transfer::ActiveModel {
        id: Set(t.id),
        admission_id: Set(t.admission_id),
        from_bed_id: Set(t.from_bed_id),
        to_bed_id: Set(t.to_bed_id),
        reason: Set(t.reason.clone()),
        transferred_at: Set(t.transferred_at.fixed_offset()),
        created_at: Set(t.created_at.fixed_offset()),
    }
}

fn discharge_to_active_model(d: &Discharge) -> discharge::ActiveModel {
    discharge::ActiveModel {
        id: Set(d.id),
        admission_id: Set(d.admission_id),
        discharging_practitioner_id: Set(d.discharging_practitioner_id),
        discharged_at: Set(d.discharged_at.fixed_offset()),
        disposition: Set(d.disposition.clone()),
        notes: Set(d.notes.clone()),
        created_at: Set(d.created_at.fixed_offset()),
    }
}

fn bed_assignment_to_active_model(ba: &BedAssignment) -> bed_assignment::ActiveModel {
    bed_assignment::ActiveModel {
        id: Set(ba.id),
        encounter_id: Set(ba.encounter_id),
        bed_id: Set(ba.bed_id),
        assigned_at: Set(ba.assigned_at.fixed_offset()),
        released_at: Set(ba.released_at.map(|t| t.fixed_offset())),
        created_at: Set(ba.created_at.fixed_offset()),
        updated_at: Set(ba.updated_at.fixed_offset()),
    }
}

fn bed_assignment_from_model(m: bed_assignment::Model) -> Result<BedAssignment> {
    Ok(BedAssignment {
        id: m.id,
        encounter_id: m.encounter_id,
        bed_id: m.bed_id,
        assigned_at: m.assigned_at.with_timezone(&chrono::Utc),
        released_at: m.released_at.map(|t| t.with_timezone(&chrono::Utc)),
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admission_roundtrip_via_active_model() {
        let a = Admission::new(Uuid::new_v4(), Uuid::new_v4());
        let am = admission_to_active_model(&a);
        let m = admission::Model {
            id: am.id.clone().unwrap(),
            encounter_id: am.encounter_id.clone().unwrap(),
            bed_id: am.bed_id.clone().unwrap(),
            admitting_practitioner_id: am.admitting_practitioner_id.clone().unwrap(),
            admitted_at: am.admitted_at.clone().unwrap(),
            reason: am.reason.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = admission_from_model(m).expect("from_model");
        assert_eq!(back.id, a.id);
        assert_eq!(back.encounter_id, a.encounter_id);
        assert_eq!(back.bed_id, a.bed_id);
    }
}
