//! FHIR R5 HTTP handlers.
//!
//! v0.1 surface: read-only GETs for `Patient`, `Encounter`, and `Appointment`.
//! Write endpoints (POST/PUT/DELETE) are intentionally deferred. Errors are
//! reported as FHIR `OperationOutcome` JSON bodies, with HTTP status mapped
//! from the PAS [`crate::Error`] variant.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::bed::BedRepository;
use crate::db::repositories::coverage::CoverageRepository;
use crate::db::repositories::encounter::EncounterRepository;
use crate::db::repositories::patient::PatientRepository;
use crate::db::repositories::schedule::ScheduleRepository;
use crate::db::repositories::slot::SlotRepository;
use crate::models::Gender;
use crate::models::patient::HumanName as DomainHumanName;
use crate::models::practitioner::Practitioner;

use super::operation_outcome::OperationOutcome;
use super::resources::{
    FhirAppointment, FhirBundle, FhirBundleResponse, FhirBundleWriteEntry, FhirCoverage,
    FhirEncounter, FhirLocation, FhirPatient, FhirPractitioner, FhirSchedule, FhirSlot,
    FhirWriteBundle, patient_bundle,
};

/// Handler return type: a typed FHIR resource on success, or an
/// `OperationOutcome` payload plus HTTP status on failure.
type FhirResult<T> = Result<Json<T>, (StatusCode, Json<OperationOutcome>)>;

/// Map a PAS [`crate::Error`] to an HTTP status + FHIR `OperationOutcome`.
fn error_to_response(err: Error) -> (StatusCode, Json<OperationOutcome>) {
    let (status, code) = match &err {
        Error::NotFound(_) => (StatusCode::NOT_FOUND, "not-found"),
        Error::Validation(_) | Error::Fhir(_) => (StatusCode::BAD_REQUEST, "invalid"),
        Error::InvalidStateTransition(_) | Error::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "exception"),
    };
    (status, Json(OperationOutcome::error(code, err.to_string())))
}

