//! consent repository

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::consent;
use crate::models::consent::{Consent, ConsentStatus, ConsentType};
use crate::{Error, Result};

pub struct ConsentRepository;

impl ConsentRepository {
    pub async fn create<C: ConnectionTrait>(conn: &C, c: &Consent) -> Result<Consent> {
        let am = to_active_model(c);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(c.clone())
    }

    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Consent>> {
        let m = consent::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    pub async fn list_for_patient<C: ConnectionTrait>(
        conn: &C,
        patient_id: Uuid,
    ) -> Result<Vec<Consent>> {
        let rows = consent::Entity::find()
            .filter(consent::Column::PatientId.eq(patient_id))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Mark a consent as Revoked. Sets `status = Revoked` and stamps
    /// `revoked_date` to today.
    pub async fn revoke<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Consent> {
        let m = consent::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("consent {id}")))?;
        let today = chrono::Utc::now().date_naive();
        let mut am: consent::ActiveModel = m.into();
        am.status = Set(consent_status_to_str(ConsentStatus::Revoked).to_string());
        am.revoked_date = Set(Some(today));
        am.updated_at = Set(chrono::Utc::now().fixed_offset());
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
    }
}

// --- conversion helpers ---

pub(crate) fn consent_type_to_str(t: ConsentType) -> &'static str {
    match t {
        ConsentType::DataProcessing => "data_processing",
        ConsentType::DataSharing => "data_sharing",
        ConsentType::Marketing => "marketing",
        ConsentType::Research => "research",
        ConsentType::EmergencyAccess => "emergency_access",
    }
}

pub(crate) fn consent_type_from_str(s: &str) -> Result<ConsentType> {
    match s {
        "data_processing" => Ok(ConsentType::DataProcessing),
        "data_sharing" => Ok(ConsentType::DataSharing),
        "marketing" => Ok(ConsentType::Marketing),
        "research" => Ok(ConsentType::Research),
        "emergency_access" => Ok(ConsentType::EmergencyAccess),
        other => Err(Error::internal(format!("unknown consent type: {other}"))),
    }
}

pub(crate) fn consent_status_to_str(s: ConsentStatus) -> &'static str {
    match s {
        ConsentStatus::Active => "active",
        ConsentStatus::Revoked => "revoked",
        ConsentStatus::Expired => "expired",
    }
}

pub(crate) fn consent_status_from_str(s: &str) -> Result<ConsentStatus> {
    match s {
        "active" => Ok(ConsentStatus::Active),
        "revoked" => Ok(ConsentStatus::Revoked),
        "expired" => Ok(ConsentStatus::Expired),
        other => Err(Error::internal(format!("unknown consent status: {other}"))),
    }
}

fn to_active_model(c: &Consent) -> consent::ActiveModel {
    consent::ActiveModel {
        id: Set(c.id),
        patient_id: Set(c.patient_id),
        consent_type: Set(consent_type_to_str(c.consent_type).to_string()),
        status: Set(consent_status_to_str(c.status).to_string()),
        granted_date: Set(c.granted_date),
        expiry_date: Set(c.expiry_date),
        revoked_date: Set(c.revoked_date),
        purpose: Set(c.purpose.clone()),
        method: Set(c.method.clone()),
        created_at: Set(c.created_at.fixed_offset()),
        updated_at: Set(c.updated_at.fixed_offset()),
    }
}

fn from_model(m: consent::Model) -> Result<Consent> {
    Ok(Consent {
        id: m.id,
        patient_id: m.patient_id,
        consent_type: consent_type_from_str(&m.consent_type)?,
        status: consent_status_from_str(&m.status)?,
        granted_date: m.granted_date,
        expiry_date: m.expiry_date,
        revoked_date: m.revoked_date,
        purpose: m.purpose,
        method: m.method,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_consent_roundtrip_via_active_model() {
        let granted = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let c = Consent::new(Uuid::new_v4(), ConsentType::DataProcessing, granted);
        let am = to_active_model(&c);
        let m = consent::Model {
            id: am.id.clone().unwrap(),
            patient_id: am.patient_id.clone().unwrap(),
            consent_type: am.consent_type.clone().unwrap(),
            status: am.status.clone().unwrap(),
            granted_date: am.granted_date.clone().unwrap(),
            expiry_date: am.expiry_date.clone().unwrap(),
            revoked_date: am.revoked_date.clone().unwrap(),
            purpose: am.purpose.clone().unwrap(),
            method: am.method.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, c.id);
        assert_eq!(back.consent_type, ConsentType::DataProcessing);
        assert_eq!(back.status, ConsentStatus::Active);
        assert_eq!(back.granted_date, granted);
    }
}
