//! adt — Admission/Discharge/Transfer service
//!
//! Orchestrates the ADT state machine on top of the persistence layer. Each
//! public method runs inside a single DB transaction so that:
//!
//! - the `Bed` row is locked with `SELECT … FOR UPDATE` before being touched,
//! - the entity row(s), audit log row, and outbox event row are all written
//!   in the same transaction (transactional outbox pattern),
//! - on commit, the service performs a best-effort publish via the configured
//!   [`EventPublisher`]. The outbox row remains the durable record of the
//!   event; the publish is a fire-and-forget convenience for in-process
//!   consumers.

use sea_orm::{DatabaseConnection, TransactionTrait};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::{
    admission::AdmissionRepository,
    audit::{AuditLogRepository, UserContext},
    bed::BedRepository,
    encounter::EncounterRepository,
    outbox::OutboxRepository,
};
use crate::models::admission::{Admission, BedAssignment, Discharge, Transfer};
use crate::models::encounter::{Encounter, EncounterClass, EncounterStatus};
use crate::models::facility::BedStatus;
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

/// The combined result of a successful admission: the new encounter, the
/// admission record, and the active bed assignment.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdmissionResult {
    pub encounter: Encounter,
    pub admission: Admission,
    pub bed_assignment: BedAssignment,
}

/// Application service for inpatient ADT (admission / discharge / transfer).
pub struct AdtService {
    db: DatabaseConnection,
    publisher: Arc<dyn EventPublisher>,
}

