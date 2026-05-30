//! patient repository

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::patient;
use crate::models::Gender;
use crate::models::patient::Patient;
use crate::{Error, Result};

pub struct PatientRepository;

impl PatientRepository {
    /// Insert a new patient row.
    pub async fn create<C: ConnectionTrait>(conn: &C, p: &Patient) -> Result<Patient> {
        let am = to_active_model(p)?;
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(p.clone())
    }

    /// Load a patient by primary key.
    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Patient>> {
        let m = patient::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Update an existing patient row (full replace).
    pub async fn update<C: ConnectionTrait>(conn: &C, p: &Patient) -> Result<Patient> {
        let am = to_active_model_for_update(p)?;
        am.update(conn).await.map_err(Error::Database)?;
        Ok(p.clone())
    }

    /// Mark a patient as deleted (soft delete via `deleted_at`).
    pub async fn soft_delete<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<()> {
        let m = patient::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("patient {id}")))?;
        let mut am: patient::ActiveModel = m.into();
        am.deleted_at = Set(Some(chrono::Utc::now().fixed_offset()));
        am.updated_at = Set(chrono::Utc::now().fixed_offset());
        am.update(conn).await.map_err(Error::Database)?;
        Ok(())
    }

    /// Page through non-deleted, non-tombstoned patients. Tombstones
    /// (merge survivors point at; see v0.11) are excluded — clients
    /// who specifically want them call [`Self::list_replaces_for`].
    pub async fn list_active<C: ConnectionTrait>(conn: &C, limit: u64) -> Result<Vec<Patient>> {
        let rows = patient::Entity::find()
            .filter(patient::Column::DeletedAt.is_null())
            .filter(patient::Column::ReplacedBy.is_null())
            .limit(limit)
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Mark `id` as merged into `target_id`: set `replaced_by = target_id`
    /// **and** flip `active = false` so the row is gone from default
    /// listings. Used by `POST /api/patients/{id}/merge-into/{target_id}`
    /// inside one DB transaction. Idempotent against re-merging into the
    /// same target (returns the existing tombstone).
    pub async fn set_replaced_by<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        target_id: Uuid,
    ) -> Result<Patient> {
        let m = patient::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("patient {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am: patient::ActiveModel = m.into();
        am.replaced_by = Set(Some(target_id));
        am.active = Set(false);
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
    }

    /// Inverse direction of the merge link: every patient row whose
    /// `replaced_by` matches `target_id`. Used by
    /// `GET /api/patients/{id}/replaces` to render the merge history of
    /// a survivor. Returns tombstones newest-first.
    pub async fn list_replaces_for<C: ConnectionTrait>(
        conn: &C,
        target_id: Uuid,
    ) -> Result<Vec<Patient>> {
        let rows = patient::Entity::find()
            .filter(patient::Column::ReplacedBy.eq(target_id))
            .order_by_desc(patient::Column::UpdatedAt)
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Look up the (first) non-deleted patient whose `identifiers` JSONB
    /// array contains an entry with the given `value`. Uses the Postgres
    /// JSONB containment operator (`@>`). Caller should still verify the
    /// identifier type (MRN/SSN/etc.) on the returned patient — the
    /// containment check matches on `value` alone.
    pub async fn find_by_identifier_value<C: ConnectionTrait>(
        conn: &C,
        value: &str,
    ) -> Result<Option<Patient>> {
        let payload = format!(
            "[{{\"value\":{}}}]",
            serde_json::Value::String(value.into())
        );
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM patients \
             WHERE deleted_at IS NULL \
             AND identifiers @> $1::jsonb \
             LIMIT 1",
            [payload.into()],
        );
        let m = patient::Entity::find()
            .from_raw_sql(stmt)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Look up a patient by their MPI identity. Returns `Ok(None)` if no
    /// row matches; if two rows match (shouldn't happen — `mpi_id` is
    /// unique in the schema), returns the first.
    pub async fn find_by_mpi_id<C: ConnectionTrait>(
        conn: &C,
        mpi_id: Uuid,
    ) -> Result<Option<Patient>> {
        let m = patient::Entity::find()
            .filter(patient::Column::MpiId.eq(mpi_id))
            .filter(patient::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }
}

// --- conversion helpers ---

fn gender_to_str(g: Gender) -> &'static str {
    match g {
        Gender::Male => "male",
        Gender::Female => "female",
        Gender::Other => "other",
        Gender::Unknown => "unknown",
    }
}

fn gender_from_str(s: &str) -> Result<Gender> {
    match s {
        "male" => Ok(Gender::Male),
        "female" => Ok(Gender::Female),
        "other" => Ok(Gender::Other),
        "unknown" => Ok(Gender::Unknown),
        other => Err(Error::internal(format!("unknown gender: {other}"))),
    }
}

fn to_active_model(p: &Patient) -> Result<patient::ActiveModel> {
    Ok(patient::ActiveModel {
        id: Set(p.id),
        mpi_id: Set(p.mpi_id),
        active: Set(p.active),
        name: Set(serde_json::to_value(&p.name)
            .map_err(|e| Error::internal(format!("serialize name: {e}")))?),
        additional_names: Set(serde_json::to_value(&p.additional_names)
            .map_err(|e| Error::internal(format!("serialize additional_names: {e}")))?),
        identifiers: Set(serde_json::to_value(&p.identifiers)
            .map_err(|e| Error::internal(format!("serialize identifiers: {e}")))?),
        telecom: Set(serde_json::to_value(&p.telecom)
            .map_err(|e| Error::internal(format!("serialize telecom: {e}")))?),
        addresses: Set(serde_json::to_value(&p.addresses)
            .map_err(|e| Error::internal(format!("serialize addresses: {e}")))?),
        gender: Set(gender_to_str(p.gender).to_string()),
        birth_date: Set(p.birth_date),
        deceased: Set(p.deceased),
        deceased_datetime: Set(p.deceased_datetime.map(|t| t.fixed_offset())),
        emergency_contacts: Set(serde_json::to_value(&p.emergency_contacts)
            .map_err(|e| Error::internal(format!("serialize emergency_contacts: {e}")))?),
        marital_status: Set(p.marital_status.clone()),
        replaced_by: Set(p.replaced_by),
        deleted_at: Set(None),
        created_at: Set(p.created_at.fixed_offset()),
        updated_at: Set(p.updated_at.fixed_offset()),
    })
}

fn to_active_model_for_update(p: &Patient) -> Result<patient::ActiveModel> {
    let mut am = to_active_model(p)?;
    // Keep `created_at` unchanged on update — only `updated_at` advances.
    am.created_at = NotSet;
    Ok(am)
}

fn from_model(m: patient::Model) -> Result<Patient> {
    Ok(Patient {
        id: m.id,
        mpi_id: m.mpi_id,
        identifiers: serde_json::from_value(m.identifiers)
            .map_err(|e| Error::internal(format!("deserialize identifiers: {e}")))?,
        active: m.active,
        name: serde_json::from_value(m.name)
            .map_err(|e| Error::internal(format!("deserialize name: {e}")))?,
        additional_names: serde_json::from_value(m.additional_names)
            .map_err(|e| Error::internal(format!("deserialize additional_names: {e}")))?,
        telecom: serde_json::from_value(m.telecom)
            .map_err(|e| Error::internal(format!("deserialize telecom: {e}")))?,
        gender: gender_from_str(&m.gender)?,
        birth_date: m.birth_date,
        addresses: serde_json::from_value(m.addresses)
            .map_err(|e| Error::internal(format!("deserialize addresses: {e}")))?,
        deceased: m.deceased,
        deceased_datetime: m.deceased_datetime.map(|t| t.with_timezone(&chrono::Utc)),
        emergency_contacts: serde_json::from_value(m.emergency_contacts)
            .map_err(|e| Error::internal(format!("deserialize emergency_contacts: {e}")))?,
        marital_status: m.marital_status,
        replaced_by: m.replaced_by,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::patient::HumanName;
    use crate::models::{Gender, NameUse};

    fn sample_patient() -> Patient {
        let name = HumanName {
            use_type: Some(NameUse::Official),
            family: "Doe".into(),
            given: vec!["Jane".into()],
            prefix: vec![],
            suffix: vec![],
        };
        Patient::new(name, Gender::Female)
    }

    #[test]
    fn test_patient_roundtrip_via_active_model() {
        let p = sample_patient();
        let am = to_active_model(&p).expect("to_active_model");
        // Reconstruct a Model by extracting each Set value.
        let m = patient::Model {
            id: am.id.clone().unwrap(),
            mpi_id: am.mpi_id.clone().unwrap(),
            active: am.active.clone().unwrap(),
            name: am.name.clone().unwrap(),
            additional_names: am.additional_names.clone().unwrap(),
            identifiers: am.identifiers.clone().unwrap(),
            telecom: am.telecom.clone().unwrap(),
            addresses: am.addresses.clone().unwrap(),
            gender: am.gender.clone().unwrap(),
            birth_date: am.birth_date.clone().unwrap(),
            deceased: am.deceased.clone().unwrap(),
            deceased_datetime: am.deceased_datetime.clone().unwrap(),
            emergency_contacts: am.emergency_contacts.clone().unwrap(),
            marital_status: am.marital_status.clone().unwrap(),
            replaced_by: am.replaced_by.clone().unwrap(),
            deleted_at: am.deleted_at.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, p.id);
        assert_eq!(back.name.family, "Doe");
        assert_eq!(back.gender, Gender::Female);
        assert!(back.active);
    }
}
