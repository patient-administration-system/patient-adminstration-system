//! appointment_series repository (v0.9.0).
//!
//! Persists [`AppointmentSeries`] aggregates. The individual occurrence
//! rows live in `appointments` and are managed by
//! [`AppointmentRepository`] — this repo only owns the series record
//! itself.

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::appointment_series;
use crate::db::repositories::appointment::AppointmentRepository;
use crate::models::appointment_series::{AppointmentSeries, SeriesStatus};
use crate::{Error, Result};

pub struct AppointmentSeriesRepository;

impl AppointmentSeriesRepository {
    /// Insert a new series row. The N occurrence rows are inserted
    /// separately by the caller (typically inside the same DB
    /// transaction) via [`AppointmentRepository::create`].
    pub async fn create<C: ConnectionTrait>(
        conn: &C,
        s: &AppointmentSeries,
    ) -> Result<AppointmentSeries> {
        let am = to_active_model(s)?;
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(s.clone())
    }

    pub async fn find_by_id<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<AppointmentSeries>> {
        let m = appointment_series::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    /// Every series row owned by a patient, newest-first.
    pub async fn list_by_patient<C: ConnectionTrait>(
        conn: &C,
        patient_id: Uuid,
    ) -> Result<Vec<AppointmentSeries>> {
        let rows = appointment_series::Entity::find()
            .filter(appointment_series::Column::PatientId.eq(patient_id))
            .order_by_desc(appointment_series::Column::CreatedAt)
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Flip the series row's status to `Cancelled` and stamp `updated_at`.
    /// Does **not** touch the contained appointment rows; callers cancel
    /// those separately so the audit trail records each one.
    pub async fn mark_cancelled<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<AppointmentSeries> {
        let m = appointment_series::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("appointment_series {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am: appointment_series::ActiveModel = m.into();
        am.status = Set(series_status_to_str(SeriesStatus::Cancelled).to_string());
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        from_model(updated)
    }

    /// Delegate to [`AppointmentRepository::list_by_series`] — convenience
    /// on the series repo so callers don't have to remember which side
    /// the join lives on.
    pub async fn list_occurrences<C: ConnectionTrait>(
        conn: &C,
        series_id: Uuid,
    ) -> Result<Vec<crate::models::appointment::Appointment>> {
        AppointmentRepository::list_by_series(conn, series_id).await
    }
}

// --- conversions ---

fn series_status_to_str(s: SeriesStatus) -> &'static str {
    match s {
        SeriesStatus::Active => "active",
        SeriesStatus::Cancelled => "cancelled",
    }
}

fn series_status_from_str(s: &str) -> Result<SeriesStatus> {
    match s {
        "active" => Ok(SeriesStatus::Active),
        "cancelled" => Ok(SeriesStatus::Cancelled),
        other => Err(Error::internal(format!("unknown series status: {other}"))),
    }
}

fn to_active_model(s: &AppointmentSeries) -> Result<appointment_series::ActiveModel> {
    Ok(appointment_series::ActiveModel {
        id: Set(s.id),
        patient_id: Set(s.patient_id),
        practitioner_id: Set(s.practitioner_id),
        service_type: Set(s.service_type.clone()),
        start_datetime: Set(s.start_datetime.fixed_offset()),
        duration_minutes: Set(s.duration_minutes as i32),
        rule: Set(serde_json::to_value(&s.rule)
            .map_err(|e| Error::internal(format!("serialize rule: {e}")))?),
        status: Set(series_status_to_str(s.status).to_string()),
        reason: Set(s.reason.clone()),
        created_at: Set(s.created_at.fixed_offset()),
        updated_at: Set(s.updated_at.fixed_offset()),
    })
}

fn from_model(m: appointment_series::Model) -> Result<AppointmentSeries> {
    Ok(AppointmentSeries {
        id: m.id,
        patient_id: m.patient_id,
        practitioner_id: m.practitioner_id,
        service_type: m.service_type,
        start_datetime: m.start_datetime.with_timezone(&chrono::Utc),
        duration_minutes: m.duration_minutes.max(0) as u32,
        rule: serde_json::from_value(m.rule)
            .map_err(|e| Error::internal(format!("deserialize rule: {e}")))?,
        status: series_status_from_str(&m.status)?,
        reason: m.reason,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::appointment_series::{Frequency, RecurrenceEnd, RecurrenceRule};

    #[test]
    fn test_series_status_roundtrip() {
        for s in [SeriesStatus::Active, SeriesStatus::Cancelled] {
            assert_eq!(series_status_from_str(series_status_to_str(s)).unwrap(), s);
        }
    }

    #[test]
    fn test_to_active_model_serialises_rule_to_json() {
        let s = AppointmentSeries::new(
            Uuid::new_v4(),
            "cardiology",
            chrono::Utc::now(),
            30,
            RecurrenceRule {
                frequency: Frequency::Weekly,
                interval: 1,
                by_weekday: None,
                end: RecurrenceEnd::Count { count: 4 },
            },
        );
        let am = to_active_model(&s).expect("to am");
        let rule_json = am.rule.clone().unwrap();
        // The JSON blob contains the FREQ marker so a quick eyeball
        // assertion catches accidental schema drift.
        assert!(
            rule_json.to_string().contains("weekly"),
            "rule json should carry frequency: {rule_json}"
        );
    }
}
