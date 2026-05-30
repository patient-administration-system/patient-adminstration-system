//! Recurring appointment series service (v0.9.0).
//!
//! The orchestration around `appointment_series` rows + the N
//! `appointments` they expand into. The pure recurrence math lives in
//! [`crate::models::appointment_series::compute_occurrences`]; this
//! module bridges it to persistence + audit + outbox.
//!
//! Two transactional invariants:
//!
//! 1. **Create is atomic.** The series row, each occurrence row, the
//!    audit log entry, and the outbox event all commit together; any
//!    per-patient overlap rolls back the whole thing.
//! 2. **Cancel only touches future / not-yet-fulfilled occurrences.**
//!    Past appointments (already `Fulfilled` / `NoShow` / `Cancelled`)
//!    are left exactly as the audit trail recorded them.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::{DatabaseConnection, TransactionTrait};
use uuid::Uuid;

use crate::db::repositories::{
    appointment::AppointmentRepository,
    appointment_series::AppointmentSeriesRepository,
    audit::{AuditLogRepository, UserContext},
    outbox::OutboxRepository,
};
use crate::models::appointment::{Appointment, AppointmentStatus, CancellationReason};
use crate::models::appointment_series::{
    AppointmentSeries, RecurrenceRule, SeriesStatus, compute_occurrences,
};
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

/// Application service for recurring appointment series.
pub struct SeriesService {
    db: DatabaseConnection,
    publisher: Arc<dyn EventPublisher>,
}

/// Input payload for [`SeriesService::preview`] and
/// [`SeriesService::create`]. Same shape on the wire as the REST
/// request structs in `api::rest::handlers`.
#[derive(Debug, Clone)]
pub struct CreateSeriesInput {
    pub patient_id: Uuid,
    pub practitioner_id: Option<Uuid>,
    pub service_type: String,
    pub start_datetime: DateTime<Utc>,
    pub duration_minutes: u32,
    pub rule: RecurrenceRule,
    pub reason: Option<String>,
}

/// Result of [`SeriesService::preview`] — the computed occurrence
/// datetimes without any DB writes. The UI calls this before the user
/// confirms `create`, so users can spot a clinic-closed Wednesday before
/// 26 weekly appointments hit the database.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreviewResult {
    pub occurrences: Vec<DateTime<Utc>>,
    pub total: usize,
}

/// Result of [`SeriesService::create`] — the persisted series + its
/// concrete appointment rows.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreateSeriesResult {
    pub series: AppointmentSeries,
    pub appointments: Vec<Appointment>,
}

/// Result of [`SeriesService::get_with_occurrences`] and the cancel
/// endpoint. Shape mirrors `CreateSeriesResult`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SeriesWithOccurrences {
    pub series: AppointmentSeries,
    pub appointments: Vec<Appointment>,
}

