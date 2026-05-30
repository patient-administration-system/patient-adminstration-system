//! HL7 v2 MLLP outbound publisher.
//!
//! Implements [`EventPublisher`] by connecting to a configured TCP peer over
//! MLLP and sending an ADT message for each interesting outbox event:
//!
//! - `EncounterAdmitted`           → `ADT^A01`
//! - `EncounterTransferred`        → `ADT^A02`
//! - `EncounterDischarged`         → `ADT^A03`
//! - `EncounterRegistered`         → `ADT^A04` (v0.29; skipped when source == "hl7v2_a04")
//! - `EncounterPreAdmitted`        → `ADT^A05` (v0.33; skipped when source == "hl7v2_a05")
//! - `EncounterPromotedToInpatient` → `ADT^A06` (v0.42; skipped when source == "hl7v2_a06")
//! - `EncounterPreAdmitCancelled`  → `ADT^A38` (v0.35; skipped when source == "hl7v2_a38")
//! - `PatientDeleted`              → `ADT^A23` (v0.40; skipped when source == "hl7v2_a23")
//! - `PatientUpdated`              → `ADT^A08` (only when the source is the HL7 v2 surface)
//! - `EncounterCancelled`          → `ADT^A11` (v0.38 retrofit; skipped when reason == "hl7v2_a11")
//! - `EncounterTransferCancelled`  → `ADT^A12` (v0.31; skipped when source == "hl7v2_a12")
//! - `EncounterLeaveStarted`       → `ADT^A21` (v0.37; skipped when source == "hl7v2_a21")
//! - `EncounterLeaveEnded`         → `ADT^A22` (v0.37; skipped when source == "hl7v2_a22")
//! - `EncounterDischargeCancelled` → `ADT^A13`
//! - `PatientMerged`               → `ADT^A40` (v0.18; skipped when source == "hl7v2_a40")
//! - `ChargePosted`                → `DFT^P03` (v0.19; skipped when source == "hl7v2_p03")
//! - `PractitionerCreated`         → `MFN^M02` MAD (v0.25; skipped when source == "hl7v2_mfn_m02")
//! - `PractitionerUpdated`         → `MFN^M02` MUP (v0.25; skipped when source == "hl7v2_mfn_m02")
//! - `PractitionerDeactivated`     → `MFN^M02` MDL (v0.25; skipped when source == "hl7v2_mfn_m02")
//! - `BedCreated`                  → `MFN^M05` MAD (v0.27; skipped when source == "hl7v2_mfn_m05")
//! - `BedUpdated`                  → `MFN^M05` MUP (v0.27; skipped when source == "hl7v2_mfn_m05")
//! - `BedRetired`                  → `MFN^M05` MDL (v0.27; skipped when source == "hl7v2_mfn_m05")
//! - `AppointmentBooked`           → `SIU^S12` (v0.16; skipped when source == "hl7v2_s12")
//! - `AppointmentRescheduled`      → `SIU^S13` (v0.17; skipped when source == "hl7v2_s13")
//! - `AppointmentModified`         → `SIU^S14` (v0.17; skipped when source == "hl7v2_s14")
//! - `AppointmentCancelled`        → `SIU^S15` (v0.16; skipped when source == "hl7v2_s15")
//!
//! Every other event type returns `Ok(())` (silently dropped) so the outbox
//! dispatcher can still mark the row published. The PAS outbox is the
//! durable record; this publisher is a fan-out transport.
//!
//! Connection strategy: one fresh TCP connection per event. Cheap to reason
//! about and gives the dispatcher's existing retry loop the right behavior
//! (a failed publish stays unpublished and is retried next tick). Production
//! deployments that need higher throughput should pool connections.

use std::time::Duration;

use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use serde::Deserialize;
use tokio::net::TcpStream;
use uuid::Uuid;

use crate::db::repositories::admission::AdmissionRepository;
use crate::db::repositories::bed::BedRepository;
use crate::db::repositories::patient::PatientRepository;
use crate::hl7v2::{
    MfnM02Item, MfnM05Item, encode_adt_a01, encode_adt_a02, encode_adt_a03, encode_adt_a04,
    encode_adt_a05, encode_adt_a06, encode_adt_a08, encode_adt_a11, encode_adt_a12, encode_adt_a13,
    encode_adt_a21, encode_adt_a22, encode_adt_a23, encode_adt_a38, encode_adt_a40, encode_dft_p03,
    encode_mfn_m02, encode_mfn_m05, encode_siu_s12, encode_siu_s13, encode_siu_s14, encode_siu_s15,
    mllp,
};
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