/// `GET /fhir/Patient/{id}` — return a patient as a FHIR `Patient` resource.
pub async fn get_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> FhirResult<FhirPatient> {
    match PatientRepository::find_by_id(&state.db, id).await {
        Ok(Some(p)) => Ok(Json(FhirPatient::from(&p))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(OperationOutcome::not_found(format!("Patient/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `GET /fhir/Encounter/{id}` — return an encounter as a FHIR `Encounter`.
pub async fn get_encounter(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> FhirResult<FhirEncounter> {
    match EncounterRepository::find_by_id(&state.db, id).await {
        Ok(Some(e)) => Ok(Json(FhirEncounter::from(&e))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(OperationOutcome::not_found(format!("Encounter/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `GET /fhir/Appointment/{id}` — return an appointment as a FHIR
/// `Appointment`.
pub async fn get_appointment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> FhirResult<FhirAppointment> {
    match AppointmentRepository::find_by_id(&state.db, id).await {
        Ok(Some(a)) => Ok(Json(FhirAppointment::from(&a))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(OperationOutcome::not_found(format!("Appointment/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `POST /fhir/Patient` — create a patient from a FHIR `Patient` resource.
pub async fn create_patient(
    State(state): State<AppState>,
    Json(fp): Json<FhirPatient>,
) -> FhirResult<FhirPatient> {
    let mut p = match fp.into_domain() {
        Ok(p) => p,
        Err(e) => return Err(error_to_response(e)),
    };
    // Force a fresh server-side id; ignore any caller-supplied id.
    p.id = Uuid::new_v4();
    p.created_at = chrono::Utc::now();
    p.updated_at = p.created_at;
    if let Err(e) = crate::validation::validate_patient(&p) {
        return Err(error_to_response(e));
    }
    match PatientRepository::create(&state.db, &p).await {
        Ok(p) => {
            if let Some(search) = &state.search {
                let _ = search.index_patient(&p);
            }
            Ok(Json(FhirPatient::from(&p)))
        }
        Err(e) => Err(error_to_response(e)),
    }
}

/// `PUT /fhir/Patient/{id}` — replace a patient from a FHIR `Patient` resource.
pub async fn update_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut fp): Json<FhirPatient>,
) -> FhirResult<FhirPatient> {
    let existing = match PatientRepository::find_by_id(&state.db, id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(OperationOutcome::not_found(format!("Patient/{id}"))),
            ));
        }
        Err(e) => return Err(error_to_response(e)),
    };
    // FHIR PUT treats the URL id as canonical. Strip any
    // client-supplied body id so `into_domain`'s strict UUID parsing
    // doesn't reject a placeholder like `"ignored-client-id"`. The
    // id is overridden to the URL value below anyway.
    fp.id = None;
    let mut next = match fp.into_domain() {
        Ok(p) => p,
        Err(e) => return Err(error_to_response(e)),
    };
    // Preserve identity: id and created_at survive PUT.
    next.id = existing.id;
    next.created_at = existing.created_at;
    next.updated_at = chrono::Utc::now();
    if let Err(e) = crate::validation::validate_patient(&next) {
        return Err(error_to_response(e));
    }
    match PatientRepository::update(&state.db, &next).await {
        Ok(p) => {
            if let Some(search) = &state.search {
                let _ = search.index_patient(&p);
            }
            Ok(Json(FhirPatient::from(&p)))
        }
        Err(e) => Err(error_to_response(e)),
    }
}

/// `DELETE /fhir/Patient/{id}` — soft-delete a patient.
pub async fn delete_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<OperationOutcome>)> {
    if let Err(e) = PatientRepository::soft_delete(&state.db, id).await {
        return Err(error_to_response(e));
    }
    if let Some(search) = &state.search {
        let _ = search.delete_patient(id);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /fhir/Encounter` — create an encounter from a FHIR `Encounter`.
pub async fn create_encounter(
    State(state): State<AppState>,
    Json(fe): Json<FhirEncounter>,
) -> FhirResult<FhirEncounter> {
    let mut e = match fe.into_domain() {
        Ok(e) => e,
        Err(err) => return Err(error_to_response(err)),
    };
    e.id = Uuid::new_v4();
    e.created_at = chrono::Utc::now();
    e.updated_at = e.created_at;
    match EncounterRepository::create(&state.db, &e).await {
        Ok(e) => Ok(Json(FhirEncounter::from(&e))),
        Err(err) => Err(error_to_response(err)),
    }
}

/// `GET /fhir/Practitioner/{id}` — return a practitioner as a FHIR
/// `Practitioner`.
pub async fn get_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> FhirResult<FhirPractitioner> {
    use crate::db::entities::practitioner;
    use sea_orm::EntityTrait;
    match practitioner::Entity::find_by_id(id).one(&state.db).await {
        Ok(Some(m)) => {
            let p = practitioner_from_model(m).map_err(error_to_response)?;
            Ok(Json(FhirPractitioner::from(&p)))
        }
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(OperationOutcome::not_found(format!("Practitioner/{id}"))),
        )),
        Err(e) => Err(error_to_response(Error::Database(e))),
    }
}

/// `GET /fhir/Schedule/{id}` — return a schedule as a FHIR `Schedule`.
pub async fn get_schedule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> FhirResult<FhirSchedule> {
    match ScheduleRepository::find_by_id(&state.db, id).await {
        Ok(Some(s)) => Ok(Json(FhirSchedule::from(&s))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(OperationOutcome::not_found(format!("Schedule/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `GET /fhir/Slot/{id}` — return a slot as a FHIR `Slot`.
pub async fn get_slot(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult<FhirSlot> {
    match SlotRepository::find_by_id(&state.db, id).await {
        Ok(Some(s)) => Ok(Json(FhirSlot::from(&s))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(OperationOutcome::not_found(format!("Slot/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `GET /fhir/Location/{id}` — return a bed as a FHIR `Location` resource.
pub async fn get_location(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> FhirResult<FhirLocation> {
    match BedRepository::find_by_id(&state.db, id).await {
        Ok(Some(b)) => Ok(Json(FhirLocation::from(&b))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(OperationOutcome::not_found(format!("Location/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

// ----- v0.21 FHIR write surface for Practitioner / Schedule / Slot -------
//
// Previously read-only at `GET /fhir/{Practitioner,Schedule,Slot}/{id}`.
// v0.21 adds the matching POST / PUT / DELETE so a FHIR client can
// fully manage these resources without dropping to the
// `/api/practitioners` / `/api/schedules` / `/api/slots` REST surface.
//
// DELETE semantics:
//   - Practitioner: flips `active = false` (soft delete via the FHIR
//     `active` flag — Practitioner rows are referenced by Appointment
//     / Encounter / Schedule and hard-delete would orphan them).
//   - Schedule: hard delete via `ScheduleRepository::delete`.
//   - Slot: hard delete via `SlotRepository::delete`.
// Invariant §5.3 (soft-delete restricted to patients / encounters /
// appointments) is honored: Schedule and Slot don't carry a
// `deleted_at` column; Practitioner re-uses the existing `active`
// flag, mirroring the REST `update_practitioner` semantics.

/// `POST /fhir/Practitioner` — create a practitioner from a FHIR
/// `Practitioner` resource. Server-side id is always assigned
/// (any client-supplied `id` is ignored).
pub async fn create_practitioner(
    State(state): State<AppState>,
    Json(fp): Json<FhirPractitioner>,
) -> FhirResult<FhirPractitioner> {
    use crate::db::entities::practitioner;
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::{ActiveModelTrait, Set, TransactionTrait};
    let domain = match fp.into_domain() {
        Ok(p) => p,
        Err(e) => return Err(error_to_response(e)),
    };
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().fixed_offset();
    let am = practitioner::ActiveModel {
        id: Set(id),
        active: Set(true),
        name: Set(serde_json::to_value(&domain.name).unwrap_or_default()),
        identifiers: Set(serde_json::to_value(&domain.identifiers).unwrap_or_default()),
        telecom: Set(serde_json::to_value(&domain.telecom).unwrap_or_default()),
        addresses: Set(serde_json::to_value(&domain.addresses).unwrap_or_default()),
        gender: Set(gender_to_str(domain.gender).to_string()),
        birth_date: Set(domain.birth_date),
        created_at: Set(now),
        updated_at: Set(now),
    };
    // v0.25: wrap insert + outbox in one transaction so the
    // `PractitionerCreated` event lands atomically with the row.
    // (FHIR-side audit is a broader gap not addressed by this
    // release; the REST-side handler does write audit.)
    let txn_res = state
        .db
        .transaction::<_, practitioner::Model, Error>(|txn| {
            Box::pin(async move {
                let m = am.insert(txn).await.map_err(Error::Database)?;
                OutboxRepository::publish(
                    txn,
                    "PractitionerCreated",
                    &serde_json::json!({ "practitioner_id": id }),
                )
                .await?;
                Ok(m)
            })
        })
        .await;
    let m = match txn_res {
        Ok(m) => m,
        Err(sea_orm::TransactionError::Connection(c)) => {
            return Err(error_to_response(Error::Database(c)));
        }
        Err(sea_orm::TransactionError::Transaction(t)) => return Err(error_to_response(t)),
    };
    match practitioner_from_model(m) {
        Ok(p) => Ok(Json(FhirPractitioner::from(&p))),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `PUT /fhir/Practitioner/{id}` — replace a practitioner from a FHIR
/// `Practitioner` resource. Preserves `id` and `created_at`.
pub async fn update_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut fp): Json<FhirPractitioner>,
) -> FhirResult<FhirPractitioner> {
    use crate::db::entities::practitioner;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    let existing = match practitioner::Entity::find_by_id(id).one(&state.db).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(OperationOutcome::not_found(format!("Practitioner/{id}"))),
            ));
        }
        Err(e) => return Err(error_to_response(Error::Database(e))),
    };
    // URL id is canonical; ignore any client-supplied body id.
    fp.id = None;
    let domain = match fp.into_domain() {
        Ok(p) => p,
        Err(e) => return Err(error_to_response(e)),
    };
    let now = chrono::Utc::now().fixed_offset();
    let mut am: practitioner::ActiveModel = existing.into();
    am.active = Set(domain.active);
    am.name = Set(serde_json::to_value(&domain.name).unwrap_or_default());
    am.identifiers = Set(serde_json::to_value(&domain.identifiers).unwrap_or_default());
    am.telecom = Set(serde_json::to_value(&domain.telecom).unwrap_or_default());
    am.addresses = Set(serde_json::to_value(&domain.addresses).unwrap_or_default());
    am.gender = Set(gender_to_str(domain.gender).to_string());
    am.birth_date = Set(domain.birth_date);
    am.updated_at = Set(now);
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::TransactionTrait;
    let txn_res = state
        .db
        .transaction::<_, practitioner::Model, Error>(|txn| {
            Box::pin(async move {
                let m = am.update(txn).await.map_err(Error::Database)?;
                OutboxRepository::publish(
                    txn,
                    "PractitionerUpdated",
                    &serde_json::json!({ "practitioner_id": id }),
                )
                .await?;
                Ok(m)
            })
        })
        .await;
    let m = match txn_res {
        Ok(m) => m,
        Err(sea_orm::TransactionError::Connection(c)) => {
            return Err(error_to_response(Error::Database(c)));
        }
        Err(sea_orm::TransactionError::Transaction(t)) => return Err(error_to_response(t)),
    };
    match practitioner_from_model(m) {
        Ok(p) => Ok(Json(FhirPractitioner::from(&p))),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `DELETE /fhir/Practitioner/{id}` — flip `active = false`.
/// Soft-delete via the FHIR `Practitioner.active` flag rather than a
/// row delete: practitioner rows are referenced by encounter /
/// appointment / schedule and an orphan reference would silently
/// break those join paths.
pub async fn delete_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> std::result::Result<StatusCode, (StatusCode, Json<OperationOutcome>)> {
    use crate::db::entities::practitioner;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    let existing = match practitioner::Entity::find_by_id(id).one(&state.db).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(OperationOutcome::not_found(format!("Practitioner/{id}"))),
            ));
        }
        Err(e) => return Err(error_to_response(Error::Database(e))),
    };
    let now = chrono::Utc::now().fixed_offset();
    let mut am: practitioner::ActiveModel = existing.into();
    am.active = Set(false);
    am.updated_at = Set(now);
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::TransactionTrait;
    let txn_res = state
        .db
        .transaction::<_, (), Error>(|txn| {
            Box::pin(async move {
                am.update(txn).await.map_err(Error::Database)?;
                OutboxRepository::publish(
                    txn,
                    "PractitionerDeactivated",
                    &serde_json::json!({ "practitioner_id": id }),
                )
                .await?;
                Ok(())
            })
        })
        .await;
    match txn_res {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(sea_orm::TransactionError::Connection(c)) => Err(error_to_response(Error::Database(c))),
        Err(sea_orm::TransactionError::Transaction(t)) => Err(error_to_response(t)),
    }
}

/// `POST /fhir/Schedule` — create a schedule from a FHIR `Schedule`
/// resource.
pub async fn create_schedule(
    State(state): State<AppState>,
    Json(fs): Json<FhirSchedule>,
) -> FhirResult<FhirSchedule> {
    let mut s = match fs.into_domain() {
        Ok(s) => s,
        Err(e) => return Err(error_to_response(e)),
    };
    s.id = Uuid::new_v4();
    s.created_at = chrono::Utc::now();
    s.updated_at = s.created_at;
    match ScheduleRepository::create(&state.db, &s).await {
        Ok(s) => Ok(Json(FhirSchedule::from(&s))),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `PUT /fhir/Schedule/{id}` — replace a schedule. Preserves `id` and
/// `created_at`.
pub async fn update_schedule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut fs): Json<FhirSchedule>,
) -> FhirResult<FhirSchedule> {
    let existing = match ScheduleRepository::find_by_id(&state.db, id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(OperationOutcome::not_found(format!("Schedule/{id}"))),
            ));
        }
        Err(e) => return Err(error_to_response(e)),
    };
    // URL id is canonical; ignore any client-supplied body id.
    fs.id = None;
    let mut next = match fs.into_domain() {
        Ok(s) => s,
        Err(e) => return Err(error_to_response(e)),
    };
    next.id = existing.id;
    next.created_at = existing.created_at;
    next.updated_at = chrono::Utc::now();
    match ScheduleRepository::update(&state.db, &next).await {
        Ok(s) => Ok(Json(FhirSchedule::from(&s))),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `DELETE /fhir/Schedule/{id}` — hard delete. Schedules don't carry
/// a `deleted_at` column (invariant §5.3 limits soft-delete to
/// patients / encounters / appointments).
pub async fn delete_schedule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> std::result::Result<StatusCode, (StatusCode, Json<OperationOutcome>)> {
    match ScheduleRepository::delete(&state.db, id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `POST /fhir/Slot` — create a slot from a FHIR `Slot` resource.
pub async fn create_slot(
    State(state): State<AppState>,
    Json(fs): Json<FhirSlot>,
) -> FhirResult<FhirSlot> {
    let mut s = match fs.into_domain() {
        Ok(s) => s,
        Err(e) => return Err(error_to_response(e)),
    };
    s.id = Uuid::new_v4();
    s.created_at = chrono::Utc::now();
    s.updated_at = s.created_at;
    match SlotRepository::create(&state.db, &s).await {
        Ok(s) => Ok(Json(FhirSlot::from(&s))),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `PUT /fhir/Slot/{id}` — replace a slot. Preserves `id` and
/// `created_at`. Status transitions are **not** validated on this
/// path — `PUT` is an operator-driven override; use the booking /
/// scheduling REST surface for state-machine-protected status flips.
pub async fn update_slot(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut fs): Json<FhirSlot>,
) -> FhirResult<FhirSlot> {
    let existing = match SlotRepository::find_by_id(&state.db, id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(OperationOutcome::not_found(format!("Slot/{id}"))),
            ));
        }
        Err(e) => return Err(error_to_response(e)),
    };
    // URL id is canonical; ignore any client-supplied body id.
    fs.id = None;
    let mut next = match fs.into_domain() {
        Ok(s) => s,
        Err(e) => return Err(error_to_response(e)),
    };
    next.id = existing.id;
    next.created_at = existing.created_at;
    next.updated_at = chrono::Utc::now();
    match SlotRepository::update(&state.db, &next).await {
        Ok(s) => Ok(Json(FhirSlot::from(&s))),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `DELETE /fhir/Slot/{id}` — hard delete. Use with care: deleting a
/// Slot that already has an Appointment booked against it will
/// orphan that appointment's `slot_id` reference (the schema does
/// not enforce a foreign-key relationship). Operators should cancel
/// the appointment first.
pub async fn delete_slot(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> std::result::Result<StatusCode, (StatusCode, Json<OperationOutcome>)> {
    match SlotRepository::delete(&state.db, id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err(error_to_response(e)),
    }
}

/// Helper: stringify the domain `Gender` enum for the practitioner
/// row. The practitioner table stores the gender as text (mirroring
/// the FHIR R5 value set).
fn gender_to_str(g: Gender) -> &'static str {
    match g {
        Gender::Male => "male",
        Gender::Female => "female",
        Gender::Other => "other",
        Gender::Unknown => "unknown",
    }
}

/// `GET /fhir/Coverage/{id}` — return a coverage row as a FHIR
/// `Coverage` resource. Coverage write through `POST /fhir` Bundle
/// entries is supported as of v0.13; this endpoint stays read-only.
pub async fn get_coverage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> FhirResult<FhirCoverage> {
    match CoverageRepository::find_by_id(&state.db, id).await {
        Ok(Some(c)) => Ok(Json(FhirCoverage::from(&c))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(OperationOutcome::not_found(format!("Coverage/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

/// Query string for `GET /fhir/Patient`.
#[derive(Debug, Deserialize)]
pub struct PatientBundleQuery {
    #[serde(default, rename = "_count")]
    pub count: Option<u64>,
}

/// `GET /fhir/Patient` — return a collection `Bundle` of recent patients.
pub async fn list_patient_bundle(
    State(state): State<AppState>,
    Query(q): Query<PatientBundleQuery>,
) -> Result<Json<FhirBundle<FhirPatient>>, (StatusCode, Json<OperationOutcome>)> {
    let limit = q.count.unwrap_or(50).min(500);
    match PatientRepository::list_active(&state.db, limit).await {
        Ok(patients) => Ok(Json(patient_bundle(&patients))),
        Err(e) => Err(error_to_response(e)),
    }
}

/// `POST /fhir` — process a write Bundle (`type: batch` or `type: transaction`).
#[utoipa::path(
    post,
    path = "/fhir",
    tag = "FHIR",
    request_body(
        content = FhirWriteBundle,
        description = "A FHIR R5 Bundle with `type: batch` or `type: transaction`. Each entry must carry a `request: { method: POST, url: <type> }` and a `resource` of type Patient, Encounter, Appointment, Coverage, Practitioner, Schedule, or Slot (last three v0.23)."
    ),
    responses(
        (status = 200, description = "Response Bundle with per-entry `status` + `location`.", body = FhirWriteBundle),
        (status = 400, description = "Transaction rolled back, or Bundle had a malformed envelope.", body = OperationOutcome)
    )
)]
///
/// Each entry must carry a `resource` with one of `resourceType` in
/// {`Patient`, `Encounter`, `Appointment`} and a `request.method = "POST"`.
///
/// **batch**: each entry is independent. Per-entry success/failure surfaces
/// in the entry's `response.status`. The HTTP status is always 200.
///
/// **transaction**: all-or-nothing. Every entry is processed inside a single
/// `sea_orm::DatabaseTransaction`. If any entry fails (parse, validation,
/// DB error), the transaction is rolled back and the endpoint returns
/// `400 + OperationOutcome` describing the offending entry — no partial
/// writes survive. Search-index updates are deferred until after the
/// commit, so Tantivy never sees rows that get rolled back.
pub async fn process_bundle(
    State(state): State<AppState>,
    Json(input): Json<FhirWriteBundle>,
) -> Result<Json<FhirWriteBundle>, (StatusCode, Json<OperationOutcome>)> {
    if input.resource_type != "Bundle" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(OperationOutcome::error(
                "invalid",
                format!("expected resourceType=Bundle, got {}", input.resource_type),
            )),
        ));
    }
    if input.bundle_type != "batch" && input.bundle_type != "transaction" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(OperationOutcome::error(
                "invalid",
                format!(
                    "unsupported Bundle.type {:?} (expected batch or transaction)",
                    input.bundle_type
                ),
            )),
        ));
    }

    if input.bundle_type == "transaction" {
        process_transaction_bundle(state, input).await
    } else {
        process_batch_bundle(state, input).await
    }
}

async fn process_batch_bundle(
    state: AppState,
    input: FhirWriteBundle,
) -> Result<Json<FhirWriteBundle>, (StatusCode, Json<OperationOutcome>)> {
    let mut out_entries: Vec<FhirBundleWriteEntry> = Vec::with_capacity(input.entry.len());
    for entry in input.entry {
        let outcome = match validate_and_route(&entry) {
            Ok(resource) => match resource {
                ResourceKind::Patient(v) => create_patient_in_db(&state.db, v).await,
                ResourceKind::Encounter(v) => create_encounter_in_db(&state.db, v).await,
                ResourceKind::Appointment(v) => create_appointment_in_db(&state.db, v).await,
                ResourceKind::Coverage(v) => create_coverage_in_db(&state.db, v).await,
                ResourceKind::Practitioner(v) => create_practitioner_in_db(&state.db, v).await,
                ResourceKind::Schedule(v) => create_schedule_in_db(&state.db, v).await,
                ResourceKind::Slot(v) => create_slot_in_db(&state.db, v).await,
            },
            Err(diag) => Err(diag),
        };
        let response = match outcome {
            Ok(success) => {
                if let Some(p) = success.indexable_patient
                    && let Some(search) = &state.search
                {
                    let _ = search.index_patient(&p);
                }
                success.response
            }
            Err(diag) => FhirBundleResponse {
                status: diag,
                location: None,
            },
        };
        out_entries.push(FhirBundleWriteEntry {
            full_url: entry.full_url,
            resource: None,
            request: None,
            response: Some(response),
        });
    }
    Ok(Json(FhirWriteBundle {
        resource_type: "Bundle".into(),
        bundle_type: "batch-response".into(),
        entry: out_entries,
    }))
}

async fn process_transaction_bundle(
    state: AppState,
    input: FhirWriteBundle,
) -> Result<Json<FhirWriteBundle>, (StatusCode, Json<OperationOutcome>)> {
    use sea_orm::TransactionTrait;

    let txn = state.db.begin().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(OperationOutcome::exception(format!(
                "begin transaction: {e}"
            ))),
        )
    })?;

    // Process every entry inside the same transaction. Collect successful
    // responses + any patients to index post-commit. On the first failure,
    // roll back and return an OperationOutcome that names the offending
    // index and diagnostic.
    let mut responses: Vec<FhirBundleResponse> = Vec::with_capacity(input.entry.len());
    let mut full_urls: Vec<Option<String>> = Vec::with_capacity(input.entry.len());
    let mut patients_to_index: Vec<crate::models::patient::Patient> = Vec::new();
    for (idx, entry) in input.entry.iter().enumerate() {
        let outcome = match validate_and_route(entry) {
            Ok(resource) => match resource {
                ResourceKind::Patient(v) => create_patient_in_db(&txn, v).await,
                ResourceKind::Encounter(v) => create_encounter_in_db(&txn, v).await,
                ResourceKind::Appointment(v) => create_appointment_in_db(&txn, v).await,
                ResourceKind::Coverage(v) => create_coverage_in_db(&txn, v).await,
                ResourceKind::Practitioner(v) => create_practitioner_in_db(&txn, v).await,
                ResourceKind::Schedule(v) => create_schedule_in_db(&txn, v).await,
                ResourceKind::Slot(v) => create_slot_in_db(&txn, v).await,
            },
            Err(diag) => Err(diag),
        };
        match outcome {
            Ok(success) => {
                if let Some(p) = success.indexable_patient {
                    patients_to_index.push(p);
                }
                responses.push(success.response);
                full_urls.push(entry.full_url.clone());
            }
            Err(diag) => {
                let _ = txn.rollback().await;
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(OperationOutcome::error(
                        "invalid",
                        format!("transaction rolled back at entry {idx}: {diag}"),
                    )),
                ));
            }
        }
    }

    if let Err(e) = txn.commit().await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(OperationOutcome::exception(format!("commit: {e}"))),
        ));
    }

    // Post-commit side effects.
    if let Some(search) = &state.search {
        for p in &patients_to_index {
            let _ = search.index_patient(p);
        }
    }

    let out_entries: Vec<FhirBundleWriteEntry> = responses
        .into_iter()
        .zip(full_urls)
        .map(|(response, full_url)| FhirBundleWriteEntry {
            full_url,
            resource: None,
            request: None,
            response: Some(response),
        })
        .collect();

    Ok(Json(FhirWriteBundle {
        resource_type: "Bundle".into(),
        bundle_type: "transaction-response".into(),
        entry: out_entries,
    }))
}

/// What kind of resource an entry carries — pre-validated so per-mode loops
/// don't have to repeat the dispatch.
enum ResourceKind {
    Patient(serde_json::Value),
    Encounter(serde_json::Value),
    Appointment(serde_json::Value),
    Coverage(serde_json::Value),
    Practitioner(serde_json::Value),
    Schedule(serde_json::Value),
    Slot(serde_json::Value),
}

/// Per-entry preflight: ensure the entry carries a resource, the request
/// method is POST (we don't support PUT/DELETE in Bundle entries), and the
/// resource type is one we know how to create. Returns `Err(diagnostic)` on
/// any preflight failure.
fn validate_and_route(entry: &FhirBundleWriteEntry) -> Result<ResourceKind, String> {
    let resource = entry
        .resource
        .clone()
        .ok_or_else(|| "400 Bad Request: entry missing resource".to_string())?;
    let request_method = entry
        .request
        .as_ref()
        .map(|r| r.method.as_str())
        .unwrap_or("POST");
    if !request_method.eq_ignore_ascii_case("POST") {
        return Err(format!(
            "405 Method Not Allowed: {request_method} not supported in Bundle entries"
        ));
    }
    let resource_type = resource
        .get("resourceType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match resource_type {
        "Patient" => Ok(ResourceKind::Patient(resource)),
        "Encounter" => Ok(ResourceKind::Encounter(resource)),
        "Appointment" => Ok(ResourceKind::Appointment(resource)),
        "Coverage" => Ok(ResourceKind::Coverage(resource)),
        "Practitioner" => Ok(ResourceKind::Practitioner(resource)),
        "Schedule" => Ok(ResourceKind::Schedule(resource)),
        "Slot" => Ok(ResourceKind::Slot(resource)),
        other => Err(format!(
            "400 Bad Request: unsupported resourceType {other:?} in Bundle entry"
        )),
    }
}

/// Per-entry success carries the wire response plus any post-commit side
/// effect (e.g. a Patient to index in Tantivy).
struct EntrySuccess {
    response: FhirBundleResponse,
    indexable_patient: Option<crate::models::patient::Patient>,
}

async fn create_patient_in_db<C: sea_orm::ConnectionTrait>(
    conn: &C,
    resource: serde_json::Value,
) -> Result<EntrySuccess, String> {
    let fp: FhirPatient = serde_json::from_value(resource)
        .map_err(|e| format!("400 Bad Request: Patient parse: {e}"))?;
    let mut p = fp
        .into_domain()
        .map_err(|e| format!("400 Bad Request: {e}"))?;
    p.id = Uuid::new_v4();
    let now = chrono::Utc::now();
    p.created_at = now;
    p.updated_at = now;
    crate::validation::validate_patient(&p).map_err(|e| format!("400 Bad Request: {e}"))?;
    let created = PatientRepository::create(conn, &p)
        .await
        .map_err(|e| format!("500 Internal Server Error: {e}"))?;
    Ok(EntrySuccess {
        response: FhirBundleResponse {
            status: "201 Created".into(),
            location: Some(format!("Patient/{}", created.id)),
        },
        indexable_patient: Some(created),
    })
}

async fn create_encounter_in_db<C: sea_orm::ConnectionTrait>(
    conn: &C,
    resource: serde_json::Value,
) -> Result<EntrySuccess, String> {
    let fe: FhirEncounter = serde_json::from_value(resource)
        .map_err(|e| format!("400 Bad Request: Encounter parse: {e}"))?;
    let mut e = fe
        .into_domain()
        .map_err(|err| format!("400 Bad Request: {err}"))?;
    e.id = Uuid::new_v4();
    let now = chrono::Utc::now();
    e.created_at = now;
    e.updated_at = now;
    let created = EncounterRepository::create(conn, &e)
        .await
        .map_err(|err| format!("500 Internal Server Error: {err}"))?;
    Ok(EntrySuccess {
        response: FhirBundleResponse {
            status: "201 Created".into(),
            location: Some(format!("Encounter/{}", created.id)),
        },
        indexable_patient: None,
    })
}

async fn create_appointment_in_db<C: sea_orm::ConnectionTrait>(
    conn: &C,
    resource: serde_json::Value,
) -> Result<EntrySuccess, String> {
    let fa: FhirAppointment = serde_json::from_value(resource)
        .map_err(|e| format!("400 Bad Request: Appointment parse: {e}"))?;
    let mut a = fa
        .into_domain()
        .map_err(|err| format!("400 Bad Request: {err}"))?;
    a.id = Uuid::new_v4();
    let now = chrono::Utc::now();
    a.created_at = now;
    a.updated_at = now;
    crate::validation::validate_appointment(&a).map_err(|e| format!("400 Bad Request: {e}"))?;
    let created = AppointmentRepository::create(conn, &a)
        .await
        .map_err(|err| format!("500 Internal Server Error: {err}"))?;
    Ok(EntrySuccess {
        response: FhirBundleResponse {
            status: "201 Created".into(),
            location: Some(format!("Appointment/{}", created.id)),
        },
        indexable_patient: None,
    })
}

async fn create_coverage_in_db<C: sea_orm::ConnectionTrait>(
    conn: &C,
    resource: serde_json::Value,
) -> Result<EntrySuccess, String> {
    let fc: FhirCoverage = serde_json::from_value(resource)
        .map_err(|e| format!("400 Bad Request: Coverage parse: {e}"))?;
    let mut c = fc
        .into_domain()
        .map_err(|err| format!("400 Bad Request: {err}"))?;
    c.id = Uuid::new_v4();
    let now = chrono::Utc::now();
    c.created_at = now;
    c.updated_at = now;
    let created = crate::db::repositories::coverage::CoverageRepository::create(conn, &c)
        .await
        .map_err(|err| format!("500 Internal Server Error: {err}"))?;
    Ok(EntrySuccess {
        response: FhirBundleResponse {
            status: "201 Created".into(),
            location: Some(format!("Coverage/{}", created.id)),
        },
        indexable_patient: None,
    })
}

async fn create_practitioner_in_db<C: sea_orm::ConnectionTrait>(
    conn: &C,
    resource: serde_json::Value,
) -> Result<EntrySuccess, String> {
    use crate::db::entities::practitioner;
    use sea_orm::{ActiveModelTrait, Set};
    let fp: FhirPractitioner = serde_json::from_value(resource)
        .map_err(|e| format!("400 Bad Request: Practitioner parse: {e}"))?;
    let domain = fp
        .into_domain()
        .map_err(|err| format!("400 Bad Request: {err}"))?;
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().fixed_offset();
    let am = practitioner::ActiveModel {
        id: Set(id),
        active: Set(true),
        name: Set(serde_json::to_value(&domain.name).unwrap_or_default()),
        identifiers: Set(serde_json::to_value(&domain.identifiers).unwrap_or_default()),
        telecom: Set(serde_json::to_value(&domain.telecom).unwrap_or_default()),
        addresses: Set(serde_json::to_value(&domain.addresses).unwrap_or_default()),
        gender: Set(match domain.gender {
            crate::models::Gender::Male => "male".into(),
            crate::models::Gender::Female => "female".into(),
            crate::models::Gender::Other => "other".into(),
            crate::models::Gender::Unknown => "unknown".into(),
        }),
        birth_date: Set(domain.birth_date),
        created_at: Set(now),
        updated_at: Set(now),
    };
    am.insert(conn)
        .await
        .map_err(|err| format!("500 Internal Server Error: {err}"))?;
    Ok(EntrySuccess {
        response: FhirBundleResponse {
            status: "201 Created".into(),
            location: Some(format!("Practitioner/{id}")),
        },
        indexable_patient: None,
    })
}

async fn create_schedule_in_db<C: sea_orm::ConnectionTrait>(
    conn: &C,
    resource: serde_json::Value,
) -> Result<EntrySuccess, String> {
    let fs: FhirSchedule = serde_json::from_value(resource)
        .map_err(|e| format!("400 Bad Request: Schedule parse: {e}"))?;
    let mut s = fs
        .into_domain()
        .map_err(|err| format!("400 Bad Request: {err}"))?;
    s.id = Uuid::new_v4();
    let now = chrono::Utc::now();
    s.created_at = now;
    s.updated_at = now;
    let created = ScheduleRepository::create(conn, &s)
        .await
        .map_err(|err| format!("500 Internal Server Error: {err}"))?;
    Ok(EntrySuccess {
        response: FhirBundleResponse {
            status: "201 Created".into(),
            location: Some(format!("Schedule/{}", created.id)),
        },
        indexable_patient: None,
    })
}

async fn create_slot_in_db<C: sea_orm::ConnectionTrait>(
    conn: &C,
    resource: serde_json::Value,
) -> Result<EntrySuccess, String> {
    let fs: FhirSlot = serde_json::from_value(resource)
        .map_err(|e| format!("400 Bad Request: Slot parse: {e}"))?;
    let mut s = fs
        .into_domain()
        .map_err(|err| format!("400 Bad Request: {err}"))?;
    s.id = Uuid::new_v4();
    let now = chrono::Utc::now();
    s.created_at = now;
    s.updated_at = now;
    let created = SlotRepository::create(conn, &s)
        .await
        .map_err(|err| format!("500 Internal Server Error: {err}"))?;
    Ok(EntrySuccess {
        response: FhirBundleResponse {
            status: "201 Created".into(),
            location: Some(format!("Slot/{}", created.id)),
        },
        indexable_patient: None,
    })
}

/// Helper: turn a `practitioner` SeaORM model into the domain `Practitioner`.
fn practitioner_from_model(
    m: crate::db::entities::practitioner::Model,
) -> Result<Practitioner, Error> {
    let name: DomainHumanName = serde_json::from_value(m.name)
        .map_err(|e| Error::internal(format!("deserialize practitioner name: {e}")))?;
    let identifiers = serde_json::from_value(m.identifiers)
        .map_err(|e| Error::internal(format!("deserialize identifiers: {e}")))?;
    let telecom = serde_json::from_value(m.telecom)
        .map_err(|e| Error::internal(format!("deserialize telecom: {e}")))?;
    let addresses = serde_json::from_value(m.addresses)
        .map_err(|e| Error::internal(format!("deserialize addresses: {e}")))?;
    let gender = match m.gender.as_str() {
        "male" => Gender::Male,
        "female" => Gender::Female,
        "other" => Gender::Other,
        _ => Gender::Unknown,
    };
    Ok(Practitioner {
        id: m.id,
        identifiers,
        active: m.active,
        name,
        telecom,
        addresses,
        gender,
        birth_date: m.birth_date,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

/// `POST /fhir/Appointment` — create an appointment from a FHIR `Appointment`.
pub async fn create_appointment(
    State(state): State<AppState>,
    Json(fa): Json<FhirAppointment>,
) -> FhirResult<FhirAppointment> {
    let mut a = match fa.into_domain() {
        Ok(a) => a,
        Err(err) => return Err(error_to_response(err)),
    };
    a.id = Uuid::new_v4();
    a.created_at = chrono::Utc::now();
    a.updated_at = a.created_at;
    if let Err(e) = crate::validation::validate_appointment(&a) {
        return Err(error_to_response(e));
    }
    match AppointmentRepository::create(&state.db, &a).await {
        Ok(a) => Ok(Json(FhirAppointment::from(&a))),
        Err(err) => Err(error_to_response(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_to_response_not_found_maps_404() {
        let (status, body) = error_to_response(Error::not_found("nope"));
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.issue[0].code, "not-found");
    }

    #[test]
    fn test_error_to_response_validation_maps_400() {
        let (status, body) = error_to_response(Error::validation("bad"));
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.issue[0].code, "invalid");
    }

    #[test]
    fn test_error_to_response_state_transition_maps_409() {
        let (status, body) = error_to_response(Error::invalid_transition("nope"));
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.issue[0].code, "conflict");
    }

    #[test]
    fn test_error_to_response_internal_maps_500() {
        let (status, body) = error_to_response(Error::internal("boom"));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.issue[0].code, "exception");
    }
}
