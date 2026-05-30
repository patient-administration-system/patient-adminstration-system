//! coverage repository (v0.10.0).
//!
//! `coverages` is a flat table — no JSONB columns, no joins required.
//! The repo carries the usual create / read / update / list shape plus
//! two convenience methods (`link_to_account`, `set_status`) that are
//! cheaper than a full PUT round-trip.

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::coverage;
use crate::models::coverage::{Coverage, CoverageKind, CoverageStatus};
use crate::{Error, Result};

pub struct CoverageRepository;

impl CoverageRepository {
    /// Insert a new coverage row.
    pub async fn create<C: ConnectionTrait>(conn: &C, c: &Coverage) -> Result<Coverage> {
        let am = to_active_model(c);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(c.clone())
    }

    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Coverage>> {
        let m = coverage::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Replace the row in place. Caller is expected to have produced a
    /// merged record (existing-row + patch) before calling.
    pub async fn update<C: ConnectionTrait>(conn: &C, c: &Coverage) -> Result<Coverage> {
        let mut am = to_active_model(c);
        am.updated_at = Set(chrono::Utc::now().fixed_offset());
        am.update(conn).await.map_err(Error::Database)?;
        Ok(c.clone())
    }

    /// Flip just the status column. Used by the soft-cancel endpoint
    /// (`DELETE /api/coverages/:id` → status = Cancelled).
    pub async fn set_status<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        new_status: CoverageStatus,
    ) -> Result<Coverage> {
        let m = coverage::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("coverage {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am: coverage::ActiveModel = m.into();
        am.status = Set(new_status.as_str().to_string());
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
    }

    /// Attach a coverage to a billing account, or detach by passing
    /// `None`.
    pub async fn link_to_account<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        account_id: Option<Uuid>,
    ) -> Result<Coverage> {
        let m = coverage::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("coverage {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am: coverage::ActiveModel = m.into();
        am.account_id = Set(account_id);
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
    }

    /// All coverage rows for a patient, newest-first.
    pub async fn list_by_patient<C: ConnectionTrait>(
        conn: &C,
        patient_id: Uuid,
    ) -> Result<Vec<Coverage>> {
        let rows = coverage::Entity::find()
            .filter(coverage::Column::PatientId.eq(patient_id))
            .order_by_desc(coverage::Column::CreatedAt)
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// All coverage rows attached to a billing account, newest-first.
    /// Useful when surfacing payer info on an invoice or statement.
    pub async fn list_by_account<C: ConnectionTrait>(
        conn: &C,
        account_id: Uuid,
    ) -> Result<Vec<Coverage>> {
        let rows = coverage::Entity::find()
            .filter(coverage::Column::AccountId.eq(account_id))
            .order_by_desc(coverage::Column::CreatedAt)
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }
}

// --- conversions ---

fn status_from_str(s: &str) -> Result<CoverageStatus> {
    match s {
        "active" => Ok(CoverageStatus::Active),
        "cancelled" => Ok(CoverageStatus::Cancelled),
        "draft" => Ok(CoverageStatus::Draft),
        "entered_in_error" => Ok(CoverageStatus::EnteredInError),
        other => Err(Error::internal(format!("unknown coverage status: {other}"))),
    }
}

fn kind_from_str(s: &str) -> Result<CoverageKind> {
    match s {
        "insurance" => Ok(CoverageKind::Insurance),
        "self_pay" => Ok(CoverageKind::SelfPay),
        "other" => Ok(CoverageKind::Other),
        other => Err(Error::internal(format!("unknown coverage kind: {other}"))),
    }
}

fn to_active_model(c: &Coverage) -> coverage::ActiveModel {
    coverage::ActiveModel {
        id: Set(c.id),
        patient_id: Set(c.patient_id),
        account_id: Set(c.account_id),
        status: Set(c.status.as_str().to_string()),
        kind: Set(c.kind.as_str().to_string()),
        subscriber_id: Set(c.subscriber_id),
        payor_name: Set(c.payor_name.clone()),
        payor_identifier: Set(c.payor_identifier.clone()),
        policy_number: Set(c.policy_number.clone()),
        group_number: Set(c.group_number.clone()),
        relationship: Set(c.relationship.clone()),
        start_date: Set(c.start_date),
        end_date: Set(c.end_date),
        created_at: Set(c.created_at.fixed_offset()),
        updated_at: Set(c.updated_at.fixed_offset()),
    }
}

fn from_model(m: coverage::Model) -> Result<Coverage> {
    Ok(Coverage {
        id: m.id,
        patient_id: m.patient_id,
        account_id: m.account_id,
        status: status_from_str(&m.status)?,
        kind: kind_from_str(&m.kind)?,
        subscriber_id: m.subscriber_id,
        payor_name: m.payor_name,
        payor_identifier: m.payor_identifier,
        policy_number: m.policy_number,
        group_number: m.group_number,
        relationship: m.relationship,
        start_date: m.start_date,
        end_date: m.end_date,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coverage_repository_is_zero_sized() {
        assert_eq!(std::mem::size_of::<CoverageRepository>(), 0);
    }

    #[test]
    fn test_status_string_round_trip_via_helpers() {
        for s in [
            CoverageStatus::Active,
            CoverageStatus::Cancelled,
            CoverageStatus::Draft,
            CoverageStatus::EnteredInError,
        ] {
            assert_eq!(status_from_str(s.as_str()).unwrap(), s);
        }
        assert!(status_from_str("not-a-real-status").is_err());
    }

    #[test]
    fn test_kind_string_round_trip_via_helpers() {
        for k in [
            CoverageKind::Insurance,
            CoverageKind::SelfPay,
            CoverageKind::Other,
        ] {
            assert_eq!(kind_from_str(k.as_str()).unwrap(), k);
        }
        assert!(kind_from_str("not-a-real-kind").is_err());
    }

    #[test]
    fn test_to_active_model_round_trip_via_explicit_model() {
        let mut c = Coverage::new(Uuid::new_v4(), "Aetna", "P-1");
        c.account_id = Some(Uuid::new_v4());
        c.kind = CoverageKind::SelfPay;
        let am = to_active_model(&c);
        let m = coverage::Model {
            id: am.id.clone().unwrap(),
            patient_id: am.patient_id.clone().unwrap(),
            account_id: am.account_id.clone().unwrap(),
            status: am.status.clone().unwrap(),
            kind: am.kind.clone().unwrap(),
            subscriber_id: am.subscriber_id.clone().unwrap(),
            payor_name: am.payor_name.clone().unwrap(),
            payor_identifier: am.payor_identifier.clone().unwrap(),
            policy_number: am.policy_number.clone().unwrap(),
            group_number: am.group_number.clone().unwrap(),
            relationship: am.relationship.clone().unwrap(),
            start_date: am.start_date.clone().unwrap(),
            end_date: am.end_date.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, c.id);
        assert_eq!(back.account_id, c.account_id);
        assert_eq!(back.kind, CoverageKind::SelfPay);
        assert_eq!(back.payor_name, "Aetna");
        assert_eq!(back.policy_number, "P-1");
    }
}