/// Connect timeout for the outbound peer.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Read timeout for the inbound ACK frame.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Outbound MLLP publisher.
pub struct Hl7v2MllpPublisher {
    db: DatabaseConnection,
    peer: String,
    sending_app: String,
    receiving_app: String,
}

impl Hl7v2MllpPublisher {
    /// Build a new publisher.
    ///
    /// * `peer` — `host:port` of the destination MLLP receiver (typically the EMR).
    /// * `sending_app` — string that lands in MSH-3 of outbound messages.
    /// * `receiving_app` — string that lands in MSH-5 of outbound messages.
    pub fn new(
        db: DatabaseConnection,
        peer: impl Into<String>,
        sending_app: impl Into<String>,
        receiving_app: impl Into<String>,
    ) -> Self {
        Self {
            db,
            peer: peer.into(),
            sending_app: sending_app.into(),
            receiving_app: receiving_app.into(),
        }
    }

    async fn send_frame(&self, body: &str, event_id: Uuid) -> Result<()> {
        let mut sock = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&self.peer))
            .await
            .map_err(|_| Error::Streaming(format!("MLLP connect to {} timed out", self.peer)))?
            .map_err(|e| Error::Streaming(format!("MLLP connect {}: {e}", self.peer)))?;
        let (mut rd, mut wr) = sock.split();
        mllp::write_frame(&mut wr, body.as_bytes()).await?;
        let ack_payload = tokio::time::timeout(READ_TIMEOUT, mllp::read_frame(&mut rd))
            .await
            .map_err(|_| Error::Streaming(format!("MLLP ack read from {} timed out", self.peer)))??
            .ok_or_else(|| {
                Error::Streaming(format!(
                    "peer {} closed connection without ACK for event {event_id}",
                    self.peer
                ))
            })?;
        let ack_str = String::from_utf8_lossy(&ack_payload);
        if !ack_contains_code(&ack_str, "AA") {
            return Err(Error::Streaming(format!(
                "peer {} returned non-AA ACK for event {event_id}: {}",
                self.peer,
                ack_str.lines().nth(1).unwrap_or("")
            )));
        }
        Ok(())
    }

    /// Resolve the admission → encounter → patient chain for an LOA
    /// outbox event (`EncounterLeaveStarted` → A21,
    /// `EncounterLeaveEnded` → A22) and send the matching frame.
    /// Source-gated on the supplied tag so v0.36 inbound events
    /// don't echo back. (*v0.37*)
    async fn emit_loa(&self, event: &DomainEvent, which: &str, source_tag: &str) -> Result<()> {
        let payload: EncounterLoaPayload = serde_json::from_value(event.payload.clone())
            .map_err(|e| Error::Streaming(format!("decode LOA payload: {e}")))?;
        if payload.source.as_deref() == Some(source_tag) {
            return Ok(());
        }
        let adm = AdmissionRepository::find_by_id(&self.db, payload.admission_id)
            .await?
            .ok_or_else(|| {
                Error::Streaming(format!(
                    "admission {} not found for outbound ADT^{which}",
                    payload.admission_id
                ))
            })?;
        let encounter = crate::db::repositories::encounter::EncounterRepository::find_by_id(
            &self.db,
            adm.encounter_id,
        )
        .await?
        .ok_or_else(|| {
            Error::Streaming(format!(
                "encounter {} not found for outbound ADT^{which}",
                adm.encounter_id
            ))
        })?;
        let patient = PatientRepository::find_by_id(&self.db, encounter.patient_id)
            .await?
            .ok_or_else(|| {
                Error::Streaming(format!(
                    "patient {} not found for outbound ADT^{which}",
                    encounter.patient_id
                ))
            })?;
        let msg_id = format!("PAS-{}", event.id.simple());
        let body = match which {
            "A21" => encode_adt_a21(&patient, &self.sending_app, &self.receiving_app, &msg_id),
            "A22" => encode_adt_a22(&patient, &self.sending_app, &self.receiving_app, &msg_id),
            _ => {
                return Err(Error::Streaming(format!(
                    "emit_loa: unsupported which={which}"
                )));
            }
        };
        self.send_frame(&body, event.id).await
    }

    /// Build and send a single-item `MFN^M02` for one practitioner
    /// outbox event (`PractitionerCreated` → `MAD`, `Updated` → `MUP`,
    /// `Deactivated` → `MDL`). Source-gated on `hl7v2_mfn_m02` so
    /// EMR-originated changes don't echo back. The MFN primary key
    /// is the practitioner's existing `urn:hl7v2:staff:id`
    /// identifier when present (so the EMR sees the id it sent us),
    /// otherwise the PAS UUID as a fallback.
    async fn emit_practitioner_mfn(&self, event: &DomainEvent, mfe_event_code: &str) -> Result<()> {
        use crate::db::repositories::practitioner::PractitionerRepository;
        use crate::models::identifier::IdentifierType;
        let payload: PractitionerEventPayload = serde_json::from_value(event.payload.clone())
            .map_err(|e| Error::Streaming(format!("decode practitioner payload: {e}")))?;
        // Boomerang protection: skip events that originated on the
        // inbound MFN path.
        if payload.source.as_deref() == Some("hl7v2_mfn_m02") {
            return Ok(());
        }
        let p = PractitionerRepository::find_by_id(&self.db, payload.practitioner_id)
            .await?
            .ok_or_else(|| {
                Error::Streaming(format!(
                    "practitioner {} not found for outbound MFN^M02",
                    payload.practitioner_id
                ))
            })?;
        // Prefer the EMR-issued staff id when the row carries one;
        // otherwise fall back to the PAS UUID. Either way the
        // receiver gets a stable, round-trippable handle.
        let primary_key = p
            .identifiers
            .iter()
            .find(|i| {
                i.identifier_type == IdentifierType::Other && i.system == "urn:hl7v2:staff:id"
            })
            .map(|i| i.value.clone())
            .unwrap_or_else(|| p.id.to_string());
        let item = MfnM02Item {
            event_code: mfe_event_code.to_string(),
            primary_key,
            name: p.name.clone(),
            gender: p.gender,
            birth_date: p.birth_date,
            active: p.active,
        };
        let msg_id = format!("PAS-{}", event.id.simple());
        let body = encode_mfn_m02(
            std::slice::from_ref(&item),
            &self.sending_app,
            &self.receiving_app,
            &msg_id,
        );
        self.send_frame(&body, event.id).await
    }

    /// Build and send a single-item `MFN^M05` for one bed outbox
    /// event (`BedCreated` → `MAD`, `BedUpdated` → `MUP`,
    /// `BedRetired` → `MDL`). Source-gated on `hl7v2_mfn_m05` so
    /// the v0.26 inbound MFN path doesn't echo straight back.
    /// LOC-1.1 = parent room code, LOC-1.3 = bed code, LOC-2 =
    /// bed display name — the same shape the v0.26 inbound parser
    /// expects, so the contract round-trips.
    async fn emit_bed_mfn(&self, event: &DomainEvent, mfe_event_code: &str) -> Result<()> {
        use crate::db::entities::room;
        use sea_orm::EntityTrait;
        let payload: BedEventPayload = serde_json::from_value(event.payload.clone())
            .map_err(|e| Error::Streaming(format!("decode bed payload: {e}")))?;
        if payload.source.as_deref() == Some("hl7v2_mfn_m05") {
            return Ok(());
        }
        let bed = BedRepository::find_by_id(&self.db, payload.bed_id)
            .await?
            .ok_or_else(|| {
                Error::Streaming(format!(
                    "bed {} not found for outbound MFN^M05",
                    payload.bed_id
                ))
            })?;
        let room = room::Entity::find_by_id(bed.room_id)
            .one(&self.db)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| {
                Error::Streaming(format!(
                    "room {} not found for outbound MFN^M05 (bed {})",
                    bed.room_id, bed.id
                ))
            })?;
        let item = MfnM05Item {
            event_code: mfe_event_code.to_string(),
            room_code: Some(room.code.clone()),
            bed_code: bed.code.clone(),
            name: Some(bed.name.clone()),
        };
        let msg_id = format!("PAS-{}", event.id.simple());
        let body = encode_mfn_m05(
            std::slice::from_ref(&item),
            &self.sending_app,
            &self.receiving_app,
            &msg_id,
        );
        self.send_frame(&body, event.id).await
    }
}

