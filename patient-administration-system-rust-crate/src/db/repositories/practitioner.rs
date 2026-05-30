//! practitioner repository (v0.24).
//!
//! The REST + FHIR Practitioner handlers were originally written
//! against the SeaORM `practitioner::Entity` directly (no repo). v0.24
//! adds this thin repository to back the new MFN^M02 inbound handler,
//! which needs to:
//!   * look up a practitioner by an external (EMR-issued) identifier
//!     value, to support `MUP` / `MDL` against an existing row;
//!   * insert / update / set-active in transactional contexts (one DB
//!     txn wraps a whole MFN message).
//!
//! The repo carries only what MFN needs. Callers that want the full
//! REST surface continue to use the inline-ActiveModel path in
//! `src/api/rest/handlers.rs` for now.

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::practitioner;
use crate::models::Gender;
use crate::models::patient::HumanName;
use crate::models::practitioner::Practitioner;
use crate::{Error, Result};

pub struct PractitionerRepository;

impl PractitionerRepository {
    /// Insert a new practitioner row.
    pub async fn create<C: ConnectionTrait>(conn: &C, p: &Practitioner) -> Result<Practitioner> {
        let am = to_active_model(p);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(p.clone())
    }

    /// Locate a practitioner by id.
    pub async fn find_by_id<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<Practitioner>> {
        let m = practitioner::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Locate a practitioner by an identifier value. Uses the
    /// Postgres JSONB containment operator (`@>`) against the
    /// `identifiers` column, identical in shape to
    /// `PatientRepository::find_by_identifier_value`. Callers should
    /// still verify the identifier `system` on the returned row if
    /// they care which scheme the value belongs to (the containment
    /// check matches on `value` alone).
    pub async fn find_by_identifier_value<C: ConnectionTrait>(
        conn: &C,
        value: &str,
    ) -> Result<Option<Practitioner>> {
        let payload = format!(
            "[{{\"value\":{}}}]",
            serde_json::Value::String(value.into())
        );
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM practitioners \
             WHERE identifiers @> $1::jsonb \
             LIMIT 1",
            [payload.into()],
        );
        let m = practitioner::Entity::find()
            .from_raw_sql(stmt)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Full replace by id. Returns `Error::NotFound` when the row is
    /// missing.
    pub async fn update<C: ConnectionTrait>(conn: &C, p: &Practitioner) -> Result<Practitioner> {
        let existing = practitioner::Entity::find_by_id(p.id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("practitioner {}", p.id)))?;
        let mut am: practitioner::ActiveModel = existing.into();
        am.active = Set(p.active);
        am.name = Set(serde_json::to_value(&p.name).unwrap_or_default());
        am.identifiers = Set(serde_json::to_value(&p.identifiers).unwrap_or_default());
        am.telecom = Set(serde_json::to_value(&p.telecom).unwrap_or_default());
        am.addresses = Set(serde_json::to_value(&p.addresses).unwrap_or_default());
        am.gender = Set(gender_to_str(p.gender).to_string());
        am.birth_date = Set(p.birth_date);
        am.updated_at = Set(chrono::Utc::now().fixed_offset());
        am.update(conn).await.map_err(Error::Database)?;
        Ok(p.clone())
    }

    /// Flip the `active` flag. Used by the MFN^M02 `MDL` (delete)
    /// path — practitioner rows are never hard-deleted because
    /// downstream encounter / appointment / schedule rows hold
    /// `practitioner_id` references.
    pub async fn set_active<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        active: bool,
    ) -> Result<Practitioner> {
        let m = practitioner::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("practitioner {id}")))?;
        let mut am: practitioner::ActiveModel = m.into();
        am.active = Set(active);
        am.updated_at = Set(chrono::Utc::now().fixed_offset());
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
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

fn gender_from_str(s: &str) -> Gender {
    match s {
        "male" => Gender::Male,
        "female" => Gender::Female,
        "other" => Gender::Other,
        _ => Gender::Unknown,
    }
}

fn to_active_model(p: &Practitioner) -> practitioner::ActiveModel {
    practitioner::ActiveModel {
        id: Set(p.id),
        active: Set(p.active),
        name: Set(serde_json::to_value(&p.name).unwrap_or_default()),
        identifiers: Set(serde_json::to_value(&p.identifiers).unwrap_or_default()),
        telecom: Set(serde_json::to_value(&p.telecom).unwrap_or_default()),
        addresses: Set(serde_json::to_value(&p.addresses).unwrap_or_default()),
        gender: Set(gender_to_str(p.gender).to_string()),
        birth_date: Set(p.birth_date),
        created_at: Set(p.created_at.fixed_offset()),
        updated_at: Set(p.updated_at.fixed_offset()),
    }
}

fn from_model(m: practitioner::Model) -> Result<Practitioner> {
    let name: HumanName = serde_json::from_value(m.name)
        .map_err(|e| Error::internal(format!("deserialize practitioner name: {e}")))?;
    let identifiers = serde_json::from_value(m.identifiers)
        .map_err(|e| Error::internal(format!("deserialize identifiers: {e}")))?;
    let telecom = serde_json::from_value(m.telecom)
        .map_err(|e| Error::internal(format!("deserialize telecom: {e}")))?;
    let addresses = serde_json::from_value(m.addresses)
        .map_err(|e| Error::internal(format!("deserialize addresses: {e}")))?;
    Ok(Practitioner {
        id: m.id,
        identifiers,
        active: m.active,
        name,
        telecom,
        addresses,
        gender: gender_from_str(&m.gender),
        birth_date: m.birth_date,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_practitioner_repository_is_zero_sized() {
        assert_eq!(std::mem::size_of::<PractitionerRepository>(), 0);
    }

    #[test]
    fn test_gender_string_round_trip() {
        for g in [Gender::Male, Gender::Female, Gender::Other, Gender::Unknown] {
            assert_eq!(gender_from_str(gender_to_str(g)), g);
        }
        assert_eq!(gender_from_str("not-a-real-value"), Gender::Unknown);
    }
}