impl AdtService {
    /// Construct an ADT service bound to a database connection and event
    /// publisher.
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self { db, publisher }
    }

    /// Admit a patient to a bed.
    ///
    /// Transactional steps:
    ///
    /// 1. `SELECT … FOR UPDATE` the target bed.
    /// 2. Assert `Bed.status == Available`.
    /// 3. Create the `Encounter` (Inpatient, status transitions
    ///    Planned → Arrived → InProgress).
    /// 4. Flip the bed to `Occupied`.
    /// 5. Insert the `BedAssignment` and `Admission` rows.
    /// 6. Write audit and outbox rows.
    /// 7. Commit. After commit, publish a best-effort `EncounterAdmitted`
    ///    event via the configured publisher.
    pub async fn admit(
        &self,
        patient_id: Uuid,
        bed_id: Uuid,
        ctx: &UserContext,
    ) -> Result<AdmissionResult> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .transaction::<_, AdmissionResult, Error>(|txn| {
                Box::pin(async move {
                    let bed = BedRepository::select_for_update(txn, bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {bed_id}")))?;
                    if bed.status != BedStatus::Available {
                        return Err(Error::conflict(format!(
                            "bed {bed_id} is not available (status={:?})",
                            bed.status
                        )));
                    }

                    // Create encounter and advance status to InProgress.
                    let mut enc = Encounter::new(patient_id, EncounterClass::Inpatient);
                    enc.status = enc.status.try_transition_to(EncounterStatus::Arrived)?;
                    enc.status = enc.status.try_transition_to(EncounterStatus::InProgress)?;
                    let enc = EncounterRepository::create(txn, &enc).await?;

                    // Mark bed Occupied.
                    BedRepository::update_status(txn, bed_id, BedStatus::Occupied).await?;

                    // Insert the active bed assignment.
                    let ba = BedAssignment::new(enc.id, bed_id);
                    let ba = AdmissionRepository::create_bed_assignment(txn, &ba).await?;

                    // Insert the admission record.
                    let adm = Admission::new(enc.id, bed_id);
                    let adm = AdmissionRepository::create(txn, &adm).await?;

                    // Audit + outbox in the same transaction.
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        "admit",
                        None,
                        Some(serde_json::to_value(&enc).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "EncounterAdmitted",
                        &serde_json::json!({
                            "encounter_id": enc.id,
                            "patient_id": patient_id,
                            "bed_id": bed_id,
                            "admission_id": adm.id,
                        }),
                    )
                    .await?;

                    Ok(AdmissionResult {
                        encounter: enc,
                        admission: adm,
                        bed_assignment: ba,
                    })
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;

        // Best-effort publish after commit. The outbox row is the durable
        // record; this is a convenience for in-process consumers.
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "EncounterAdmitted",
                serde_json::json!({
                    "encounter_id": res.encounter.id,
                    "admission_id": res.admission.id,
                }),
            ))
            .await;
        Ok(res)
    }

    /// Pre-admit a patient (HL7 v2 ADT^A05 semantics): reserve a bed and
    /// create a `Planned`-status inpatient encounter without actually
    /// admitting the patient yet.
    ///
    /// Transactional steps: lock the destination bed (must be
    /// `Available`), flip it to `Reserved` via the regular state-machine
    /// transition (`Available → Reserved` is legal), create an
    /// `Encounter` in `Planned` status, write audit + outbox, commit.
    ///
    /// Unlike `admit`, this does *not* create an `Admission` row or a
    /// `BedAssignment` — the patient isn't physically in the bed yet.
    /// The bed reservation is tracked only via the bed's `Reserved`
    /// status. A subsequent ADT^A01 admit for the same patient is
    /// expected to land on a different code path that's aware of the
    /// pre-existing planned encounter (future work).
    pub async fn pre_admit(
        &self,
        patient_id: Uuid,
        bed_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Encounter> {
        let ctx_clone = ctx.clone();
        let source_owned = source.map(|s| s.to_string());
        let res = self
            .db
            .transaction::<_, Encounter, Error>(|txn| {
                let source_for_txn = source_owned.clone();
                Box::pin(async move {
                    let bed = BedRepository::select_for_update(txn, bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {bed_id}")))?;
                    if bed.status != BedStatus::Available {
                        return Err(Error::conflict(format!(
                            "bed {bed_id} is not available for pre-admit (status={:?})",
                            bed.status
                        )));
                    }
                    BedRepository::update_status(txn, bed_id, BedStatus::Reserved).await?;
                    let enc = Encounter::new(patient_id, EncounterClass::Inpatient);
                    let enc = EncounterRepository::create(txn, &enc).await?;
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        "pre_admit",
                        None,
                        Some(serde_json::to_value(&enc).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "encounter_id": enc.id,
                        "patient_id": patient_id,
                        "bed_id": bed_id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterPreAdmitted", &payload).await?;
                    Ok(enc)
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;

        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "EncounterPreAdmitted",
                serde_json::json!({
                    "encounter_id": res.id,
                    "patient_id": patient_id,
                    "bed_id": bed_id,
                }),
            ))
            .await;
        Ok(res)
    }

    /// Cancel a pre-admission (HL7 v2 ADT^A38 semantics): release the
    /// bed reservation and cancel the Planned inpatient encounter.
    ///
    /// Transactional steps: lock the bed (must be `Reserved` — anything
    /// else means the reservation already advanced or never existed),
    /// locate the patient's most-recent Planned inpatient encounter
    /// (404 if none), flip bed → `Available` via the regular state-
    /// machine transition (`Reserved → Available` is legal), set the
    /// encounter status → `Cancelled` via the regular state machine
    /// (`Planned → Cancelled` is legal), write audit + outbox, commit.
    pub async fn cancel_pre_admit(
        &self,
        patient_id: Uuid,
        bed_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Encounter> {
        let ctx_clone = ctx.clone();
        let source_owned = source.map(|s| s.to_string());
        let res = self
            .db
            .transaction::<_, Encounter, Error>(|txn| {
                let source_for_txn = source_owned.clone();
                Box::pin(async move {
                    let bed = BedRepository::select_for_update(txn, bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {bed_id}")))?;
                    if bed.status != BedStatus::Reserved {
                        return Err(Error::conflict(format!(
                            "bed {bed_id} is in status {:?}; expected Reserved for cancel-pre-admit",
                            bed.status
                        )));
                    }
                    let enc =
                        EncounterRepository::find_latest_planned_inpatient_for_patient(
                            txn,
                            patient_id,
                        )
                        .await?
                        .ok_or_else(|| {
                            Error::not_found(format!(
                                "patient {patient_id} has no planned inpatient encounter to cancel"
                            ))
                        })?;
                    BedRepository::update_status(txn, bed_id, BedStatus::Available).await?;
                    let enc = EncounterRepository::set_status(
                        txn,
                        enc.id,
                        EncounterStatus::Cancelled,
                    )
                    .await?;
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        "cancel_pre_admit",
                        None,
                        Some(serde_json::json!({
                            "patient_id": patient_id,
                            "bed_id": bed_id,
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "encounter_id": enc.id,
                        "patient_id": patient_id,
                        "bed_id": bed_id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterPreAdmitCancelled", &payload).await?;
                    Ok(enc)
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;

        let mut in_mem = serde_json::json!({
            "encounter_id": res.id,
            "patient_id": patient_id,
            "bed_id": bed_id,
        });
        if let Some(s) = source_owned.as_deref() {
            in_mem["source"] = serde_json::Value::String(s.to_string());
        }
        let _ = self
            .publisher
            .publish(DomainEvent::new("EncounterPreAdmitCancelled", in_mem))
            .await;
        Ok(res)
    }

    /// Mark a patient as on temporary leave of absence (HL7 v2
    /// ADT^A21 semantics): the encounter moves `InProgress → OnLeave`.
    /// The bed remains `Occupied` — the patient is expected back so
    /// the bed is held for them.
    pub async fn start_leave(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Encounter> {
        self.transition_loa(
            admission_id,
            source,
            ctx,
            EncounterStatus::OnLeave,
            "start_leave",
            "EncounterLeaveStarted",
        )
        .await
    }

    /// Mark a patient as returned from leave (HL7 v2 ADT^A22
    /// semantics): the encounter moves `OnLeave → InProgress`.
    pub async fn end_leave(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Encounter> {
        self.transition_loa(
            admission_id,
            source,
            ctx,
            EncounterStatus::InProgress,
            "end_leave",
            "EncounterLeaveEnded",
        )
        .await
    }

    async fn transition_loa(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
        target: EncounterStatus,
        audit_action: &'static str,
        event_type: &'static str,
    ) -> Result<Encounter> {
        let ctx_clone = ctx.clone();
        let source_owned = source.map(|s| s.to_string());
        let res = self
            .db
            .transaction::<_, Encounter, Error>(|txn| {
                let source_for_txn = source_owned.clone();
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;
                    // set_status validates the state-machine transition.
                    let enc =
                        EncounterRepository::set_status(txn, adm.encounter_id, target).await?;
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        audit_action,
                        None,
                        Some(serde_json::json!({
                            "admission_id": admission_id,
                            "status": format!("{:?}", target),
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "admission_id": admission_id,
                        "encounter_id": enc.id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, event_type, &payload).await?;
                    Ok(enc)
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;
        let mut in_mem = serde_json::json!({
            "admission_id": admission_id,
            "encounter_id": res.id,
        });
        if let Some(s) = source_owned.as_deref() {
            in_mem["source"] = serde_json::Value::String(s.to_string());
        }
        let _ = self
            .publisher
            .publish(DomainEvent::new(event_type, in_mem))
            .await;
        Ok(res)
    }

    /// Transfer an admitted patient to a new bed.
    ///
    /// Transactional steps: end the active `BedAssignment`, free the old bed
    /// (status → `Cleaning`), lock and assert the new bed is `Available`,
    /// mark it `Occupied`, insert the new `BedAssignment` and the `Transfer`
    /// record, then write audit and outbox rows.
    pub async fn transfer(
        &self,
        admission_id: Uuid,
        new_bed_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Transfer> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .transaction::<_, Transfer, Error>(|txn| {
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;
                    let active_ba =
                        AdmissionRepository::find_active_by_encounter(txn, adm.encounter_id)
                            .await?
                            .ok_or_else(|| Error::not_found("no active bed assignment"))?;
                    let old_bed_id = active_ba.bed_id;
                    if old_bed_id == new_bed_id {
                        return Err(Error::validation("cannot transfer to the same bed"));
                    }

                    // Lock and verify the new bed is available.
                    let new_bed = BedRepository::select_for_update(txn, new_bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {new_bed_id}")))?;
                    if new_bed.status != BedStatus::Available {
                        return Err(Error::conflict(format!(
                            "bed {new_bed_id} is not available"
                        )));
                    }

                    // Release the old assignment and free the old bed.
                    AdmissionRepository::release_bed_assignment(txn, active_ba.id).await?;
                    BedRepository::update_status(txn, old_bed_id, BedStatus::Cleaning).await?;
                    BedRepository::update_status(txn, new_bed_id, BedStatus::Occupied).await?;

                    // New assignment + transfer record.
                    let new_ba = BedAssignment::new(adm.encounter_id, new_bed_id);
                    let _ = AdmissionRepository::create_bed_assignment(txn, &new_ba).await?;
                    let t = Transfer::new(admission_id, old_bed_id, new_bed_id);
                    let t = AdmissionRepository::create_transfer(txn, &t).await?;

                    AuditLogRepository::log(
                        txn,
                        "admission",
                        admission_id,
                        "transfer",
                        None,
                        Some(serde_json::to_value(&t).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "EncounterTransferred",
                        &serde_json::json!({
                            "admission_id": admission_id,
                            "from_bed_id": old_bed_id,
                            "to_bed_id": new_bed_id,
                        }),
                    )
                    .await?;

                    Ok(t)
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;

        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "EncounterTransferred",
                serde_json::json!({ "admission_id": admission_id }),
            ))
            .await;
        Ok(res)
    }

    /// Promote an ambulatory encounter to an inpatient admission
    /// (HL7 v2 ADT^A06 semantics): find the patient's currently-active
    /// Outpatient or Emergency encounter, allocate a bed, reclassify
    /// the encounter to `Inpatient`, advance the status to
    /// `InProgress` if still `Arrived`, and write a fresh
    /// `Admission` row + `BedAssignment` so subsequent ADT messages
    /// (transfer, discharge) can locate the new inpatient state.
    ///
    /// Transactional steps:
    /// 1. Lock the destination bed (404 if missing; 409 if not
    ///    `Available`).
    /// 2. Find the patient's most-recent active ambulatory encounter
    ///    (404 + AE if none).
    /// 3. Reclassify the encounter to `Inpatient` via
    ///    `EncounterRepository::set_class`.
    /// 4. If the encounter is still `Arrived`, advance to
    ///    `InProgress` via the regular state machine.
    /// 5. Flip the bed to `Occupied`.
    /// 6. Insert `BedAssignment` + `Admission`.
    /// 7. Write audit + outbox `EncounterPromotedToInpatient`.
    pub async fn change_to_inpatient(
        &self,
        patient_id: Uuid,
        bed_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<AdmissionResult> {
        let ctx_clone = ctx.clone();
        let source_owned = source.map(|s| s.to_string());
        let res = self
            .db
            .transaction::<_, AdmissionResult, Error>(|txn| {
                let source_for_txn = source_owned.clone();
                Box::pin(async move {
                    let bed = BedRepository::select_for_update(txn, bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {bed_id}")))?;
                    if bed.status != BedStatus::Available {
                        return Err(Error::conflict(format!(
                            "bed {bed_id} is not available (status={:?})",
                            bed.status
                        )));
                    }
                    let enc = EncounterRepository::find_latest_active_ambulatory_for_patient(
                        txn, patient_id,
                    )
                    .await?
                    .ok_or_else(|| {
                        Error::not_found(format!(
                            "patient {patient_id} has no active ambulatory encounter to promote"
                        ))
                    })?;
                    let enc =
                        EncounterRepository::set_class(txn, enc.id, EncounterClass::Inpatient)
                            .await?;
                    let enc = if enc.status == EncounterStatus::Arrived {
                        EncounterRepository::set_status(txn, enc.id, EncounterStatus::InProgress)
                            .await?
                    } else {
                        enc
                    };
                    BedRepository::update_status(txn, bed_id, BedStatus::Occupied).await?;
                    let ba = BedAssignment::new(enc.id, bed_id);
                    let ba = AdmissionRepository::create_bed_assignment(txn, &ba).await?;
                    let adm = Admission::new(enc.id, bed_id);
                    let adm = AdmissionRepository::create(txn, &adm).await?;
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        "change_to_inpatient",
                        None,
                        Some(serde_json::json!({
                            "admission_id": adm.id,
                            "bed_id": bed_id,
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "encounter_id": enc.id,
                        "patient_id": patient_id,
                        "bed_id": bed_id,
                        "admission_id": adm.id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterPromotedToInpatient", &payload)
                        .await?;
                    Ok(AdmissionResult {
                        encounter: enc,
                        admission: adm,
                        bed_assignment: ba,
                    })
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;
        let mut in_mem = serde_json::json!({
            "encounter_id": res.encounter.id,
            "admission_id": res.admission.id,
            "patient_id": patient_id,
            "bed_id": bed_id,
        });
        if let Some(s) = source_owned.as_deref() {
            in_mem["source"] = serde_json::Value::String(s.to_string());
        }
        let _ = self
            .publisher
            .publish(DomainEvent::new("EncounterPromotedToInpatient", in_mem))
            .await;
        Ok(res)
    }

    /// Demote an inpatient encounter to ambulatory (HL7 v2 ADT^A07
    /// semantics): release the bed assignment, reclassify the
    /// encounter from `Inpatient` to `Outpatient`, leave the
    /// encounter status as `InProgress` (the patient is still in
    /// active care — just no longer admitted to a bed). The
    /// `admissions` row is preserved as historical record.
    ///
    /// Transactional steps:
    /// 1. Load the admission (404 if missing).
    /// 2. Release the active `BedAssignment` (404 if none).
    /// 3. Flip bed → `Cleaning` (regular `Occupied → Cleaning`
    ///    transition, same as discharge).
    /// 4. Reclassify the encounter via `EncounterRepository::set_class`
    ///    to `Outpatient`.
    /// 5. Write audit + outbox `EncounterDemotedToOutpatient`.
    pub async fn change_to_outpatient(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Encounter> {
        let ctx_clone = ctx.clone();
        let source_owned = source.map(|s| s.to_string());
        let res = self
            .db
            .transaction::<_, Encounter, Error>(|txn| {
                let source_for_txn = source_owned.clone();
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;
                    let ba = AdmissionRepository::find_active_by_encounter(txn, adm.encounter_id)
                        .await?
                        .ok_or_else(|| {
                            Error::not_found(format!(
                                "admission {admission_id} has no active bed assignment"
                            ))
                        })?;
                    AdmissionRepository::release_bed_assignment(txn, ba.id).await?;
                    BedRepository::update_status(txn, ba.bed_id, BedStatus::Cleaning).await?;
                    let enc = EncounterRepository::set_class(
                        txn,
                        adm.encounter_id,
                        EncounterClass::Outpatient,
                    )
                    .await?;
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        "change_to_outpatient",
                        None,
                        Some(serde_json::json!({
                            "admission_id": admission_id,
                            "released_bed_id": ba.bed_id,
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "encounter_id": enc.id,
                        "patient_id": enc.patient_id,
                        "admission_id": admission_id,
                        "released_bed_id": ba.bed_id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterDemotedToOutpatient", &payload)
                        .await?;
                    Ok(enc)
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;
        let mut in_mem = serde_json::json!({
            "encounter_id": res.id,
            "patient_id": res.patient_id,
            "admission_id": admission_id,
        });
        if let Some(s) = source_owned.as_deref() {
            in_mem["source"] = serde_json::Value::String(s.to_string());
        }
        let _ = self
            .publisher
            .publish(DomainEvent::new("EncounterDemotedToOutpatient", in_mem))
            .await;
        Ok(res)
    }

    /// Cancel a wrong transfer (HL7 v2 ADT^A12 semantics): restore the
    /// patient to the bed they occupied before the most-recent transfer.
    ///
    /// Transactional steps: load the admission, find the most-recent
    /// `Transfer` row for it (404 if none), release the currently-active
    /// `BedAssignment` (must be on the transfer's `to_bed`), lock the
    /// original `from_bed` (must be `Available` or `Cleaning` — anything
    /// else means another patient now owns it and we can't undo the
    /// transfer), flip the destination bed back to `Cleaning` and the
    /// origin bed back to `Occupied` via the `_unchecked` helpers (the
    /// `Cleaning → Occupied` transition is outside the regular state-
    /// machine flow, same exception documented in spec.md §6.4 for A13),
    /// insert a new active `BedAssignment` on the original bed, delete
    /// the cancelled `Transfer` row (the `transfers` table is
    /// operational, not append-only — same as `discharges` for A13),
    /// write audit + outbox, commit.
    pub async fn cancel_transfer(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Admission> {
        let ctx_clone = ctx.clone();
        let source_owned = source.map(|s| s.to_string());
        let res = self
            .db
            .transaction::<_, Admission, Error>(|txn| {
                let source_for_txn = source_owned.clone();
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;

                    let last = AdmissionRepository::find_latest_transfer_for_admission(
                        txn,
                        admission_id,
                    )
                    .await?
                    .ok_or_else(|| {
                        Error::not_found(format!(
                            "admission {admission_id} has no transfer to cancel"
                        ))
                    })?;
                    let from_bed_id = last.from_bed_id;
                    let to_bed_id = last.to_bed_id;

                    // Release the currently-active assignment (which the
                    // application invariants put on `to_bed_id`).
                    let active = AdmissionRepository::find_active_by_encounter(
                        txn,
                        adm.encounter_id,
                    )
                    .await?
                    .ok_or_else(|| Error::not_found("no active bed assignment"))?;
                    if active.bed_id != to_bed_id {
                        return Err(Error::conflict(format!(
                            "active bed_assignment is on {} but latest transfer landed on {to_bed_id}; refusing to cancel-transfer",
                            active.bed_id
                        )));
                    }

                    // Lock the origin bed. Same constraint as A13: it must
                    // still be Available or Cleaning. Anything else means
                    // someone else is now there.
                    let from_bed = BedRepository::select_for_update(txn, from_bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {from_bed_id}")))?;
                    if !matches!(from_bed.status, BedStatus::Available | BedStatus::Cleaning) {
                        return Err(Error::conflict(format!(
                            "origin bed {from_bed_id} is in status {:?}; cannot cancel transfer",
                            from_bed.status
                        )));
                    }

                    AdmissionRepository::release_bed_assignment(txn, active.id).await?;

                    // Flip beds: to_bed → Cleaning (regular Occupied →
                    // Cleaning is legal but the patient just left so this
                    // is the natural post-state); from_bed → Occupied via
                    // the unchecked helper (Cleaning → Occupied is not in
                    // the regular state-machine flow).
                    BedRepository::update_status(txn, to_bed_id, BedStatus::Cleaning).await?;
                    BedRepository::set_status_unchecked(txn, from_bed_id, BedStatus::Occupied)
                        .await?;

                    let ba = BedAssignment::new(adm.encounter_id, from_bed_id);
                    let _ = AdmissionRepository::create_bed_assignment(txn, &ba).await?;

                    let deleted =
                        AdmissionRepository::delete_transfer(txn, last.id).await?;
                    if deleted == 0 {
                        return Err(Error::not_found(format!(
                            "transfer {} disappeared mid-transaction",
                            last.id
                        )));
                    }

                    AuditLogRepository::log(
                        txn,
                        "admission",
                        admission_id,
                        "cancel_transfer",
                        None,
                        Some(serde_json::json!({
                            "transfer_id": last.id,
                            "from_bed_id": from_bed_id,
                            "to_bed_id": to_bed_id,
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "admission_id": admission_id,
                        "encounter_id": adm.encounter_id,
                        "from_bed_id": from_bed_id,
                        "to_bed_id": to_bed_id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterTransferCancelled", &payload)
                        .await?;

                    Ok(adm)
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;

        let mut in_mem = serde_json::json!({
            "admission_id": admission_id,
            "encounter_id": res.encounter_id,
        });
        if let Some(s) = source_owned.as_deref() {
            in_mem["source"] = serde_json::Value::String(s.to_string());
        }
        let _ = self
            .publisher
            .publish(DomainEvent::new("EncounterTransferCancelled", in_mem))
            .await;
        Ok(res)
    }

    /// Cancel a wrong admission (HL7 v2 ADT^A11 semantics).
    ///
    /// Transactional steps: load the admission, release the active
    /// `BedAssignment` (if any), flip the bed to `Cleaning` (the regular
    /// post-Occupied transition — operators can subsequently mark the bed
    /// `Available` if no cleaning is actually required), force the encounter
    /// to `Cancelled` (allowed from `InProgress` by the regular state
    /// machine), write audit + outbox, commit. The `admissions` row is
    /// preserved so the cancelled admission is still visible in the
    /// patient's history.
    pub async fn cancel_admission(
        &self,
        admission_id: Uuid,
        reason: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Admission> {
        let ctx_clone = ctx.clone();
        let reason_owned = reason.map(|s| s.to_string());
        let res = self
            .db
            .transaction::<_, Admission, Error>(|txn| {
                let reason_for_txn = reason_owned.clone();
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;

                    // Release the active bed assignment (if any) and free
                    // the bed by walking it through Cleaning. We respect
                    // the BedStatus state machine because Occupied → Cleaning
                    // is a legal transition; an operator can mark the bed
                    // Available immediately afterwards if no cleaning is
                    // actually required.
                    let freed_bed_id = if let Some(ba) =
                        AdmissionRepository::find_active_by_encounter(txn, adm.encounter_id).await?
                    {
                        AdmissionRepository::release_bed_assignment(txn, ba.id).await?;
                        BedRepository::update_status(txn, ba.bed_id, BedStatus::Cleaning).await?;
                        Some(ba.bed_id)
                    } else {
                        None
                    };

                    // Move the encounter to Cancelled. This goes through the
                    // regular state machine — Planned/Arrived/InProgress/
                    // OnLeave all permit a transition to Cancelled.
                    EncounterRepository::set_status(
                        txn,
                        adm.encounter_id,
                        EncounterStatus::Cancelled,
                    )
                    .await?;

                    AuditLogRepository::log(
                        txn,
                        "admission",
                        admission_id,
                        "cancel_admit",
                        None,
                        Some(serde_json::to_value(&adm).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "admission_id": admission_id,
                        "encounter_id": adm.encounter_id,
                        "bed_id": freed_bed_id,
                    });
                    if let Some(r) = reason_for_txn.as_deref() {
                        payload["reason"] = serde_json::Value::String(r.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterCancelled", &payload).await?;

                    Ok(adm)
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;

        let mut in_mem = serde_json::json!({
            "admission_id": admission_id,
            "encounter_id": res.encounter_id,
        });
        if let Some(r) = reason_owned.as_deref() {
            in_mem["reason"] = serde_json::Value::String(r.to_string());
        }
        let _ = self
            .publisher
            .publish(DomainEvent::new("EncounterCancelled", in_mem))
            .await;
        Ok(res)
    }

    /// Cancel a wrong discharge (HL7 v2 ADT^A13 semantics): reinstate the
    /// admission, put the patient back in the original bed.
    ///
    /// Transactional steps: load the admission + discharge, locate the
    /// most-recently-released `BedAssignment` for the encounter, lock the
    /// original bed (must be in `Available` or `Cleaning` — anything else
    /// means another patient now owns it and we can't undo the discharge),
    /// delete the `discharges` row, force-flip the bed to `Occupied`,
    /// force-set the encounter back to `InProgress`, insert a new active
    /// `BedAssignment`, write audit + outbox, commit.
    ///
    /// **State-machine bypass.** Both the encounter (`Finished` is normally
    /// terminal) and the bed (`Cleaning → Occupied` is not in the regular
    /// flow) are forced via the `_unchecked` repository helpers; this is
    /// the explicit exception documented in spec.md §6.4. Every other
    /// write path keeps using the state-machine-guarded helpers.
    pub async fn cancel_discharge(
        &self,
        admission_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Admission> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .transaction::<_, Admission, Error>(|txn| {
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;

                    let prior_ba = AdmissionRepository::find_latest_bed_assignment_for_encounter(
                        txn,
                        adm.encounter_id,
                    )
                    .await?
                    .ok_or_else(|| {
                        Error::not_found(format!(
                            "encounter {} has no bed_assignment history",
                            adm.encounter_id
                        ))
                    })?;
                    let bed_id = prior_ba.bed_id;

                    // Lock the original bed. Allowed reinstatement states
                    // are Available (cleaned + ready) or Cleaning (just
                    // released, no one else has taken it). Occupied means
                    // somebody else is now in it; Reserved / OutOfService
                    // mean the bed is otherwise spoken for.
                    let bed = BedRepository::select_for_update(txn, bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {bed_id}")))?;
                    if !matches!(bed.status, BedStatus::Available | BedStatus::Cleaning) {
                        return Err(Error::conflict(format!(
                            "bed {bed_id} is in status {:?}; cannot reinstate admission",
                            bed.status
                        )));
                    }

                    // Physically remove the discharge row. The `discharges`
                    // table is operational (no soft-delete on it per
                    // spec.md §5).
                    let deleted = AdmissionRepository::delete_discharge(txn, admission_id).await?;
                    if deleted == 0 {
                        return Err(Error::not_found(format!(
                            "admission {admission_id} has no discharge to cancel"
                        )));
                    }

                    // Force the bed and encounter back to their pre-discharge
                    // states. Both transitions are outside the regular
                    // state-machine flow.
                    BedRepository::set_status_unchecked(txn, bed_id, BedStatus::Occupied).await?;
                    EncounterRepository::set_status_unchecked(
                        txn,
                        adm.encounter_id,
                        EncounterStatus::InProgress,
                    )
                    .await?;

                    // New active bed_assignment for the original bed. The
                    // partial unique index on bed_assignments(bed_id)
                    // WHERE released_at IS NULL guarantees we can only do
                    // this when no other active assignment exists.
                    let ba = BedAssignment::new(adm.encounter_id, bed_id);
                    let _ = AdmissionRepository::create_bed_assignment(txn, &ba).await?;

                    AuditLogRepository::log(
                        txn,
                        "admission",
                        admission_id,
                        "cancel_discharge",
                        None,
                        Some(serde_json::to_value(&adm).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "EncounterDischargeCancelled",
                        &serde_json::json!({
                            "admission_id": admission_id,
                            "encounter_id": adm.encounter_id,
                            "bed_id": bed_id,
                            "reason": "hl7v2_a13",
                        }),
                    )
                    .await?;

                    Ok(adm)
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;

        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "EncounterDischargeCancelled",
                serde_json::json!({
                    "admission_id": admission_id,
                    "encounter_id": res.encounter_id,
                }),
            ))
            .await;
        Ok(res)
    }

    /// Discharge an admitted patient.
    ///
    /// Transactional steps: finish the encounter, release the active bed
    /// assignment (if any), flip the bed to `Cleaning`, insert the
    /// `Discharge` record, and write audit and outbox rows.
    pub async fn discharge(&self, admission_id: Uuid, ctx: &UserContext) -> Result<Discharge> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .transaction::<_, Discharge, Error>(|txn| {
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;

                    // End the encounter.
                    EncounterRepository::set_status(
                        txn,
                        adm.encounter_id,
                        EncounterStatus::Finished,
                    )
                    .await?;

                    // Release the active bed assignment (if any) and free
                    // the bed.
                    if let Some(ba) =
                        AdmissionRepository::find_active_by_encounter(txn, adm.encounter_id).await?
                    {
                        AdmissionRepository::release_bed_assignment(txn, ba.id).await?;
                        BedRepository::update_status(txn, ba.bed_id, BedStatus::Cleaning).await?;
                    }

                    let d = Discharge::new(admission_id);
                    let d = AdmissionRepository::create_discharge(txn, &d).await?;

                    AuditLogRepository::log(
                        txn,
                        "admission",
                        admission_id,
                        "discharge",
                        None,
                        Some(serde_json::to_value(&d).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "EncounterDischarged",
                        &serde_json::json!({
                            "admission_id": admission_id,
                            "encounter_id": adm.encounter_id,
                        }),
                    )
                    .await?;

                    Ok(d)
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;

        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "EncounterDischarged",
                serde_json::json!({ "admission_id": admission_id }),
            ))
            .await;
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The admit() service drives the encounter through the
    /// Planned → Arrived → InProgress state machine. The pure transition
    /// logic is verified here; DB-touching behavior is covered by
    /// integration tests.
    #[test]
    fn test_encounter_admitted_status() {
        let mut enc = Encounter::new(Uuid::new_v4(), EncounterClass::Inpatient);
        assert_eq!(enc.status, EncounterStatus::Planned);
        enc.status = enc
            .status
            .try_transition_to(EncounterStatus::Arrived)
            .expect("planned -> arrived");
        assert_eq!(enc.status, EncounterStatus::Arrived);
        enc.status = enc
            .status
            .try_transition_to(EncounterStatus::InProgress)
            .expect("arrived -> in_progress");
        assert_eq!(enc.status, EncounterStatus::InProgress);
    }
}