fn ack_contains_code(ack: &str, code: &str) -> bool {
    // Look for an MSA segment whose MSA-1 (the code field) matches.
    for line in ack.split('\r') {
        if let Some(rest) = line.strip_prefix("MSA|") {
            let first = rest.split('|').next().unwrap_or("");
            if first == code {
                return true;
            }
        }
    }
    false
}

#[derive(Debug, Deserialize)]
struct AdmittedPayload {
    patient_id: Uuid,
    bed_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct TransferredPayload {
    patient_id: Uuid,
    new_bed_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct DischargedPayload {
    patient_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct PatientUpdatedPayload {
    patient_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EncounterCancelledPayload {
    admission_id: Uuid,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EncounterDischargeCancelledPayload {
    admission_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct AppointmentBookedPayload {
    appointment_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AppointmentCancelledPayload {
    appointment_id: Uuid,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AppointmentRescheduledPayload {
    appointment_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChargePostedPayload {
    charge_id: Uuid,
    #[serde(default)]
    patient_id: Option<Uuid>,
    // `account_id` is intentionally ignored on the outbound path —
    // we hop through the charge row to resolve the patient when
    // `patient_id` isn't present. The field is still accepted so
    // existing payload shapes deserialize cleanly.
    #[serde(default, rename = "account_id")]
    _account_id: Option<Uuid>,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PatientMergedPayload {
    source_id: Uuid,
    target_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PractitionerEventPayload {
    practitioner_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BedEventPayload {
    bed_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EncounterRegisteredPayload {
    patient_id: Uuid,
    class: String,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EncounterPromotedToInpatientPayload {
    patient_id: Uuid,
    bed_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EncounterPreAdmittedPayload {
    patient_id: Uuid,
    bed_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PatientDeletedPayload {
    patient_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EncounterPreAdmitCancelledPayload {
    patient_id: Uuid,
    bed_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EncounterLoaPayload {
    admission_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EncounterTransferCancelledPayload {
    admission_id: Uuid,
    from_bed_id: Uuid,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AppointmentModifiedPayload {
    appointment_id: Uuid,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[async_trait]
impl EventPublisher for Hl7v2MllpPublisher {
    async fn publish(&self, event: DomainEvent) -> Result<()> {
        let msg_id = format!("PAS-{}", event.id.simple());
        match event.event_type.as_str() {
            "EncounterAdmitted" => {
                let payload: AdmittedPayload = serde_json::from_value(event.payload.clone())
                    .map_err(|e| {
                        Error::Streaming(format!("decode EncounterAdmitted payload: {e}"))
                    })?;
                let patient = PatientRepository::find_by_id(&self.db, payload.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A01",
                            payload.patient_id
                        ))
                    })?;
                let bed = BedRepository::find_by_id(&self.db, payload.bed_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "bed {} not found for outbound ADT^A01",
                            payload.bed_id
                        ))
                    })?;
                let body = encode_adt_a01(
                    &patient,
                    &bed,
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "EncounterTransferred" => {
                let payload: TransferredPayload = serde_json::from_value(event.payload.clone())
                    .map_err(|e| {
                        Error::Streaming(format!("decode EncounterTransferred payload: {e}"))
                    })?;
                let patient = PatientRepository::find_by_id(&self.db, payload.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A02",
                            payload.patient_id
                        ))
                    })?;
                let bed = BedRepository::find_by_id(&self.db, payload.new_bed_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "new_bed {} not found for outbound ADT^A02",
                            payload.new_bed_id
                        ))
                    })?;
                let body = encode_adt_a02(
                    &patient,
                    &bed,
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "EncounterRegistered" => {
                let payload: EncounterRegisteredPayload =
                    serde_json::from_value(event.payload.clone()).map_err(|e| {
                        Error::Streaming(format!("decode EncounterRegistered payload: {e}"))
                    })?;
                // Boomerang protection: skip events that came in via
                // the v0.28 inbound ADT^A04 path.
                if payload.source.as_deref() == Some("hl7v2_a04") {
                    return Ok(());
                }
                let patient = PatientRepository::find_by_id(&self.db, payload.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A04",
                            payload.patient_id
                        ))
                    })?;
                // Map the EncounterClass enum debug form to the PV1-2
                // patient-class code. Emergency / Outpatient are the
                // canonical A04 classes; anything else falls back to
                // "O" (outpatient) since A04 by definition isn't
                // inpatient.
                let class_code = match payload.class.as_str() {
                    "Emergency" => "E",
                    _ => "O",
                };
                let body = encode_adt_a04(
                    &patient,
                    class_code,
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "EncounterDischarged" => {
                let payload: DischargedPayload = serde_json::from_value(event.payload.clone())
                    .map_err(|e| {
                        Error::Streaming(format!("decode EncounterDischarged payload: {e}"))
                    })?;
                let patient = PatientRepository::find_by_id(&self.db, payload.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A03",
                            payload.patient_id
                        ))
                    })?;
                let body =
                    encode_adt_a03(&patient, &self.sending_app, &self.receiving_app, &msg_id);
                self.send_frame(&body, event.id).await
            }
            "PatientUpdated" => {
                let payload: PatientUpdatedPayload = serde_json::from_value(event.payload.clone())
                    .map_err(|e| Error::Streaming(format!("decode PatientUpdated payload: {e}")))?;
                // Only relay updates that originated on the HL7 v2 surface.
                // REST-driven patient edits land in the outbox too (as of v0.4)
                // but they don't carry the source tag, and we don't want to
                // boomerang those back to an EMR that may have been the
                // ultimate source.
                if payload.source.as_deref() != Some("hl7v2_a08") {
                    return Ok(());
                }
                let patient = PatientRepository::find_by_id(&self.db, payload.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A08",
                            payload.patient_id
                        ))
                    })?;
                let body =
                    encode_adt_a08(&patient, &self.sending_app, &self.receiving_app, &msg_id);
                self.send_frame(&body, event.id).await
            }
            "EncounterCancelled" => {
                let payload: EncounterCancelledPayload =
                    serde_json::from_value(event.payload.clone()).map_err(|e| {
                        Error::Streaming(format!("decode EncounterCancelled payload: {e}"))
                    })?;
                // Boomerang protection: skip events that came in via
                // the v0.4 inbound ADT^A11 path (v0.38 retrofit).
                // REST-driven encounter cancellations now DO echo
                // downstream — same pattern as the rest of the
                // outbound matchers from v0.25 onwards.
                if payload.reason.as_deref() == Some("hl7v2_a11") {
                    return Ok(());
                }
                let adm = AdmissionRepository::find_by_id(&self.db, payload.admission_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "admission {} not found for outbound ADT^A11",
                            payload.admission_id
                        ))
                    })?;
                let encounter =
                    crate::db::repositories::encounter::EncounterRepository::find_by_id(
                        &self.db,
                        adm.encounter_id,
                    )
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "encounter {} not found for outbound ADT^A11",
                            adm.encounter_id
                        ))
                    })?;
                let patient = PatientRepository::find_by_id(&self.db, encounter.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A11",
                            encounter.patient_id
                        ))
                    })?;
                let body =
                    encode_adt_a11(&patient, &self.sending_app, &self.receiving_app, &msg_id);
                self.send_frame(&body, event.id).await
            }
            "EncounterPreAdmitted" => {
                let payload: EncounterPreAdmittedPayload =
                    serde_json::from_value(event.payload.clone()).map_err(|e| {
                        Error::Streaming(format!("decode EncounterPreAdmitted payload: {e}"))
                    })?;
                // Boomerang protection: skip events that came in via
                // the v0.32 inbound ADT^A05 path.
                if payload.source.as_deref() == Some("hl7v2_a05") {
                    return Ok(());
                }
                let patient = PatientRepository::find_by_id(&self.db, payload.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A05",
                            payload.patient_id
                        ))
                    })?;
                let bed = BedRepository::find_by_id(&self.db, payload.bed_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "bed {} not found for outbound ADT^A05",
                            payload.bed_id
                        ))
                    })?;
                let body = encode_adt_a05(
                    &patient,
                    &bed,
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "EncounterPromotedToInpatient" => {
                let payload: EncounterPromotedToInpatientPayload =
                    serde_json::from_value(event.payload.clone()).map_err(|e| {
                        Error::Streaming(format!(
                            "decode EncounterPromotedToInpatient payload: {e}"
                        ))
                    })?;
                if payload.source.as_deref() == Some("hl7v2_a06") {
                    return Ok(());
                }
                let patient = PatientRepository::find_by_id(&self.db, payload.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A06",
                            payload.patient_id
                        ))
                    })?;
                let bed = BedRepository::find_by_id(&self.db, payload.bed_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "bed {} not found for outbound ADT^A06",
                            payload.bed_id
                        ))
                    })?;
                let body = encode_adt_a06(
                    &patient,
                    &bed,
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "PatientDeleted" => {
                let payload: PatientDeletedPayload = serde_json::from_value(event.payload.clone())
                    .map_err(|e| Error::Streaming(format!("decode PatientDeleted payload: {e}")))?;
                // Boomerang protection: skip events that came in via
                // the v0.39 inbound ADT^A23 path.
                if payload.source.as_deref() == Some("hl7v2_a23") {
                    return Ok(());
                }
                // The patient row has been soft-deleted by the time we
                // reach this match arm (the outbox dispatcher runs after
                // commit), but `find_by_id` returns rows by primary key
                // without filtering on `deleted_at`, so the lookup still
                // succeeds and we can encode the original PID.
                let patient = PatientRepository::find_by_id(&self.db, payload.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A23",
                            payload.patient_id
                        ))
                    })?;
                let body =
                    encode_adt_a23(&patient, &self.sending_app, &self.receiving_app, &msg_id);
                self.send_frame(&body, event.id).await
            }
            "EncounterLeaveStarted" => self.emit_loa(&event, "A21", "hl7v2_a21").await,
            "EncounterLeaveEnded" => self.emit_loa(&event, "A22", "hl7v2_a22").await,
            "EncounterPreAdmitCancelled" => {
                let payload: EncounterPreAdmitCancelledPayload =
                    serde_json::from_value(event.payload.clone()).map_err(|e| {
                        Error::Streaming(format!("decode EncounterPreAdmitCancelled payload: {e}"))
                    })?;
                // Boomerang protection: skip events that came in via
                // the v0.34 inbound ADT^A38 path.
                if payload.source.as_deref() == Some("hl7v2_a38") {
                    return Ok(());
                }
                let patient = PatientRepository::find_by_id(&self.db, payload.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A38",
                            payload.patient_id
                        ))
                    })?;
                let bed = BedRepository::find_by_id(&self.db, payload.bed_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "bed {} not found for outbound ADT^A38",
                            payload.bed_id
                        ))
                    })?;
                let body = encode_adt_a38(
                    &patient,
                    &bed,
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "EncounterTransferCancelled" => {
                let payload: EncounterTransferCancelledPayload =
                    serde_json::from_value(event.payload.clone()).map_err(|e| {
                        Error::Streaming(format!("decode EncounterTransferCancelled payload: {e}"))
                    })?;
                // Boomerang protection: skip events that came in via
                // the v0.30 inbound ADT^A12 path.
                if payload.source.as_deref() == Some("hl7v2_a12") {
                    return Ok(());
                }
                let adm = AdmissionRepository::find_by_id(&self.db, payload.admission_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "admission {} not found for outbound ADT^A12",
                            payload.admission_id
                        ))
                    })?;
                let encounter =
                    crate::db::repositories::encounter::EncounterRepository::find_by_id(
                        &self.db,
                        adm.encounter_id,
                    )
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "encounter {} not found for outbound ADT^A12",
                            adm.encounter_id
                        ))
                    })?;
                let patient = PatientRepository::find_by_id(&self.db, encounter.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A12",
                            encounter.patient_id
                        ))
                    })?;
                let origin_bed = BedRepository::find_by_id(&self.db, payload.from_bed_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "origin bed {} not found for outbound ADT^A12",
                            payload.from_bed_id
                        ))
                    })?;
                let body = encode_adt_a12(
                    &patient,
                    &origin_bed,
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "EncounterDischargeCancelled" => {
                let payload: EncounterDischargeCancelledPayload =
                    serde_json::from_value(event.payload.clone()).map_err(|e| {
                        Error::Streaming(format!("decode EncounterDischargeCancelled payload: {e}"))
                    })?;
                let adm = AdmissionRepository::find_by_id(&self.db, payload.admission_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "admission {} not found for outbound ADT^A13",
                            payload.admission_id
                        ))
                    })?;
                let encounter =
                    crate::db::repositories::encounter::EncounterRepository::find_by_id(
                        &self.db,
                        adm.encounter_id,
                    )
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "encounter {} not found for outbound ADT^A13",
                            adm.encounter_id
                        ))
                    })?;
                let patient = PatientRepository::find_by_id(&self.db, encounter.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound ADT^A13",
                            encounter.patient_id
                        ))
                    })?;
                let bed = BedRepository::find_by_id(&self.db, adm.bed_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "bed {} not found for outbound ADT^A13",
                            adm.bed_id
                        ))
                    })?;
                let body = encode_adt_a13(
                    &patient,
                    &bed,
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "PractitionerCreated" => self.emit_practitioner_mfn(&event, "MAD").await,
            "PractitionerUpdated" => self.emit_practitioner_mfn(&event, "MUP").await,
            "PractitionerDeactivated" => self.emit_practitioner_mfn(&event, "MDL").await,
            "BedCreated" => self.emit_bed_mfn(&event, "MAD").await,
            "BedUpdated" => self.emit_bed_mfn(&event, "MUP").await,
            "BedRetired" => self.emit_bed_mfn(&event, "MDL").await,
            "AppointmentBooked" => {
                let payload: AppointmentBookedPayload =
                    serde_json::from_value(event.payload.clone()).map_err(|e| {
                        Error::Streaming(format!("decode AppointmentBooked payload: {e}"))
                    })?;
                // Don't echo back appointments that we just received from
                // an HL7 v2 peer (boomerang protection). REST / FHIR /
                // SchedulingService bookings have no `source` tag and DO
                // get relayed.
                if payload.source.as_deref() == Some("hl7v2_s12") {
                    return Ok(());
                }
                let appt = crate::db::repositories::appointment::AppointmentRepository::find_by_id(
                    &self.db,
                    payload.appointment_id,
                )
                .await?
                .ok_or_else(|| {
                    Error::Streaming(format!(
                        "appointment {} not found for outbound SIU^S12",
                        payload.appointment_id
                    ))
                })?;
                let patient = PatientRepository::find_by_id(&self.db, appt.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound SIU^S12",
                            appt.patient_id
                        ))
                    })?;
                let body = encode_siu_s12(
                    &patient,
                    appt.id,
                    None,
                    appt.start_datetime,
                    appt.end_datetime,
                    appt.reason.as_deref(),
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "AppointmentCancelled" => {
                let payload: AppointmentCancelledPayload =
                    serde_json::from_value(event.payload.clone()).map_err(|e| {
                        Error::Streaming(format!("decode AppointmentCancelled payload: {e}"))
                    })?;
                // Same boomerang rule as AppointmentBooked.
                if payload.source.as_deref() == Some("hl7v2_s15") {
                    return Ok(());
                }
                let appt = crate::db::repositories::appointment::AppointmentRepository::find_by_id(
                    &self.db,
                    payload.appointment_id,
                )
                .await?
                .ok_or_else(|| {
                    Error::Streaming(format!(
                        "appointment {} not found for outbound SIU^S15",
                        payload.appointment_id
                    ))
                })?;
                let patient = PatientRepository::find_by_id(&self.db, appt.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound SIU^S15",
                            appt.patient_id
                        ))
                    })?;
                let body = encode_siu_s15(
                    &patient,
                    appt.id,
                    None,
                    payload.reason.as_deref(),
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "ChargePosted" => {
                let payload: ChargePostedPayload = serde_json::from_value(event.payload.clone())
                    .map_err(|e| Error::Streaming(format!("decode ChargePosted payload: {e}")))?;
                if payload.source.as_deref() == Some("hl7v2_p03") {
                    return Ok(());
                }
                let charge =
                    crate::db::repositories::billing::BillingRepository::find_charge_by_id(
                        &self.db,
                        payload.charge_id,
                    )
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "charge {} not found for outbound DFT^P03",
                            payload.charge_id
                        ))
                    })?;
                // patient_id may arrive in the payload (HL7 v2 inbound)
                // or have to be resolved through the account (REST
                // post_charge / BillingService::post_charge writes a
                // payload without patient_id).
                let patient_id = match payload.patient_id {
                    Some(pid) => pid,
                    None => {
                        let account =
                            crate::db::repositories::billing::BillingRepository::find_account_by_id(
                                &self.db,
                                charge.account_id,
                            )
                            .await?
                            .ok_or_else(|| {
                                Error::Streaming(format!(
                                    "account {} not found for outbound DFT^P03",
                                    charge.account_id
                                ))
                            })?;
                        account.patient_id
                    }
                };
                let patient = PatientRepository::find_by_id(&self.db, patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {patient_id} not found for outbound DFT^P03"
                        ))
                    })?;
                let body = encode_dft_p03(
                    &patient,
                    &charge,
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "PatientMerged" => {
                let payload: PatientMergedPayload = serde_json::from_value(event.payload.clone())
                    .map_err(|e| {
                    Error::Streaming(format!("decode PatientMerged payload: {e}"))
                })?;
                // Skip the boomerang: don't echo back a merge we just
                // received from this EMR (`source = "hl7v2_a40"`).
                if payload.source.as_deref() == Some("hl7v2_a40") {
                    return Ok(());
                }
                let survivor = PatientRepository::find_by_id(&self.db, payload.target_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "survivor patient {} not found for outbound ADT^A40",
                            payload.target_id
                        ))
                    })?;
                let source = PatientRepository::find_by_id(&self.db, payload.source_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "source patient {} not found for outbound ADT^A40",
                            payload.source_id
                        ))
                    })?;
                let body = encode_adt_a40(
                    &survivor,
                    &source,
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "AppointmentRescheduled" => {
                let payload: AppointmentRescheduledPayload =
                    serde_json::from_value(event.payload.clone()).map_err(|e| {
                        Error::Streaming(format!("decode AppointmentRescheduled payload: {e}"))
                    })?;
                if payload.source.as_deref() == Some("hl7v2_s13") {
                    return Ok(());
                }
                let appt = crate::db::repositories::appointment::AppointmentRepository::find_by_id(
                    &self.db,
                    payload.appointment_id,
                )
                .await?
                .ok_or_else(|| {
                    Error::Streaming(format!(
                        "appointment {} not found for outbound SIU^S13",
                        payload.appointment_id
                    ))
                })?;
                let patient = PatientRepository::find_by_id(&self.db, appt.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound SIU^S13",
                            appt.patient_id
                        ))
                    })?;
                let body = encode_siu_s13(
                    &patient,
                    appt.id,
                    None,
                    appt.start_datetime,
                    appt.end_datetime,
                    appt.reason.as_deref(),
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            "AppointmentModified" => {
                let payload: AppointmentModifiedPayload =
                    serde_json::from_value(event.payload.clone()).map_err(|e| {
                        Error::Streaming(format!("decode AppointmentModified payload: {e}"))
                    })?;
                if payload.source.as_deref() == Some("hl7v2_s14") {
                    return Ok(());
                }
                let appt = crate::db::repositories::appointment::AppointmentRepository::find_by_id(
                    &self.db,
                    payload.appointment_id,
                )
                .await?
                .ok_or_else(|| {
                    Error::Streaming(format!(
                        "appointment {} not found for outbound SIU^S14",
                        payload.appointment_id
                    ))
                })?;
                let patient = PatientRepository::find_by_id(&self.db, appt.patient_id)
                    .await?
                    .ok_or_else(|| {
                        Error::Streaming(format!(
                            "patient {} not found for outbound SIU^S14",
                            appt.patient_id
                        ))
                    })?;
                let body = encode_siu_s14(
                    &patient,
                    appt.id,
                    None,
                    payload.reason.as_deref().or(appt.reason.as_deref()),
                    &self.sending_app,
                    &self.receiving_app,
                    &msg_id,
                );
                self.send_frame(&body, event.id).await
            }
            _ => Ok(()), // silently drop non-ADT events
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ack_contains_code_matches_aa() {
        let ack = "MSH|^~\\&|PAS|FAC|EMR|FAC|20260523120000||ACK|MSG1|P|2.5\r\
MSA|AA|MSG1\r";
        assert!(ack_contains_code(ack, "AA"));
        assert!(!ack_contains_code(ack, "AE"));
    }

    #[test]
    fn test_ack_contains_code_matches_ae() {
        let ack = "MSH|^~\\&|PAS|FAC|EMR|FAC|20260523120000||ACK|MSG2|P|2.5\r\
MSA|AE|MSG2|something broken\r";
        assert!(ack_contains_code(ack, "AE"));
        assert!(!ack_contains_code(ack, "AA"));
    }

    #[test]
    fn test_ack_contains_code_handles_missing_msa() {
        let ack = "MSH|^~\\&|PAS|FAC|EMR|FAC|20260523120000||ACK|MSG3|P|2.5\r";
        assert!(!ack_contains_code(ack, "AA"));
    }
}