impl SeriesService {
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self { db, publisher }
    }

    /// Pure dry-run: compute the recurrence's concrete datetimes and
    /// hand them back. No DB hit. No overlap check — the UI is welcome
    /// to call `find_overlapping_for_patient` separately if it wants
    /// to surface conflicts in the preview, but the *contract* of
    /// preview is "this is what `create` would attempt to insert".
    pub fn preview(&self, input: &CreateSeriesInput) -> Result<PreviewResult> {
        let occurrences = compute_occurrences(&input.rule, input.start_datetime)?;
        let total = occurrences.len();
        Ok(PreviewResult { occurrences, total })
    }

    /// Persist a series + every occurrence in one DB transaction. Any
    /// per-patient overlap on any occurrence rolls back the whole
    /// transaction — the caller can re-run after resolving conflicts.
    ///
    /// On success: emits `AppointmentSeriesCreated` to the outbox and
    /// publishes a best-effort event to the in-process [`EventPublisher`].
    pub async fn create(
        &self,
        input: CreateSeriesInput,
        ctx: &UserContext,
    ) -> Result<CreateSeriesResult> {
        // Validate + expand up front so a bad rule fails before we open
        // the transaction. compute_occurrences re-validates internally,
        // but doing it twice is cheap and gives a cleaner error.
        input.rule.validate()?;
        let occurrences_at = compute_occurrences(&input.rule, input.start_datetime)?;
        if occurrences_at.is_empty() {
            return Err(Error::validation(
                "recurrence rule produced zero occurrences (until-before-start?)",
            ));
        }
        if input.duration_minutes == 0 {
            return Err(Error::validation("duration_minutes must be >= 1"));
        }

        let series = AppointmentSeries {
            id: Uuid::new_v4(),
            patient_id: input.patient_id,
            practitioner_id: input.practitioner_id,
            service_type: input.service_type.clone(),
            start_datetime: input.start_datetime,
            duration_minutes: input.duration_minutes,
            rule: input.rule.clone(),
            status: SeriesStatus::Active,
            reason: input.reason.clone(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let series_clone = series.clone();
        let ctx_clone = ctx.clone();
        let duration_minutes = input.duration_minutes;
        let res = self
            .db
            .transaction::<_, CreateSeriesResult, Error>(|txn| {
                Box::pin(async move {
                    AppointmentSeriesRepository::create(txn, &series_clone).await?;
                    let mut appointments = Vec::with_capacity(occurrences_at.len());
                    for start in occurrences_at {
                        let end = start + chrono::Duration::minutes(duration_minutes as i64);
                        // Atomic overlap reject: any conflict on any
                        // occurrence aborts the series. The diagnostic
                        // names the conflicting appointment + datetime
                        // so the user can fix it before the next try.
                        let conflicts = AppointmentRepository::find_overlapping_for_patient(
                            txn,
                            series_clone.patient_id,
                            start,
                            end,
                        )
                        .await?;
                        if let Some(c) = conflicts.first() {
                            return Err(Error::conflict(format!(
                                "series occurrence at {start} overlaps existing \
                                 appointment {} ({}..{})",
                                c.id, c.start_datetime, c.end_datetime
                            )));
                        }
                        let mut a = Appointment::new(series_clone.patient_id, start, end);
                        a.practitioner_id = series_clone.practitioner_id;
                        a.status = AppointmentStatus::Booked;
                        a.reason = series_clone.reason.clone();
                        a.series_id = Some(series_clone.id);
                        let saved = AppointmentRepository::create(txn, &a).await?;
                        appointments.push(saved);
                    }
                    AuditLogRepository::log(
                        txn,
                        "appointment_series",
                        series_clone.id,
                        "create",
                        None,
                        Some(serde_json::json!({
                            "series_id": series_clone.id,
                            "patient_id": series_clone.patient_id,
                            "count": appointments.len(),
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "AppointmentSeriesCreated",
                        &serde_json::json!({
                            "series_id": series_clone.id,
                            "patient_id": series_clone.patient_id,
                            "count": appointments.len(),
                        }),
                    )
                    .await?;
                    Ok(CreateSeriesResult {
                        series: series_clone,
                        appointments,
                    })
                })
            })
            .await
            .map_err(unwrap_txn_err)?;

        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "AppointmentSeriesCreated",
                serde_json::json!({
                    "series_id": res.series.id,
                    "count": res.appointments.len(),
                }),
            ))
            .await;
        Ok(res)
    }

    /// Cancel the series and every "future" occurrence (`Proposed` or
    /// `Booked`). `Arrived` / `Fulfilled` / `NoShow` / already-`Cancelled`
    /// rows are left alone.
    ///
    /// Runs in one transaction: series row flipped, each occurrence
    /// flipped, audit + outbox written.
    pub async fn cancel(
        &self,
        series_id: Uuid,
        reason: CancellationReason,
        ctx: &UserContext,
    ) -> Result<SeriesWithOccurrences> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .transaction::<_, SeriesWithOccurrences, Error>(|txn| {
                Box::pin(async move {
                    let series = AppointmentSeriesRepository::find_by_id(txn, series_id)
                        .await?
                        .ok_or_else(|| {
                            Error::not_found(format!("appointment_series {series_id}"))
                        })?;
                    let occurrences =
                        AppointmentSeriesRepository::list_occurrences(txn, series_id).await?;
                    let mut updated_occurrences = Vec::with_capacity(occurrences.len());
                    let mut cancelled_count = 0usize;
                    for occ in occurrences {
                        if matches!(
                            occ.status,
                            AppointmentStatus::Proposed | AppointmentStatus::Booked
                        ) {
                            if let Some(updated) = AppointmentRepository::set_status_and_reason(
                                txn,
                                occ.id,
                                AppointmentStatus::Cancelled,
                                Some(reason),
                            )
                            .await?
                            {
                                cancelled_count += 1;
                                updated_occurrences.push(updated);
                            } else {
                                updated_occurrences.push(occ);
                            }
                        } else {
                            updated_occurrences.push(occ);
                        }
                    }
                    let series_after =
                        AppointmentSeriesRepository::mark_cancelled(txn, series_id).await?;
                    AuditLogRepository::log(
                        txn,
                        "appointment_series",
                        series_id,
                        "cancel",
                        None,
                        Some(serde_json::json!({
                            "series_id": series_id,
                            "cancelled_occurrences": cancelled_count,
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "AppointmentSeriesCancelled",
                        &serde_json::json!({
                            "series_id": series_id,
                            "patient_id": series.patient_id,
                            "cancelled_occurrences": cancelled_count,
                        }),
                    )
                    .await?;
                    Ok(SeriesWithOccurrences {
                        series: series_after,
                        appointments: updated_occurrences,
                    })
                })
            })
            .await
            .map_err(unwrap_txn_err)?;

        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "AppointmentSeriesCancelled",
                serde_json::json!({ "series_id": series_id }),
            ))
            .await;
        Ok(res)
    }

    /// Read the series row + every linked occurrence (any status).
    pub async fn get_with_occurrences(
        &self,
        series_id: Uuid,
    ) -> Result<Option<SeriesWithOccurrences>> {
        let series = AppointmentSeriesRepository::find_by_id(&self.db, series_id).await?;
        let Some(series) = series else {
            return Ok(None);
        };
        let appointments =
            AppointmentSeriesRepository::list_occurrences(&self.db, series_id).await?;
        Ok(Some(SeriesWithOccurrences {
            series,
            appointments,
        }))
    }

    /// List every series belonging to a patient, newest-first.
    pub async fn list_by_patient(&self, patient_id: Uuid) -> Result<Vec<AppointmentSeries>> {
        AppointmentSeriesRepository::list_by_patient(&self.db, patient_id).await
    }
}

fn unwrap_txn_err(e: sea_orm::TransactionError<Error>) -> Error {
    match e {
        sea_orm::TransactionError::Connection(c) => Error::Database(c),
        sea_orm::TransactionError::Transaction(t) => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::appointment_series::{Frequency, RecurrenceEnd};

    #[test]
    fn test_preview_input_struct_constructs() {
        // Smoke test the input struct — real exercise lives in the
        // integration test where a DB is available.
        let _ = CreateSeriesInput {
            patient_id: Uuid::new_v4(),
            practitioner_id: None,
            service_type: "cardiology".into(),
            start_datetime: chrono::Utc::now(),
            duration_minutes: 30,
            rule: RecurrenceRule {
                frequency: Frequency::Weekly,
                interval: 1,
                by_weekday: None,
                end: RecurrenceEnd::Count { count: 4 },
            },
            reason: None,
        };
    }
}
