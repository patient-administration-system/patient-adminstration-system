//! REST API handlers.
//!
//! Handlers are intentionally thin: they extract request payloads, build a
//! [`UserContext`] from the standard PAS headers, call the appropriate service
//! method, and serialize the result into an [`ApiResponse`].
//!
//! v0.1 pragma: every handler returns `Json<ApiResponse<serde_json::Value>>`
//! so we don't have to define a typed response per endpoint. Errors are mapped
//! to a `(StatusCode, Json<ApiResponse<…>>)` tuple based on the variant of
//! [`crate::Error`].

use std::str::FromStr;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::ApiResponse;
use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::bed::BedRepository;
use crate::db::repositories::billing::BillingRepository;
use crate::db::repositories::consent::ConsentRepository;
use crate::db::repositories::encounter::EncounterRepository;
use crate::db::repositories::letter::LetterRepository;
use crate::db::repositories::patient::PatientRepository;
use crate::db::repositories::rtt::RttRepository;
use crate::db::repositories::schedule::ScheduleRepository;
use crate::db::repositories::slot::SlotRepository;
use crate::db::repositories::waitlist::WaitlistRepository;
use crate::models::appointment::CancellationReason;
use crate::models::billing::PaymentMethod;
use crate::models::communication::DeliveryChannel;
use crate::models::facility::BedStatus;
use crate::models::patient::{HumanName, Patient};
use crate::models::waitlist::Priority;
use crate::models::{Gender, Iso4217, Money};
use crate::privacy::{export_patient, mask_patient};
use crate::validation::validate_patient;
use crate::{Error, Result};

use super::state::AppState;

/// Standard handler return type: an HTTP status plus a JSON `ApiResponse`.
type HandlerResult = (StatusCode, Json<ApiResponse<Value>>);

/// Map a service-layer [`Error`] onto an HTTP status code + machine-readable
/// error code.
fn error_to_response(err: Error) -> HandlerResult {
    let (status, code) = match &err {
        Error::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
        Error::Validation(_) => (StatusCode::BAD_REQUEST, "VALIDATION"),
        Error::InvalidStateTransition(_) => (StatusCode::CONFLICT, "INVALID_STATE_TRANSITION"),
        Error::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
        Error::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "DATABASE"),
        Error::Search(_) => (StatusCode::INTERNAL_SERVER_ERROR, "SEARCH"),
        Error::Streaming(_) => (StatusCode::INTERNAL_SERVER_ERROR, "STREAMING"),
        Error::Fhir(_) => (StatusCode::BAD_REQUEST, "FHIR"),
        Error::Render(_) => (StatusCode::INTERNAL_SERVER_ERROR, "RENDER"),
        Error::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, "CONFIG"),
        Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
    };
    (status, Json(ApiResponse::err(code, err.to_string())))
}

/// Convert a `Result<Value>` into a handler response: `200 OK` on success,
/// or the appropriate status + code on failure.
fn ok_response(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

fn finish<T: serde::Serialize>(result: Result<T>) -> HandlerResult {
    match result {
        Ok(v) => match serde_json::to_value(&v) {
            Ok(value) => ok_response(value),
            Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
        },
        Err(e) => error_to_response(e),
    }
}

/// Extract `UserContext` from the standard PAS audit headers
/// (`X-User-Id`, `X-User-Ip`, `X-User-Agent`).
fn user_context_from_headers(headers: &HeaderMap) -> UserContext {
    UserContext {
        user_id: headers
            .get("X-User-Id")
            .and_then(|h| h.to_str().ok())
            .map(String::from),
        user_ip: headers
            .get("X-User-Ip")
            .and_then(|h| h.to_str().ok())
            .map(String::from),
        user_agent: headers
            .get("X-User-Agent")
            .and_then(|h| h.to_str().ok())
            .map(String::from),
    }
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

#[utoipa::path(get, path = "/api/health", tag = "Health",
    responses((status = 200, description = "Server + database health: `{ status, database }`.")))]
pub async fn health(State(state): State<AppState>) -> Json<ApiResponse<Value>> {
    let db_ok = state.db.ping().await.is_ok();
    Json(ApiResponse::ok(serde_json::json!({
        "status": if db_ok { "ok" } else { "degraded" },
        "database": if db_ok { "ok" } else { "unreachable" },
    })))
}

// ---------------------------------------------------------------------------
// ADT
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AdmitRequest {
    pub patient_id: Uuid,
    pub bed_id: Uuid,
}

#[utoipa::path(post, path = "/api/admissions", tag = "ADT",
    request_body = AdmitRequest,
    responses(
        (status = 200, description = "Admission committed: encounter, bed assignment, admission row."),
        (status = 409, description = "Bed unavailable."),
        (status = 404, description = "Bed not found.")
    ))]
pub async fn admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AdmitRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.adt.admit(req.patient_id, req.bed_id, &ctx).await)
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TransferRequest {
    pub new_bed_id: Uuid,
}

#[utoipa::path(post, path = "/api/admissions/{admission_id}/transfer", tag = "ADT",
    params(("admission_id" = Uuid, Path, description = "Admission to transfer")),
    request_body = TransferRequest,
    responses((status = 200, description = "Patient transferred to the new bed."),
              (status = 409, description = "New bed unavailable.")))]
pub async fn transfer(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<TransferRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.adt.transfer(admission_id, req.new_bed_id, &ctx).await)
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PreAdmitRequest {
    pub patient_id: Uuid,
    pub bed_id: Uuid,
}

#[utoipa::path(post, path = "/api/admissions/pre-admit", tag = "ADT",
    request_body = PreAdmitRequest,
    responses(
        (status = 200, description = "Bed reserved; Planned inpatient encounter opened."),
        (status = 404, description = "Bed not found."),
        (status = 409, description = "Bed not available for reservation.")
    ))]
pub async fn pre_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PreAdmitRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(
        state
            .adt
            .pre_admit(req.patient_id, req.bed_id, None, &ctx)
            .await,
    )
}

#[utoipa::path(post, path = "/api/admissions/{admission_id}/cancel-admit", tag = "ADT",
    params(("admission_id" = Uuid, Path, description = "Admission to cancel")),
    responses(
        (status = 200, description = "Admission cancelled; bed released; encounter Cancelled."),
        (status = 404, description = "Admission not found."),
        (status = 409, description = "Encounter cannot be cancelled (already terminal).")
    ))]
pub async fn cancel_admit(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.adt.cancel_admission(admission_id, None, &ctx).await)
}

#[utoipa::path(post, path = "/api/admissions/{admission_id}/leave-start", tag = "ADT",
    params(("admission_id" = Uuid, Path, description = "Admission whose encounter goes on leave")),
    responses(
        (status = 200, description = "Encounter transitioned InProgress → OnLeave."),
        (status = 404, description = "Admission not found."),
        (status = 409, description = "Encounter not in InProgress.")
    ))]
pub async fn leave_start(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.adt.start_leave(admission_id, None, &ctx).await)
}

#[utoipa::path(post, path = "/api/admissions/{admission_id}/leave-end", tag = "ADT",
    params(("admission_id" = Uuid, Path, description = "Admission whose encounter returns from leave")),
    responses(
        (status = 200, description = "Encounter transitioned OnLeave → InProgress."),
        (status = 404, description = "Admission not found."),
        (status = 409, description = "Encounter not OnLeave.")
    ))]
pub async fn leave_end(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.adt.end_leave(admission_id, None, &ctx).await)
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ChangeToInpatientRequest {
    pub patient_id: Uuid,
    pub bed_id: Uuid,
}

#[utoipa::path(post, path = "/api/admissions/change-to-inpatient", tag = "ADT",
    request_body = ChangeToInpatientRequest,
    responses(
        (status = 200, description = "Encounter promoted to Inpatient; bed allocated."),
        (status = 404, description = "Patient / bed / active ambulatory encounter not found."),
        (status = 409, description = "Bed not available.")
    ))]
pub async fn change_to_inpatient(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChangeToInpatientRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(
        state
            .adt
            .change_to_inpatient(req.patient_id, req.bed_id, None, &ctx)
            .await,
    )
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CancelPreAdmitRequest {
    pub patient_id: Uuid,
    pub bed_id: Uuid,
}

#[utoipa::path(post, path = "/api/admissions/cancel-pre-admit", tag = "ADT",
    request_body = CancelPreAdmitRequest,
    responses(
        (status = 200, description = "Bed reservation released; Planned encounter cancelled."),
        (status = 404, description = "Bed / patient / planned encounter not found."),
        (status = 409, description = "Bed is not Reserved.")
    ))]
pub async fn cancel_pre_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CancelPreAdmitRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(
        state
            .adt
            .cancel_pre_admit(req.patient_id, req.bed_id, None, &ctx)
            .await,
    )
}

#[utoipa::path(post, path = "/api/admissions/{admission_id}/cancel-transfer", tag = "ADT",
    params(("admission_id" = Uuid, Path, description = "Admission whose latest transfer to undo")),
    responses(
        (status = 200, description = "Transfer cancelled; patient restored to origin bed."),
        (status = 404, description = "Admission has no transfer to cancel."),
        (status = 409, description = "Origin bed unavailable for restoration.")
    ))]
pub async fn cancel_transfer(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.adt.cancel_transfer(admission_id, None, &ctx).await)
}

#[utoipa::path(post, path = "/api/admissions/{admission_id}/discharge", tag = "ADT",
    params(("admission_id" = Uuid, Path, description = "Admission to discharge")),
    responses((status = 200, description = "Patient discharged; bed released.")))]
pub async fn discharge(
    State(state): State<AppState>,
    Path(admission_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.adt.discharge(admission_id, &ctx).await)
}

// ---------------------------------------------------------------------------
// Scheduling
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BookRequest {
    pub patient_id: Uuid,
}

#[utoipa::path(post, path = "/api/slots/{slot_id}/book", tag = "Scheduling",
    params(("slot_id" = Uuid, Path, description = "Slot to book")),
    request_body = BookRequest,
    responses((status = 200, description = "Slot booked; appointment created."),
              (status = 409, description = "Slot busy or patient has an overlap.")))]
pub async fn book_slot(
    State(state): State<AppState>,
    Path(slot_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<BookRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(
        state
            .scheduling
            .book_slot(slot_id, req.patient_id, &ctx)
            .await,
    )
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CancelRequest {
    #[schema(value_type = String)]
    pub reason: CancellationReason,
}

#[utoipa::path(post, path = "/api/appointments/{appointment_id}/cancel", tag = "Scheduling",
    params(("appointment_id" = Uuid, Path)),
    request_body = CancelRequest,
    responses((status = 200, description = "Appointment cancelled; slot freed.")))]
pub async fn cancel_appointment(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<CancelRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(
        state
            .scheduling
            .cancel(appointment_id, req.reason, &ctx)
            .await,
    )
}

#[utoipa::path(post, path = "/api/appointments/{appointment_id}/check-in", tag = "Scheduling",
    params(("appointment_id" = Uuid, Path)),
    responses((status = 200, description = "Patient marked Arrived.")))]
pub async fn check_in_appointment(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.scheduling.check_in(appointment_id, &ctx).await)
}

#[utoipa::path(post, path = "/api/appointments/{appointment_id}/complete", tag = "Scheduling",
    params(("appointment_id" = Uuid, Path)),
    responses((status = 200, description = "Appointment marked Fulfilled.")))]
pub async fn complete_appointment(
    State(state): State<AppState>,
    Path(appointment_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.scheduling.complete(appointment_id, &ctx).await)
}

// ---------------------------------------------------------------------------
// Waitlist
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct WaitlistAddRequest {
    pub patient_id: Uuid,
    pub target_service: String,
    #[schema(value_type = String)]
    pub priority: Priority,
    pub referral_id: Option<Uuid>,
}

#[utoipa::path(post, path = "/api/waitlist", tag = "Waitlist",
    request_body = WaitlistAddRequest,
    responses((status = 200, description = "Waitlist entry created.")))]
pub async fn add_to_waitlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<WaitlistAddRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(
        state
            .waitlist
            .add(
                req.referral_id,
                req.patient_id,
                req.target_service,
                req.priority,
                &ctx,
            )
            .await,
    )
}

#[utoipa::path(delete, path = "/api/waitlist/{entry_id}", tag = "Waitlist",
    params(("entry_id" = Uuid, Path)),
    responses((status = 200, description = "Waitlist entry removed.")))]
pub async fn remove_from_waitlist(
    State(state): State<AppState>,
    Path(entry_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.waitlist.remove(entry_id, &ctx).await)
}

// ---------------------------------------------------------------------------
// RTT
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RttStartRequest {
    pub patient_id: Uuid,
    pub target_service: String,
}

#[utoipa::path(post, path = "/api/rtt/start", tag = "RTT",
    request_body = RttStartRequest,
    responses((status = 200, description = "RTT clock started on a new pathway.")))]
pub async fn rtt_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RttStartRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(
        state
            .rtt
            .start(req.patient_id, req.target_service, &ctx)
            .await,
    )
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RttReasonRequest {
    pub reason: String,
}

#[utoipa::path(post, path = "/api/rtt/{pathway_id}/pause", tag = "RTT",
    params(("pathway_id" = Uuid, Path)),
    request_body = RttReasonRequest,
    responses((status = 200, description = "RTT clock paused.")))]
pub async fn rtt_pause(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<RttReasonRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.rtt.pause(pathway_id, req.reason, &ctx).await)
}

#[utoipa::path(post, path = "/api/rtt/{pathway_id}/resume", tag = "RTT",
    params(("pathway_id" = Uuid, Path)),
    responses((status = 200, description = "RTT clock resumed.")))]
pub async fn rtt_resume(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.rtt.resume(pathway_id, &ctx).await)
}

#[utoipa::path(post, path = "/api/rtt/{pathway_id}/stop", tag = "RTT",
    params(("pathway_id" = Uuid, Path)),
    request_body = RttReasonRequest,
    responses((status = 200, description = "RTT clock stopped (terminal).")))]
pub async fn rtt_stop(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<RttReasonRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(state.rtt.stop(pathway_id, req.reason, &ctx).await)
}

#[utoipa::path(get, path = "/api/rtt/{pathway_id}/weeks-waiting", tag = "RTT",
    params(("pathway_id" = Uuid, Path)),
    responses((status = 200, description = "Active weeks waiting on the pathway.")))]
pub async fn rtt_weeks_waiting(
    State(state): State<AppState>,
    Path(pathway_id): Path<Uuid>,
) -> HandlerResult {
    match state.rtt.weeks_waiting(pathway_id).await {
        Ok(w) => ok_response(serde_json::json!({ "pathway_id": pathway_id, "weeks_waiting": w })),
        Err(e) => error_to_response(e),
    }
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

#[utoipa::path(get, path = "/api/wards/{ward_id}/occupancy", tag = "Resources",
    params(("ward_id" = Uuid, Path)),
    responses((status = 200, description = "Snapshot of bed occupancy in the ward.")))]
pub async fn ward_occupancy(
    State(state): State<AppState>,
    Path(ward_id): Path<Uuid>,
) -> HandlerResult {
    finish(state.resources.ward_occupancy(ward_id).await)
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetBedStatusRequest {
    #[schema(value_type = String)]
    pub status: BedStatus,
}

#[utoipa::path(put, path = "/api/beds/{bed_id}/status", tag = "Resources",
    params(("bed_id" = Uuid, Path)),
    request_body = SetBedStatusRequest,
    responses((status = 200, description = "Bed status transition committed.")))]
pub async fn set_bed_status(
    State(state): State<AppState>,
    Path(bed_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<SetBedStatusRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(
        state
            .resources
            .set_bed_status(bed_id, req.status, &ctx)
            .await,
    )
}

// ---------------------------------------------------------------------------
// Billing
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct OpenAccountRequest {
    pub patient_id: Uuid,
    pub currency: String,
}

#[utoipa::path(post, path = "/api/accounts", tag = "Billing",
    request_body = OpenAccountRequest,
    responses((status = 200, description = "New patient billing account opened."),
              (status = 409, description = "Patient already has an open account.")))]
pub async fn open_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<OpenAccountRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    let currency = match Iso4217::new(&req.currency) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    finish(
        state
            .billing
            .open_account(req.patient_id, currency, &ctx)
            .await,
    )
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PostChargeRequest {
    pub account_id: Uuid,
    pub code: String,
    pub description: String,
    pub amount_value: String,
    pub amount_currency: String,
    pub encounter_id: Option<Uuid>,
    pub appointment_id: Option<Uuid>,
}

#[utoipa::path(post, path = "/api/charges", tag = "Billing",
    request_body = PostChargeRequest,
    responses((status = 200, description = "Charge posted to the account.")))]
pub async fn post_charge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PostChargeRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    let amount = match build_money(&req.amount_value, &req.amount_currency) {
        Ok(m) => m,
        Err(e) => return error_to_response(e),
    };
    finish(
        state
            .billing
            .post_charge(
                crate::billing::PostChargeInput {
                    account_id: req.account_id,
                    code: req.code,
                    description: req.description,
                    amount,
                    encounter_id: req.encounter_id,
                    appointment_id: req.appointment_id,
                },
                &ctx,
            )
            .await,
    )
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct FinalizeInvoiceRequest {
    pub account_id: Uuid,
    pub charge_ids: Vec<Uuid>,
}

#[utoipa::path(post, path = "/api/invoices", tag = "Billing",
    request_body = FinalizeInvoiceRequest,
    responses((status = 200, description = "Invoice finalized from the supplied charges.")))]
pub async fn finalize_invoice(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<FinalizeInvoiceRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(
        state
            .billing
            .finalize_invoice(req.account_id, req.charge_ids, &ctx)
            .await,
    )
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PostPaymentRequest {
    pub invoice_id: Uuid,
    pub amount_value: String,
    pub amount_currency: String,
    #[schema(value_type = String)]
    pub method: PaymentMethod,
    pub reference: Option<String>,
}

#[utoipa::path(post, path = "/api/payments", tag = "Billing",
    request_body = PostPaymentRequest,
    responses((status = 200, description = "Payment posted; invoice outstanding updated.")))]
pub async fn post_payment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PostPaymentRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    let amount = match build_money(&req.amount_value, &req.amount_currency) {
        Ok(m) => m,
        Err(e) => return error_to_response(e),
    };
    finish(
        state
            .billing
            .post_payment(req.invoice_id, amount, req.method, req.reference, &ctx)
            .await,
    )
}

fn build_money(value: &str, currency: &str) -> Result<Money> {
    let amount = Decimal::from_str(value)
        .map_err(|e| Error::validation(format!("invalid amount {value:?}: {e}")))?;
    let iso = Iso4217::new(currency)?;
    Ok(Money::new(amount, iso))
}

// ---------------------------------------------------------------------------
// Communication
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct GenerateLetterRequest {
    pub template_id: Uuid,
    pub patient_id: Uuid,
    pub appointment_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub channel: DeliveryChannel,
    #[schema(value_type = Object)]
    #[serde(default)]
    pub extra: Value,
}

#[utoipa::path(post, path = "/api/letters/generate", tag = "Communication",
    request_body = GenerateLetterRequest,
    responses((status = 200, description = "Letter rendered + persisted as Pending."),
              (status = 400, description = "Template missing required variables.")))]
pub async fn generate_letter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<GenerateLetterRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    finish(
        state
            .communication
            .generate_letter(
                req.template_id,
                req.patient_id,
                req.appointment_id,
                req.channel,
                req.extra,
                &ctx,
            )
            .await,
    )
}

// ---------------------------------------------------------------------------
// Patient CRUD + search
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreatePatientRequest {
    pub family: String,
    pub given: Vec<String>,
    #[schema(value_type = String)]
    #[serde(default)]
    pub gender: Option<Gender>,
    #[serde(default)]
    pub birth_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub mpi_id: Option<Uuid>,
}

#[utoipa::path(post, path = "/api/patients", tag = "Patient",
    request_body = CreatePatientRequest,
    responses((status = 200, description = "Patient created + indexed + audited.")))]
pub async fn create_patient(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreatePatientRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    let name = HumanName {
        use_type: None,
        family: req.family,
        given: req.given,
        prefix: vec![],
        suffix: vec![],
    };
    let mut p = Patient::new(name, req.gender.unwrap_or(Gender::Unknown));
    p.birth_date = req.birth_date;
    p.mpi_id = req.mpi_id;
    if let Err(e) = validate_patient(&p) {
        return error_to_response(e);
    }
    let created = match PatientRepository::create(&state.db, &p).await {
        Ok(p) => p,
        Err(e) => return error_to_response(e),
    };
    if let Some(search) = &state.search {
        let _ = search.index_patient(&created);
    }
    let _ = AuditLogRepository::log(
        &state.db,
        "patient",
        created.id,
        "create",
        None,
        Some(serde_json::to_value(&created).unwrap_or_default()),
        &ctx,
    )
    .await;
    finish::<Patient>(Ok(created))
}

#[utoipa::path(get, path = "/api/patients/{id}", tag = "Patient",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Patient record."),
              (status = 404, description = "No patient with that id.")))]
pub async fn get_patient(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match PatientRepository::find_by_id(&state.db, id).await {
        Ok(Some(p)) => finish::<Patient>(Ok(p)),
        Ok(None) => error_to_response(Error::not_found(format!("patient {id}"))),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdatePatientRequest {
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub given: Option<Vec<String>>,
    #[schema(value_type = String)]
    #[serde(default)]
    pub gender: Option<Gender>,
    #[serde(default)]
    pub birth_date: Option<chrono::NaiveDate>,
}

#[utoipa::path(put, path = "/api/patients/{id}", tag = "Patient",
    params(("id" = Uuid, Path)),
    request_body = UpdatePatientRequest,
    responses((status = 200, description = "Selective field update; reindexed + audited.")))]
pub async fn update_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<UpdatePatientRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    let mut p = match PatientRepository::find_by_id(&state.db, id).await {
        Ok(Some(p)) => p,
        Ok(None) => return error_to_response(Error::not_found(format!("patient {id}"))),
        Err(e) => return error_to_response(e),
    };
    if let Some(f) = req.family {
        p.name.family = f;
    }
    if let Some(g) = req.given {
        p.name.given = g;
    }
    if let Some(g) = req.gender {
        p.gender = g;
    }
    if let Some(bd) = req.birth_date {
        p.birth_date = Some(bd);
    }
    p.updated_at = chrono::Utc::now();
    if let Err(e) = validate_patient(&p) {
        return error_to_response(e);
    }
    let updated = match PatientRepository::update(&state.db, &p).await {
        Ok(p) => p,
        Err(e) => return error_to_response(e),
    };
    if let Some(search) = &state.search {
        let _ = search.index_patient(&updated);
    }
    let _ = AuditLogRepository::log(
        &state.db,
        "patient",
        updated.id,
        "update",
        None,
        Some(serde_json::to_value(&updated).unwrap_or_default()),
        &ctx,
    )
    .await;
    finish::<Patient>(Ok(updated))
}

#[utoipa::path(delete, path = "/api/patients/{id}", tag = "Patient",
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "Patient soft-deleted (deleted_at set, dropped from search)."),
        (status = 404, description = "Patient not found."),
        (status = 409, description = "Patient has an open admission; refuse to delete.")
    ))]
pub async fn delete_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::TransactionTrait;
    let ctx = user_context_from_headers(&headers);
    // v0.40: same safety check as the ADT^A23 inbound handler — refuse
    // to soft-delete a patient who currently has an open admission.
    match AdmissionRepository::find_open_for_patient(&state.db, id).await {
        Ok(Some(adm)) => {
            return error_to_response(Error::conflict(format!(
                "patient {id} has open admission {}; refuse to delete",
                adm.id
            )));
        }
        Ok(None) => {}
        Err(e) => return error_to_response(e),
    }
    let txn_res = state
        .db
        .transaction::<_, (), Error>(|txn| {
            Box::pin(async move {
                PatientRepository::soft_delete(txn, id).await?;
                AuditLogRepository::log(txn, "patient", id, "soft_delete", None, None, &ctx)
                    .await?;
                OutboxRepository::publish(
                    txn,
                    "PatientDeleted",
                    &serde_json::json!({ "patient_id": id }),
                )
                .await?;
                Ok(())
            })
        })
        .await;
    if let Err(e) = txn_res {
        return match e {
            sea_orm::TransactionError::Connection(c) => error_to_response(Error::Database(c)),
            sea_orm::TransactionError::Transaction(t) => error_to_response(t),
        };
    }
    if let Some(search) = &state.search {
        let _ = search.delete_patient(id);
    }
    ok_response(serde_json::json!({ "id": id, "deleted": true }))
}

#[derive(Debug, Deserialize)]
pub struct SearchPatientsQuery {
    pub q: String,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

fn default_search_limit() -> usize {
    10
}

#[utoipa::path(get, path = "/api/patients/search", tag = "Patient",
    params(("q" = String, Query, description = "Search term"),
           ("limit" = Option<usize>, Query, description = "Max hits, default 10")),
    responses((status = 200, description = "Tantivy search hits ordered by relevance.")))]
pub async fn search_patients(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<SearchPatientsQuery>,
) -> HandlerResult {
    let search = match &state.search {
        Some(s) => s,
        None => return error_to_response(Error::Search("search engine not configured".into())),
    };
    let ids = match search.search(&query.q, query.limit) {
        Ok(v) => v,
        Err(e) => return error_to_response(e),
    };
    let mut patients = Vec::with_capacity(ids.len());
    for id in ids {
        if let Ok(Some(p)) = PatientRepository::find_by_id(&state.db, id).await {
            patients.push(p);
        }
    }
    finish::<Vec<Patient>>(Ok(patients))
}

// ---------------------------------------------------------------------------
// Privacy
// ---------------------------------------------------------------------------

#[utoipa::path(get, path = "/api/patients/{id}/masked", tag = "Privacy",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Patient with sensitive fields masked (SSN/MRN tail, phone middle, postal-code last 3, address line 1).")))]
pub async fn get_patient_masked(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match PatientRepository::find_by_id(&state.db, id).await {
        Ok(Some(p)) => finish::<Patient>(Ok(mask_patient(&p))),
        Ok(None) => error_to_response(Error::not_found(format!("patient {id}"))),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/patients/{id}/export", tag = "Privacy",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "GDPR subject-access export: full Patient JSON dump.")))]
pub async fn get_patient_export(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match PatientRepository::find_by_id(&state.db, id).await {
        Ok(Some(p)) => ok_response(export_patient(&p)),
        Ok(None) => error_to_response(Error::not_found(format!("patient {id}"))),
        Err(e) => error_to_response(e),
    }
}

// ---------------------------------------------------------------------------
// Audit query
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    pub limit: u64,
}

fn default_audit_limit() -> u64 {
    50
}

#[utoipa::path(get, path = "/api/patients/{id}/audit", tag = "Audit",
    params(("id" = Uuid, Path), ("limit" = Option<u64>, Query, description = "Max rows, default 50, capped at 500")),
    responses((status = 200, description = "Audit log entries for the patient, newest first.")))]
pub async fn get_patient_audit(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<AuditQuery>,
) -> HandlerResult {
    let limit = q.limit.min(500);
    match AuditLogRepository::list_for_entity(&state.db, "patient", id, limit).await {
        Ok(rows) => match serde_json::to_value(&rows) {
            Ok(v) => ok_response(v),
            Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
        },
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/audit/recent", tag = "Audit",
    params(("limit" = Option<u64>, Query, description = "Max rows, default 50, capped at 500")),
    responses((status = 200, description = "Most recent audit entries across all entities.")))]
pub async fn get_recent_audit(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AuditQuery>,
) -> HandlerResult {
    let limit = q.limit.min(500);
    match AuditLogRepository::list_recent(&state.db, limit).await {
        Ok(rows) => match serde_json::to_value(&rows) {
            Ok(v) => ok_response(v),
            Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
        },
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct EntityAuditQuery {
    pub entity_type: String,
    pub entity_id: Uuid,
    #[serde(default = "default_audit_limit")]
    pub limit: u64,
}

/// Generic audit query — surfaces history for any entity type (encounter,
/// admission, charge, consent, etc.), not just patient.
#[utoipa::path(get, path = "/api/audit/entity", tag = "Audit",
    params(
        ("entity_type" = String, Query, description = "e.g. encounter, admission, charge"),
        ("entity_id" = Uuid, Query),
        ("limit" = Option<u64>, Query, description = "Max rows, capped at 500")
    ),
    responses((status = 200, description = "Audit entries for the named entity.")))]
pub async fn get_entity_audit(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<EntityAuditQuery>,
) -> HandlerResult {
    let limit = q.limit.min(500);
    match AuditLogRepository::list_for_entity(&state.db, &q.entity_type, q.entity_id, limit).await {
        Ok(rows) => match serde_json::to_value(&rows) {
            Ok(v) => ok_response(v),
            Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
        },
        Err(e) => error_to_response(e),
    }
}

// ---------------------------------------------------------------------------
// Outbox diagnostics
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OutboxQuery {
    #[serde(default = "default_outbox_limit")]
    pub limit: u64,
}

fn default_outbox_limit() -> u64 {
    100
}

/// Operational endpoint: list currently-unpublished outbox events so an
/// operator can see what the dispatcher is failing to deliver. Capped at 500.
#[utoipa::path(get, path = "/api/admin/outbox/unpublished", tag = "Admin",
    params(("limit" = Option<u64>, Query, description = "Max rows, default 100, capped at 500")),
    responses((status = 200, description = "Outbox rows still flagged unpublished.")))]
pub async fn get_unpublished_outbox(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<OutboxQuery>,
) -> HandlerResult {
    use crate::db::repositories::outbox::OutboxRepository;
    let limit = q.limit.min(500);
    match OutboxRepository::fetch_unpublished(&state.db, limit).await {
        Ok(rows) => match serde_json::to_value(&rows) {
            Ok(v) => ok_response(v),
            Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
        },
        Err(e) => error_to_response(e),
    }
}

/// Operational endpoint (v0.5): list outbox rows that exceeded the
/// configured retry budget and were moved to the dead-letter queue. Newest
/// first; capped at 500.
#[utoipa::path(get, path = "/api/admin/outbox/dead-letters", tag = "Admin",
    params(("limit" = Option<u64>, Query, description = "Max rows, default 100, capped at 500")),
    responses((status = 200, description = "Dead-letter rows with original payload, retry count, last error.")))]
pub async fn list_dead_letter_outbox(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<OutboxQuery>,
) -> HandlerResult {
    use crate::db::repositories::DeadLetterRepository;
    let limit = q.limit.min(500);
    match DeadLetterRepository::list(&state.db, limit).await {
        Ok(rows) => match serde_json::to_value(&rows) {
            Ok(v) => ok_response(v),
            Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
        },
        Err(e) => error_to_response(e),
    }
}

/// Operational endpoint (v0.5): replay a dead-letter row back into the
/// outbox. Inserts a fresh `outbox_events` row carrying the same payload
/// and `event_type` (with `retry_count = 0`), deletes the DLQ row, and
/// returns `{ new_outbox_id }`. Both writes happen in one DB transaction.
/// Idempotent: returns `404 NOT_FOUND` if the DLQ id is already gone.
#[utoipa::path(post, path = "/api/admin/outbox/dead-letters/{id}/replay", tag = "Admin",
    params(("id" = Uuid, Path, description = "DLQ row id (not the original outbox id)")),
    responses(
        (status = 200, description = "Replayed; body carries the new outbox row id."),
        (status = 404, description = "DLQ id not found."),
    ))]
pub async fn replay_dead_letter_outbox(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    use crate::db::repositories::DeadLetterRepository;
    use sea_orm::TransactionTrait;
    let ctx = user_context_from_headers(&headers);
    let txn_res = state
        .db
        .transaction::<_, Uuid, Error>(|txn| {
            Box::pin(async move { DeadLetterRepository::replay(txn, id).await })
        })
        .await;
    let new_outbox_id = match txn_res {
        Ok(v) => v,
        Err(sea_orm::TransactionError::Connection(c)) => {
            return error_to_response(Error::Database(c));
        }
        Err(sea_orm::TransactionError::Transaction(t)) => return error_to_response(t),
    };
    // Audit the replay so the action is traceable even though the original
    // event payload is recoverable from the new outbox row.
    let _ = AuditLogRepository::log(
        &state.db,
        "outbox_dead_letter",
        id,
        "replay",
        None,
        Some(serde_json::json!({ "new_outbox_id": new_outbox_id })),
        &ctx,
    )
    .await;
    ok_response(serde_json::json!({
        "dead_letter_id": id,
        "new_outbox_id": new_outbox_id,
    }))
}

// ---------------------------------------------------------------------------
// Query endpoints (read-only)
// ---------------------------------------------------------------------------

#[utoipa::path(get, path = "/api/encounters/{id}", tag = "Encounter",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Encounter record."), (status = 404, description = "Not found.")))]
pub async fn get_encounter(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match EncounterRepository::find_by_id(&state.db, id).await {
        Ok(Some(e)) => finish(Ok(e)),
        Ok(None) => error_to_response(Error::not_found(format!("encounter {id}"))),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/patients/{id}/encounters", tag = "Encounter",
    params(("id" = Uuid, Path, description = "Patient id")),
    responses((status = 200, description = "All encounters for the patient.")))]
pub async fn list_patient_encounters(
    State(state): State<AppState>,
    Path(patient_id): Path<Uuid>,
) -> HandlerResult {
    match EncounterRepository::list_by_patient(&state.db, patient_id).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(post, path = "/api/encounters/{id}/cancel", tag = "Encounter",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Encounter status flipped to Cancelled.")))]
pub async fn cancel_encounter(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    match EncounterRepository::set_status(
        &state.db,
        id,
        crate::models::encounter::EncounterStatus::Cancelled,
    )
    .await
    {
        Ok(e) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "encounter",
                id,
                "cancel",
                None,
                Some(serde_json::to_value(&e).unwrap_or_default()),
                &ctx,
            )
            .await;
            finish(Ok(e))
        }
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct EncounterStatusRequest {
    #[schema(value_type = String)]
    pub status: crate::models::encounter::EncounterStatus,
}

#[utoipa::path(put, path = "/api/encounters/{id}/status", tag = "Encounter",
    params(("id" = Uuid, Path)),
    request_body = EncounterStatusRequest,
    responses((status = 200, description = "Encounter status transition committed."),
              (status = 409, description = "Invalid state transition.")))]
pub async fn set_encounter_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<EncounterStatusRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    match EncounterRepository::set_status(&state.db, id, req.status).await {
        Ok(e) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "encounter",
                id,
                "set_status",
                None,
                Some(serde_json::to_value(&e).unwrap_or_default()),
                &ctx,
            )
            .await;
            finish(Ok(e))
        }
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/appointments/{id}", tag = "Scheduling",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Appointment record."), (status = 404, description = "Not found.")))]
pub async fn get_appointment(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match AppointmentRepository::find_by_id(&state.db, id).await {
        Ok(Some(a)) => finish(Ok(a)),
        Ok(None) => error_to_response(Error::not_found(format!("appointment {id}"))),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct AppointmentListQuery {
    #[serde(default)]
    pub patient_id: Option<Uuid>,
}

#[utoipa::path(get, path = "/api/appointments", tag = "Scheduling",
    params(("patient_id" = Uuid, Query, description = "Required: appointments for this patient")),
    responses((status = 200, description = "Appointments for the named patient.")))]
pub async fn list_appointments(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AppointmentListQuery>,
) -> HandlerResult {
    let patient_id = match q.patient_id {
        Some(p) => p,
        None => return error_to_response(Error::validation("patient_id query param required")),
    };
    match AppointmentRepository::list_by_patient(&state.db, patient_id).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct WaitlistListQuery {
    pub service: String,
}

#[utoipa::path(get, path = "/api/waitlist", tag = "Waitlist",
    params(("service" = String, Query, description = "Service code")),
    responses((status = 200, description = "Waitlist entries for the named service.")))]
pub async fn list_waitlist(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<WaitlistListQuery>,
) -> HandlerResult {
    match WaitlistRepository::list_by_service(&state.db, &q.service).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/patients/{id}/rtt", tag = "RTT",
    params(("id" = Uuid, Path, description = "Patient id")),
    responses((status = 200, description = "RTT pathways for the patient.")))]
pub async fn list_patient_rtt(
    State(state): State<AppState>,
    Path(patient_id): Path<Uuid>,
) -> HandlerResult {
    match RttRepository::list_pathways_by_patient(&state.db, patient_id).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/patients/{id}/account", tag = "Billing",
    params(("id" = Uuid, Path, description = "Patient id")),
    responses((status = 200, description = "Open billing account for the patient."),
              (status = 404, description = "No open account.")))]
pub async fn get_patient_account(
    State(state): State<AppState>,
    Path(patient_id): Path<Uuid>,
) -> HandlerResult {
    match BillingRepository::find_open_account_for_patient(&state.db, patient_id).await {
        Ok(Some(a)) => finish(Ok(a)),
        Ok(None) => error_to_response(Error::not_found(format!(
            "no open account for patient {patient_id}"
        ))),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateLetterTemplateRequest {
    pub name: String,
    pub subject: String,
    pub body_tera: String,
    #[serde(default)]
    pub required_variables: Vec<String>,
    #[schema(value_type = Vec<String>)]
    #[serde(default)]
    pub channels: Vec<DeliveryChannel>,
}

#[utoipa::path(post, path = "/api/letter-templates", tag = "Communication",
    request_body = CreateLetterTemplateRequest,
    responses((status = 200, description = "Letter template stored.")))]
pub async fn create_letter_template(
    State(state): State<AppState>,
    Json(req): Json<CreateLetterTemplateRequest>,
) -> HandlerResult {
    let mut tpl =
        crate::models::communication::LetterTemplate::new(req.name, req.subject, req.body_tera);
    tpl.required_variables = req.required_variables;
    if !req.channels.is_empty() {
        tpl.channels = req.channels;
    }
    match LetterRepository::create_template(&state.db, &tpl).await {
        Ok(t) => finish(Ok(t)),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/letter-templates", tag = "Communication",
    responses((status = 200, description = "Active letter templates.")))]
pub async fn list_letter_templates(State(state): State<AppState>) -> HandlerResult {
    match LetterRepository::list_active_templates(&state.db).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/letters/{id}", tag = "Communication",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Generated letter record."),
              (status = 404, description = "Not found.")))]
pub async fn get_generated_letter(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match LetterRepository::find_generated_letter_by_id(&state.db, id).await {
        Ok(Some(l)) => finish(Ok(l)),
        Ok(None) => error_to_response(Error::not_found(format!("letter {id}"))),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(post, path = "/api/letters/{id}/sent", tag = "Communication",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Letter status flipped to Sent.")))]
pub async fn mark_letter_sent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    match LetterRepository::mark_sent(&state.db, id).await {
        Ok(l) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "generated_letter",
                id,
                "mark_sent",
                None,
                None,
                &ctx,
            )
            .await;
            finish(Ok(l))
        }
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(post, path = "/api/letters/{id}/failed", tag = "Communication",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Letter status flipped to Failed.")))]
pub async fn mark_letter_failed(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    match LetterRepository::mark_failed(&state.db, id).await {
        Ok(l) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "generated_letter",
                id,
                "mark_failed",
                None,
                None,
                &ctx,
            )
            .await;
            finish(Ok(l))
        }
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateLetterTemplateRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub body_tera: Option<String>,
    #[serde(default)]
    pub required_variables: Option<Vec<String>>,
    #[schema(value_type = Option<Vec<String>>)]
    #[serde(default)]
    pub channels: Option<Vec<DeliveryChannel>>,
    #[serde(default)]
    pub active: Option<bool>,
}

#[utoipa::path(put, path = "/api/letter-templates/{id}", tag = "Communication",
    params(("id" = Uuid, Path)),
    request_body = UpdateLetterTemplateRequest,
    responses((status = 200, description = "Selective field update applied.")))]
pub async fn update_letter_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateLetterTemplateRequest>,
) -> HandlerResult {
    use crate::db::entities::letter_template;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    let existing = match letter_template::Entity::find_by_id(id).one(&state.db).await {
        Ok(Some(m)) => m,
        Ok(None) => return error_to_response(Error::not_found(format!("letter template {id}"))),
        Err(e) => return error_to_response(Error::Database(e)),
    };
    let mut am: letter_template::ActiveModel = existing.into();
    if let Some(n) = req.name {
        am.name = Set(n);
    }
    if let Some(s) = req.subject {
        am.subject = Set(s);
    }
    if let Some(b) = req.body_tera {
        am.body_tera = Set(b);
    }
    if let Some(rv) = req.required_variables {
        am.required_variables = Set(serde_json::to_value(rv).unwrap_or_default());
    }
    if let Some(ch) = req.channels {
        am.channels = Set(serde_json::to_value(ch).unwrap_or_default());
    }
    if let Some(active) = req.active {
        am.active = Set(active);
    }
    am.updated_at = Set(chrono::Utc::now().fixed_offset());
    match am.update(&state.db).await {
        Ok(m) => finish(Ok(m)),
        Err(e) => error_to_response(Error::Database(e)),
    }
}

#[utoipa::path(delete, path = "/api/letter-templates/{id}", tag = "Communication",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Template soft-deleted (active=false).")))]
pub async fn delete_letter_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    use crate::db::entities::letter_template;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    let existing = match letter_template::Entity::find_by_id(id).one(&state.db).await {
        Ok(Some(m)) => m,
        Ok(None) => return error_to_response(Error::not_found(format!("letter template {id}"))),
        Err(e) => return error_to_response(Error::Database(e)),
    };
    let mut am: letter_template::ActiveModel = existing.into();
    am.active = Set(false);
    am.updated_at = Set(chrono::Utc::now().fixed_offset());
    match am.update(&state.db).await {
        Ok(_) => ok_response(serde_json::json!({ "id": id, "deactivated": true })),
        Err(e) => error_to_response(Error::Database(e)),
    }
}

#[utoipa::path(delete, path = "/api/schedules/{id}", tag = "Scheduling",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Schedule soft-deleted (active=false).")))]
pub async fn delete_schedule(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    use crate::db::entities::schedule;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    let existing = match schedule::Entity::find_by_id(id).one(&state.db).await {
        Ok(Some(m)) => m,
        Ok(None) => return error_to_response(Error::not_found(format!("schedule {id}"))),
        Err(e) => return error_to_response(Error::Database(e)),
    };
    let mut am: schedule::ActiveModel = existing.into();
    am.active = Set(false);
    am.updated_at = Set(chrono::Utc::now().fixed_offset());
    match am.update(&state.db).await {
        Ok(_) => ok_response(serde_json::json!({ "id": id, "deactivated": true })),
        Err(e) => error_to_response(Error::Database(e)),
    }
}

/// Delete (block out) a slot. Refuses if the slot is `busy` — busy slots
/// must be released via the appointment-cancel flow first.
#[utoipa::path(delete, path = "/api/slots/{id}", tag = "Scheduling",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Slot set to BlockedOut."),
              (status = 409, description = "Slot busy; cancel the booked appointment first.")))]
pub async fn delete_slot(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    use crate::models::schedule::SlotStatus;
    let s = match SlotRepository::find_by_id(&state.db, id).await {
        Ok(Some(s)) => s,
        Ok(None) => return error_to_response(Error::not_found(format!("slot {id}"))),
        Err(e) => return error_to_response(e),
    };
    if s.status == SlotStatus::Busy {
        return error_to_response(Error::conflict(
            "slot is busy; cancel the booked appointment first",
        ));
    }
    match SlotRepository::update_status(&state.db, id, SlotStatus::BlockedOut).await {
        Ok(s) => finish(Ok(s)),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/schedules/{id}", tag = "Scheduling",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Schedule record."),
              (status = 404, description = "Not found.")))]
pub async fn get_schedule(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match ScheduleRepository::find_by_id(&state.db, id).await {
        Ok(Some(s)) => finish(Ok(s)),
        Ok(None) => error_to_response(Error::not_found(format!("schedule {id}"))),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct SlotRangeQuery {
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
}

#[utoipa::path(get, path = "/api/schedules/{id}/slots", tag = "Scheduling",
    params(
        ("id" = Uuid, Path, description = "Schedule id"),
        ("start" = String, Query, description = "ISO-8601 UTC start"),
        ("end" = String, Query, description = "ISO-8601 UTC end")
    ),
    responses((status = 200, description = "Free slots in the date range.")))]
pub async fn list_schedule_slots(
    State(state): State<AppState>,
    Path(schedule_id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<SlotRangeQuery>,
) -> HandlerResult {
    match SlotRepository::find_free_in_range(&state.db, schedule_id, q.start, q.end).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/wards/{id}/beds", tag = "Resources",
    params(("id" = Uuid, Path, description = "Ward id")),
    responses((status = 200, description = "Beds in the ward.")))]
pub async fn list_ward_beds(
    State(state): State<AppState>,
    Path(ward_id): Path<Uuid>,
) -> HandlerResult {
    match BedRepository::list_by_ward(&state.db, ward_id).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/beds/{id}", tag = "Resources",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Bed record."), (status = 404, description = "Not found.")))]
pub async fn get_bed(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match BedRepository::find_by_id(&state.db, id).await {
        Ok(Some(b)) => finish(Ok(b)),
        Ok(None) => error_to_response(Error::not_found(format!("bed {id}"))),
        Err(e) => error_to_response(e),
    }
}

// ---------------------------------------------------------------------------
// Encounter / Schedule / Slot creation
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateEncounterRequest {
    pub patient_id: Uuid,
    #[schema(value_type = String)]
    pub class: crate::models::encounter::EncounterClass,
    #[serde(default)]
    pub practitioner_id: Option<Uuid>,
    #[serde(default)]
    pub department_id: Option<Uuid>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[utoipa::path(post, path = "/api/encounters", tag = "Encounter",
    request_body = CreateEncounterRequest,
    responses((status = 200, description = "Encounter created in Planned status (non-inpatient).")))]
pub async fn create_encounter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateEncounterRequest>,
) -> HandlerResult {
    use crate::db::repositories::outbox::OutboxRepository;
    use crate::models::encounter::EncounterClass;
    use sea_orm::TransactionTrait;
    let ctx = user_context_from_headers(&headers);
    let mut e = crate::models::encounter::Encounter::new(req.patient_id, req.class);
    e.practitioner_id = req.practitioner_id;
    e.department_id = req.department_id;
    e.reason = req.reason;
    let txn_res = state
        .db
        .transaction::<_, crate::models::encounter::Encounter, Error>(|txn| {
            Box::pin(async move {
                let created = EncounterRepository::create(txn, &e).await?;
                AuditLogRepository::log(
                    txn,
                    "encounter",
                    created.id,
                    "create",
                    None,
                    Some(serde_json::to_value(&created).unwrap_or_default()),
                    &ctx,
                )
                .await?;
                // EncounterRegistered fires for ambulatory classes
                // (Outpatient + Emergency). Inpatient creation runs
                // through the AdtService::admit flow, which has its own
                // EncounterAdmitted event; emitting EncounterRegistered
                // there would double-publish.
                if matches!(
                    created.class,
                    EncounterClass::Outpatient | EncounterClass::Emergency
                ) {
                    OutboxRepository::publish(
                        txn,
                        "EncounterRegistered",
                        &serde_json::json!({
                            "encounter_id": created.id,
                            "patient_id": created.patient_id,
                            "class": format!("{:?}", created.class),
                        }),
                    )
                    .await?;
                }
                Ok(created)
            })
        })
        .await;
    match txn_res {
        Ok(created) => finish(Ok(created)),
        Err(sea_orm::TransactionError::Connection(c)) => error_to_response(Error::Database(c)),
        Err(sea_orm::TransactionError::Transaction(t)) => error_to_response(t),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ScheduleOwnerInput {
    Practitioner(Uuid),
    Bed(Uuid),
    Room(Uuid),
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateScheduleRequest {
    pub owner: ScheduleOwnerInput,
    pub service_type: String,
}

#[utoipa::path(post, path = "/api/schedules", tag = "Scheduling",
    request_body = CreateScheduleRequest,
    responses((status = 200, description = "Schedule created (owner = practitioner / bed / room).")))]
pub async fn create_schedule(
    State(state): State<AppState>,
    Json(req): Json<CreateScheduleRequest>,
) -> HandlerResult {
    let owner = match req.owner {
        ScheduleOwnerInput::Practitioner(id) => {
            crate::models::schedule::ScheduleOwner::Practitioner(id)
        }
        ScheduleOwnerInput::Bed(id) => crate::models::schedule::ScheduleOwner::Bed(id),
        ScheduleOwnerInput::Room(id) => crate::models::schedule::ScheduleOwner::Room(id),
    };
    let s = crate::models::schedule::Schedule::new(owner, req.service_type);
    match ScheduleRepository::create(&state.db, &s).await {
        Ok(s) => finish(Ok(s)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateSlotRequest {
    pub start_datetime: chrono::DateTime<chrono::Utc>,
    pub end_datetime: chrono::DateTime<chrono::Utc>,
}

#[utoipa::path(post, path = "/api/schedules/{id}/slots", tag = "Scheduling",
    params(("id" = Uuid, Path, description = "Schedule id")),
    request_body = CreateSlotRequest,
    responses((status = 200, description = "Slot created in Free status.")))]
pub async fn create_slot(
    State(state): State<AppState>,
    Path(schedule_id): Path<Uuid>,
    Json(req): Json<CreateSlotRequest>,
) -> HandlerResult {
    if req.start_datetime >= req.end_datetime {
        return error_to_response(Error::validation("slot start must be before end"));
    }
    let s = crate::models::schedule::Slot::new(schedule_id, req.start_datetime, req.end_datetime);
    match SlotRepository::create(&state.db, &s).await {
        Ok(s) => finish(Ok(s)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BulkSlotRequest {
    pub start_datetime: chrono::DateTime<chrono::Utc>,
    pub end_datetime: chrono::DateTime<chrono::Utc>,
    /// Slot length in minutes. Must be > 0.
    pub slot_minutes: u32,
}

/// `POST /api/schedules/:id/slots/bulk` — generate a series of consecutive
/// Free slots between `start_datetime` and `end_datetime`, each
/// `slot_minutes` long. Stops at the last slot that fits entirely before
/// `end_datetime`.
#[utoipa::path(post, path = "/api/schedules/{id}/slots/bulk", tag = "Scheduling",
    params(("id" = Uuid, Path, description = "Schedule id")),
    request_body = BulkSlotRequest,
    responses((status = 200, description = "Generated slots returned as an array.")))]
pub async fn bulk_create_slots(
    State(state): State<AppState>,
    Path(schedule_id): Path<Uuid>,
    Json(req): Json<BulkSlotRequest>,
) -> HandlerResult {
    if req.slot_minutes == 0 {
        return error_to_response(Error::validation("slot_minutes must be > 0"));
    }
    if req.start_datetime >= req.end_datetime {
        return error_to_response(Error::validation(
            "start_datetime must be before end_datetime",
        ));
    }
    let step = chrono::Duration::minutes(req.slot_minutes as i64);
    let mut created = Vec::new();
    let mut cursor = req.start_datetime;
    while cursor + step <= req.end_datetime {
        let s = crate::models::schedule::Slot::new(schedule_id, cursor, cursor + step);
        match SlotRepository::create(&state.db, &s).await {
            Ok(s) => created.push(s),
            Err(e) => return error_to_response(e),
        }
        cursor += step;
    }
    finish(Ok(created))
}

// ---------------------------------------------------------------------------
// Facility / Ward / Room / Bed administrative setup
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateFacilityRequest {
    pub name: String,
    pub code: String,
    /// Optional postal address. Defaults to an all-empty `Address`
    /// when omitted — useful for facilities where the address is
    /// captured elsewhere (e.g. multi-site organisations) or for
    /// HL7 v2-driven setups where the address comes via a separate
    /// MFN later. v0.22 relaxes this from required to optional.
    #[schema(value_type = Object)]
    #[serde(default)]
    pub address: crate::models::Address,
}

#[utoipa::path(post, path = "/api/facilities", tag = "FacilitySetup",
    request_body = CreateFacilityRequest,
    responses((status = 200, description = "Facility created.")))]
pub async fn create_facility(
    State(state): State<AppState>,
    Json(req): Json<CreateFacilityRequest>,
) -> HandlerResult {
    use crate::db::entities::facility;
    use sea_orm::{ActiveModelTrait, Set};
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().fixed_offset();
    let am = facility::ActiveModel {
        id: Set(id),
        name: Set(req.name),
        code: Set(req.code),
        address: Set(serde_json::to_value(&req.address).unwrap_or_default()),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    };
    match am.insert(&state.db).await {
        Ok(m) => finish(Ok(m)),
        Err(e) => error_to_response(Error::Database(e)),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateWardRequest {
    pub facility_id: Uuid,
    pub name: String,
    pub code: String,
    /// Ward bed capacity. Optional; defaults to `0` when omitted.
    /// v0.22 relaxes this from required to optional — capacity can
    /// be reset after the ward is populated.
    #[serde(default)]
    pub capacity: i32,
}

#[utoipa::path(post, path = "/api/wards", tag = "FacilitySetup",
    request_body = CreateWardRequest,
    responses((status = 200, description = "Ward created under the named facility.")))]
pub async fn create_ward(
    State(state): State<AppState>,
    Json(req): Json<CreateWardRequest>,
) -> HandlerResult {
    use crate::db::entities::ward;
    use sea_orm::{ActiveModelTrait, Set};
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().fixed_offset();
    let am = ward::ActiveModel {
        id: Set(id),
        facility_id: Set(req.facility_id),
        name: Set(req.name),
        code: Set(req.code),
        capacity: Set(req.capacity),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    };
    match am.insert(&state.db).await {
        Ok(m) => finish(Ok(m)),
        Err(e) => error_to_response(Error::Database(e)),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateRoomRequest {
    pub ward_id: Uuid,
    pub name: String,
    pub code: String,
}

#[utoipa::path(post, path = "/api/rooms", tag = "FacilitySetup",
    request_body = CreateRoomRequest,
    responses((status = 200, description = "Room created under the named ward.")))]
pub async fn create_room(
    State(state): State<AppState>,
    Json(req): Json<CreateRoomRequest>,
) -> HandlerResult {
    use crate::db::entities::room;
    use sea_orm::{ActiveModelTrait, Set};
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().fixed_offset();
    let am = room::ActiveModel {
        id: Set(id),
        ward_id: Set(req.ward_id),
        name: Set(req.name),
        code: Set(req.code),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    };
    match am.insert(&state.db).await {
        Ok(m) => finish(Ok(m)),
        Err(e) => error_to_response(Error::Database(e)),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateBedRequest {
    pub room_id: Uuid,
    pub name: String,
    pub code: String,
}

#[utoipa::path(post, path = "/api/beds", tag = "FacilitySetup",
    request_body = CreateBedRequest,
    responses((status = 200, description = "Bed created in Available status.")))]
pub async fn create_bed(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateBedRequest>,
) -> HandlerResult {
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::TransactionTrait;
    let ctx = user_context_from_headers(&headers);
    let b = crate::models::facility::Bed::new(req.room_id, req.name, req.code);
    let bed_code = b.code.clone();
    let txn_res = state
        .db
        .transaction::<_, crate::models::facility::Bed, Error>(|txn| {
            Box::pin(async move {
                let saved = BedRepository::create(txn, &b).await?;
                AuditLogRepository::log(
                    txn,
                    "bed",
                    saved.id,
                    "create",
                    None,
                    Some(serde_json::to_value(&saved).unwrap_or_default()),
                    &ctx,
                )
                .await?;
                OutboxRepository::publish(
                    txn,
                    "BedCreated",
                    &serde_json::json!({
                        "bed_id": saved.id,
                        "bed_code": bed_code,
                    }),
                )
                .await?;
                Ok(saved)
            })
        })
        .await;
    match txn_res {
        Ok(saved) => finish(Ok(saved)),
        Err(sea_orm::TransactionError::Connection(c)) => error_to_response(Error::Database(c)),
        Err(sea_orm::TransactionError::Transaction(t)) => error_to_response(t),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateBedRequest {
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
}

#[utoipa::path(put, path = "/api/beds/{id}", tag = "FacilitySetup",
    params(("id" = Uuid, Path)),
    request_body = UpdateBedRequest,
    responses((status = 200, description = "Bed updated (full replace of name/room/code)."),
              (status = 404, description = "Bed not found.")))]
pub async fn update_bed(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<UpdateBedRequest>,
) -> HandlerResult {
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::TransactionTrait;
    let ctx = user_context_from_headers(&headers);
    let existing = match BedRepository::find_by_id(&state.db, id).await {
        Ok(Some(b)) => b,
        Ok(None) => return error_to_response(Error::not_found(format!("bed {id}"))),
        Err(e) => return error_to_response(e),
    };
    let mut next = existing;
    if let Some(rid) = req.room_id {
        next.room_id = rid;
    }
    if let Some(n) = req.name {
        next.name = n;
    }
    if let Some(c) = req.code {
        next.code = c;
    }
    let bed_code = next.code.clone();
    let txn_res = state
        .db
        .transaction::<_, crate::models::facility::Bed, Error>(|txn| {
            Box::pin(async move {
                let saved = BedRepository::update(txn, &next).await?;
                AuditLogRepository::log(
                    txn,
                    "bed",
                    id,
                    "update",
                    None,
                    Some(serde_json::to_value(&saved).unwrap_or_default()),
                    &ctx,
                )
                .await?;
                OutboxRepository::publish(
                    txn,
                    "BedUpdated",
                    &serde_json::json!({
                        "bed_id": id,
                        "bed_code": bed_code,
                    }),
                )
                .await?;
                Ok(saved)
            })
        })
        .await;
    match txn_res {
        Ok(saved) => finish(Ok(saved)),
        Err(sea_orm::TransactionError::Connection(c)) => error_to_response(Error::Database(c)),
        Err(sea_orm::TransactionError::Transaction(t)) => error_to_response(t),
    }
}

// ---------------------------------------------------------------------------
// Practitioner CRUD
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreatePractitionerRequest {
    pub family: String,
    pub given: Vec<String>,
    #[schema(value_type = String)]
    #[serde(default)]
    pub gender: Option<Gender>,
    #[serde(default)]
    pub birth_date: Option<chrono::NaiveDate>,
}

#[utoipa::path(post, path = "/api/practitioners", tag = "Workforce",
    request_body = CreatePractitionerRequest,
    responses((status = 200, description = "Practitioner created.")))]
pub async fn create_practitioner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreatePractitionerRequest>,
) -> HandlerResult {
    use crate::db::entities::practitioner;
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::{ActiveModelTrait, Set, TransactionTrait};
    let ctx = user_context_from_headers(&headers);
    let now = chrono::Utc::now().fixed_offset();
    let id = Uuid::new_v4();
    let name = HumanName {
        use_type: None,
        family: req.family,
        given: req.given,
        prefix: vec![],
        suffix: vec![],
    };
    let am = practitioner::ActiveModel {
        id: Set(id),
        active: Set(true),
        name: Set(serde_json::to_value(&name).unwrap_or_default()),
        identifiers: Set(serde_json::json!([])),
        telecom: Set(serde_json::json!([])),
        addresses: Set(serde_json::json!([])),
        gender: Set(match req.gender.unwrap_or(Gender::Unknown) {
            Gender::Male => "male".into(),
            Gender::Female => "female".into(),
            Gender::Other => "other".into(),
            Gender::Unknown => "unknown".into(),
        }),
        birth_date: Set(req.birth_date),
        created_at: Set(now),
        updated_at: Set(now),
    };
    // Wrap insert + audit + outbox in one DB transaction (v0.25 added
    // audit + outbox; previously this handler bypassed both). The
    // outbox `PractitionerCreated` event with no `source` tag is what
    // the v0.25 HL7 v2 outbound publisher fans out as `MFN^M02`.
    let txn_res = state
        .db
        .transaction::<_, practitioner::Model, Error>(|txn| {
            Box::pin(async move {
                let m = am.insert(txn).await.map_err(Error::Database)?;
                AuditLogRepository::log(
                    txn,
                    "practitioner",
                    id,
                    "create",
                    None,
                    Some(serde_json::to_value(&m).unwrap_or_default()),
                    &ctx,
                )
                .await?;
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
    match txn_res {
        Ok(m) => finish(Ok(m)),
        Err(sea_orm::TransactionError::Connection(c)) => error_to_response(Error::Database(c)),
        Err(sea_orm::TransactionError::Transaction(t)) => error_to_response(t),
    }
}

#[utoipa::path(get, path = "/api/practitioners/{id}", tag = "Workforce",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Practitioner record."), (status = 404, description = "Not found.")))]
pub async fn get_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    use crate::db::entities::practitioner;
    use sea_orm::EntityTrait;
    match practitioner::Entity::find_by_id(id).one(&state.db).await {
        Ok(Some(m)) => finish(Ok(m)),
        Ok(None) => error_to_response(Error::not_found(format!("practitioner {id}"))),
        Err(e) => error_to_response(Error::Database(e)),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdatePractitionerRequest {
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub given: Option<Vec<String>>,
    #[schema(value_type = String)]
    #[serde(default)]
    pub gender: Option<Gender>,
    #[serde(default)]
    pub birth_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub active: Option<bool>,
}

#[utoipa::path(put, path = "/api/practitioners/{id}", tag = "Workforce",
    params(("id" = Uuid, Path)),
    request_body = UpdatePractitionerRequest,
    responses((status = 200, description = "Selective field update applied.")))]
pub async fn update_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<UpdatePractitionerRequest>,
) -> HandlerResult {
    use crate::db::entities::practitioner;
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set, TransactionTrait};
    let ctx = user_context_from_headers(&headers);
    let existing = match practitioner::Entity::find_by_id(id).one(&state.db).await {
        Ok(Some(m)) => m,
        Ok(None) => return error_to_response(Error::not_found(format!("practitioner {id}"))),
        Err(e) => return error_to_response(Error::Database(e)),
    };
    let mut name: HumanName = serde_json::from_value(existing.name.clone()).unwrap_or(HumanName {
        use_type: None,
        family: String::new(),
        given: vec![],
        prefix: vec![],
        suffix: vec![],
    });
    if let Some(f) = req.family {
        name.family = f;
    }
    if let Some(g) = req.given {
        name.given = g;
    }
    let mut am: practitioner::ActiveModel = existing.into();
    am.name = Set(serde_json::to_value(&name).unwrap_or_default());
    if let Some(g) = req.gender {
        am.gender = Set(match g {
            Gender::Male => "male".into(),
            Gender::Female => "female".into(),
            Gender::Other => "other".into(),
            Gender::Unknown => "unknown".into(),
        });
    }
    if let Some(bd) = req.birth_date {
        am.birth_date = Set(Some(bd));
    }
    if let Some(active) = req.active {
        am.active = Set(active);
    }
    am.updated_at = Set(chrono::Utc::now().fixed_offset());
    let txn_res = state
        .db
        .transaction::<_, practitioner::Model, Error>(|txn| {
            Box::pin(async move {
                let m = am.update(txn).await.map_err(Error::Database)?;
                AuditLogRepository::log(
                    txn,
                    "practitioner",
                    id,
                    "update",
                    None,
                    Some(serde_json::to_value(&m).unwrap_or_default()),
                    &ctx,
                )
                .await?;
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
    match txn_res {
        Ok(m) => finish(Ok(m)),
        Err(sea_orm::TransactionError::Connection(c)) => error_to_response(Error::Database(c)),
        Err(sea_orm::TransactionError::Transaction(t)) => error_to_response(t),
    }
}

#[utoipa::path(delete, path = "/api/practitioners/{id}", tag = "Workforce",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Practitioner deactivated (active=false).")))]
pub async fn delete_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    use crate::db::entities::practitioner;
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set, TransactionTrait};
    let ctx = user_context_from_headers(&headers);
    let existing = match practitioner::Entity::find_by_id(id).one(&state.db).await {
        Ok(Some(m)) => m,
        Ok(None) => return error_to_response(Error::not_found(format!("practitioner {id}"))),
        Err(e) => return error_to_response(Error::Database(e)),
    };
    let mut am: practitioner::ActiveModel = existing.into();
    am.active = Set(false);
    am.updated_at = Set(chrono::Utc::now().fixed_offset());
    let txn_res = state
        .db
        .transaction::<_, (), Error>(|txn| {
            Box::pin(async move {
                am.update(txn).await.map_err(Error::Database)?;
                AuditLogRepository::log(
                    txn,
                    "practitioner",
                    id,
                    "deactivate",
                    None,
                    Some(serde_json::json!({ "active": false })),
                    &ctx,
                )
                .await?;
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
        Ok(()) => ok_response(serde_json::json!({ "id": id, "deactivated": true })),
        Err(sea_orm::TransactionError::Connection(c)) => error_to_response(Error::Database(c)),
        Err(sea_orm::TransactionError::Transaction(t)) => error_to_response(t),
    }
}

// ---------------------------------------------------------------------------
// Patient list
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PatientListQuery {
    #[serde(default = "default_patient_list_limit")]
    pub limit: u64,
}

fn default_patient_list_limit() -> u64 {
    50
}

#[utoipa::path(get, path = "/api/patients", tag = "Patient",
    params(("limit" = Option<u64>, Query, description = "Max rows, default 50, capped at 500")),
    responses((status = 200, description = "Active patients (non-deleted) up to `limit`.")))]
pub async fn list_patients(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<PatientListQuery>,
) -> HandlerResult {
    let limit = q.limit.min(500);
    match PatientRepository::list_active(&state.db, limit).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

// ---------------------------------------------------------------------------
// Consent
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateConsentRequest {
    #[schema(value_type = String)]
    pub consent_type: crate::models::consent::ConsentType,
    pub granted_date: chrono::NaiveDate,
    #[serde(default)]
    pub expiry_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
}

#[utoipa::path(post, path = "/api/patients/{id}/consents", tag = "Consent",
    params(("id" = Uuid, Path, description = "Patient id")),
    request_body = CreateConsentRequest,
    responses((status = 200, description = "Consent recorded.")))]
pub async fn create_consent(
    State(state): State<AppState>,
    Path(patient_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<CreateConsentRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    let mut c =
        crate::models::consent::Consent::new(patient_id, req.consent_type, req.granted_date);
    c.expiry_date = req.expiry_date;
    c.purpose = req.purpose;
    c.method = req.method;
    let created = match ConsentRepository::create(&state.db, &c).await {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let _ = AuditLogRepository::log(
        &state.db,
        "consent",
        created.id,
        "create",
        None,
        Some(serde_json::to_value(&created).unwrap_or_default()),
        &ctx,
    )
    .await;
    finish(Ok(created))
}

#[utoipa::path(get, path = "/api/patients/{id}/consents", tag = "Consent",
    params(("id" = Uuid, Path, description = "Patient id")),
    responses((status = 200, description = "All consent records (active + revoked + expired).")))]
pub async fn list_patient_consents(
    State(state): State<AppState>,
    Path(patient_id): Path<Uuid>,
) -> HandlerResult {
    match ConsentRepository::list_for_patient(&state.db, patient_id).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(post, path = "/api/consents/{id}/revoke", tag = "Consent",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Consent status flipped to Revoked.")))]
pub async fn revoke_consent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    match ConsentRepository::revoke(&state.db, id).await {
        Ok(c) => {
            let _ =
                AuditLogRepository::log(&state.db, "consent", id, "revoke", None, None, &ctx).await;
            finish(Ok(c))
        }
        Err(e) => error_to_response(e),
    }
}

// ---------------------------------------------------------------------------
// List endpoints for bootstrap entities
// ---------------------------------------------------------------------------

#[utoipa::path(get, path = "/api/facilities", tag = "FacilitySetup",
    responses((status = 200, description = "All facilities.")))]
pub async fn list_facilities(State(state): State<AppState>) -> HandlerResult {
    use crate::db::entities::facility;
    use sea_orm::EntityTrait;
    match facility::Entity::find().all(&state.db).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(Error::Database(e)),
    }
}

#[derive(Debug, Deserialize)]
pub struct WardListQuery {
    #[serde(default)]
    pub facility_id: Option<Uuid>,
}

#[utoipa::path(get, path = "/api/wards", tag = "FacilitySetup",
    params(("facility_id" = Option<Uuid>, Query, description = "Restrict to this facility")),
    responses((status = 200, description = "Wards, optionally filtered by facility.")))]
pub async fn list_wards(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<WardListQuery>,
) -> HandlerResult {
    use crate::db::entities::ward;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let result = match q.facility_id {
        Some(fid) => {
            ward::Entity::find()
                .filter(ward::Column::FacilityId.eq(fid))
                .all(&state.db)
                .await
        }
        None => ward::Entity::find().all(&state.db).await,
    };
    match result {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(Error::Database(e)),
    }
}

#[utoipa::path(get, path = "/api/practitioners", tag = "Workforce",
    responses((status = 200, description = "Active practitioners.")))]
pub async fn list_practitioners(State(state): State<AppState>) -> HandlerResult {
    use crate::db::entities::practitioner;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    match practitioner::Entity::find()
        .filter(practitioner::Column::Active.eq(true))
        .all(&state.db)
        .await
    {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(Error::Database(e)),
    }
}

// ---------------------------------------------------------------------------
// Waitlist update
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateWaitlistRequest {
    #[schema(value_type = String)]
    #[serde(default)]
    pub priority: Option<Priority>,
    #[schema(value_type = String)]
    #[serde(default)]
    pub status: Option<crate::models::waitlist::WaitlistStatus>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct RttBreach {
    pub pathway_id: Uuid,
    pub patient_id: Uuid,
    pub target_service: String,
    pub weeks_waiting: u32,
    pub breach_weeks: u32,
}

#[utoipa::path(get, path = "/api/waitlist/breaches", tag = "RTT",
    responses((status = 200, description = "Pathways whose active weeks exceed their breach threshold.", body = [RttBreach])))]
pub async fn list_rtt_breaches(State(state): State<AppState>) -> HandlerResult {
    use crate::db::entities::rtt_pathway;
    use crate::models::rtt::compute_active_weeks;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let pathways = match rtt_pathway::Entity::find()
        .filter(rtt_pathway::Column::Status.ne("stopped"))
        .all(&state.db)
        .await
    {
        Ok(v) => v,
        Err(e) => return error_to_response(Error::Database(e)),
    };
    let now = chrono::Utc::now();
    let mut breaches = Vec::new();
    for p in pathways {
        let events = match RttRepository::list_events_for_pathway(&state.db, p.id).await {
            Ok(v) => v,
            Err(e) => return error_to_response(e),
        };
        let weeks = compute_active_weeks(&events, now);
        let breach_weeks = p.breach_weeks.max(0) as u32;
        if weeks > breach_weeks {
            breaches.push(RttBreach {
                pathway_id: p.id,
                patient_id: p.patient_id,
                target_service: p.target_service,
                weeks_waiting: weeks,
                breach_weeks,
            });
        }
    }
    finish(Ok(breaches))
}

#[utoipa::path(put, path = "/api/waitlist/{id}", tag = "Waitlist",
    params(("id" = Uuid, Path)),
    request_body = UpdateWaitlistRequest,
    responses((status = 200, description = "Waitlist entry priority and/or status updated.")))]
pub async fn update_waitlist_entry(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<UpdateWaitlistRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    let mut entry = match WaitlistRepository::find_entry_by_id(&state.db, id).await {
        Ok(Some(e)) => e,
        Ok(None) => return error_to_response(Error::not_found(format!("waitlist {id}"))),
        Err(e) => return error_to_response(e),
    };
    if let Some(p) = req.priority {
        entry.priority = p;
    }
    if let Some(s) = req.status {
        entry.status = s;
    }
    entry.updated_at = chrono::Utc::now();
    match WaitlistRepository::update_entry(&state.db, &entry).await {
        Ok(e) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "waitlist_entry",
                id,
                "update",
                None,
                Some(serde_json::to_value(&e).unwrap_or_default()),
                &ctx,
            )
            .await;
            finish(Ok(e))
        }
        Err(e) => error_to_response(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_context_from_headers_populates_all_fields() {
        let mut h = HeaderMap::new();
        h.insert("X-User-Id", "u123".parse().unwrap());
        h.insert("X-User-Ip", "10.0.0.1".parse().unwrap());
        h.insert("X-User-Agent", "curl/8".parse().unwrap());
        let ctx = user_context_from_headers(&h);
        assert_eq!(ctx.user_id.as_deref(), Some("u123"));
        assert_eq!(ctx.user_ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(ctx.user_agent.as_deref(), Some("curl/8"));
    }

    #[test]
    fn test_user_context_from_headers_missing_is_none() {
        let h = HeaderMap::new();
        let ctx = user_context_from_headers(&h);
        assert!(ctx.user_id.is_none());
        assert!(ctx.user_ip.is_none());
        assert!(ctx.user_agent.is_none());
    }

    #[test]
    fn test_error_to_response_not_found_returns_404() {
        let (status, _) = error_to_response(Error::not_found("x"));
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_error_to_response_validation_returns_400() {
        let (status, _) = error_to_response(Error::validation("x"));
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_error_to_response_conflict_returns_409() {
        let (status, _) = error_to_response(Error::conflict("x"));
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[test]
    fn test_build_money_parses_decimal_and_currency() {
        let m = build_money("100.50", "USD").expect("valid money");
        assert_eq!(m.amount, Decimal::from_str("100.50").unwrap());
        assert_eq!(m.currency.0, "USD");
    }

    #[test]
    fn test_build_money_rejects_bad_currency() {
        assert!(build_money("100.00", "us").is_err());
    }

    #[test]
    fn test_build_money_rejects_bad_amount() {
        assert!(build_money("not-a-number", "USD").is_err());
    }
}

// ---------------------------------------------------------------------------
// HL7 v2 ADT — pipe-delimited message ingest
// ---------------------------------------------------------------------------

use crate::hl7v2::{
    AckCode, ack, message_type, parse_dft_p03, parse_merge_source_mrn, parse_message,
    parse_mfn_m02, parse_mfn_m05, parse_siu, patient_from_pid,
};
use crate::models::identifier::IdentifierType;

/// Resolve an inbound `PID` segment to a Patient row, deduplicating by exact
/// MRN match against the existing `patients` table.
///
/// Behavior:
/// - Project the PID into a domain `Patient` via [`patient_from_pid`].
/// - If PID-3.1 carries an MRN, look it up via
///   [`PatientRepository::find_by_identifier_value`]. If a non-deleted row
///   matches *and* the row's identifier set actually contains that MRN type,
///   reuse it — return `(existing, true)`.
/// - Otherwise validate + insert as a fresh row — return `(created, false)`.
///
/// On any failure (PID parse, validation, DB error), returns a tuple the
/// caller can fold into a v2 ACK: `(http_status, ack_code, diagnostic)`.
async fn dedup_or_create_patient_from_pid(
    state: &AppState,
    pid: &crate::hl7v2::Segment,
) -> std::result::Result<(crate::models::patient::Patient, bool), (StatusCode, AckCode, String)> {
    let mut projected = patient_from_pid(pid)
        .map_err(|e| (StatusCode::BAD_REQUEST, AckCode::AppError, e.to_string()))?;
    let mrn_value = projected
        .identifiers
        .iter()
        .find(|i| i.identifier_type == IdentifierType::MRN)
        .map(|i| i.value.clone());
    if let Some(value) = mrn_value {
        match PatientRepository::find_by_identifier_value(&state.db, &value).await {
            Ok(Some(existing))
                if existing
                    .identifiers
                    .iter()
                    .any(|i| i.identifier_type == IdentifierType::MRN && i.value == value) =>
            {
                return Ok((existing, true));
            }
            Ok(_) => {}
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    AckCode::Reject,
                    format!("MRN lookup: {e}"),
                ));
            }
        }
    }
    crate::validation::validate_patient(&projected)
        .map_err(|e| (StatusCode::BAD_REQUEST, AckCode::AppError, e.to_string()))?;
    projected.id = Uuid::new_v4();
    let now = chrono::Utc::now();
    projected.created_at = now;
    projected.updated_at = now;
    let created = PatientRepository::create(&state.db, &projected)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AckCode::Reject,
                format!("create patient: {e}"),
            )
        })?;
    if let Some(search) = &state.search {
        let _ = search.index_patient(&created);
    }
    Ok((created, false))
}

/// `POST /api/hl7/v2/parse` — parse an HL7 v2 message and return its
/// structured representation as JSON (useful for inspection and tooling).
#[utoipa::path(
    post,
    path = "/api/hl7/v2/parse",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "Parsed v2 message as JSON `{ segments: [{ name, fields }] }`."),
        (status = 400, description = "Body could not be parsed.")
    )
)]
pub async fn hl7_v2_parse(body: String) -> HandlerResult {
    match parse_message(&body) {
        Ok(m) => match serde_json::to_value(&m) {
            Ok(v) => ok_response(v),
            Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
        },
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/hl7/v2/patient` — ingest an ADT^A28 (or A01/A04/A08) message,
/// create a [`crate::models::patient::Patient`] from the `PID` segment, and
/// return a v2 `ACK^...` envelope in the response body.
///
/// Response: `text/plain` body with the ACK; HTTP status reflects whether the
/// PAS accepted (`200 + AA`), rejected the PID (`400 + AE`), or hit a DB
/// error (`500 + AR`).
#[utoipa::path(
    post,
    path = "/api/hl7/v2/patient",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — patient created.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE/AR ACK — message rejected.", body = String, content_type = "application/hl7-v2"),
        (status = 500, description = "AR ACK — server error.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_patient(
    State(state): State<AppState>,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());
    let (code, event) = message_type(&msg);
    if code != "ADT" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("unsupported message type {code}^{event}")),
            ),
        );
    }
    let pid = match msg.segment("PID") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PID segment"),
                ),
            );
        }
    };
    let (patient, was_dedup) = match dedup_or_create_patient_from_pid(&state, pid).await {
        Ok(v) => v,
        Err((status, code, diag)) => {
            return (
                status,
                [CT],
                ack("PAS", &sender, &control_id, code, Some(&diag)),
            );
        }
    };
    let diag = if was_dedup {
        Some(format!("matched existing patient {}", patient.id))
    } else {
        None
    };
    (
        StatusCode::OK,
        [CT],
        ack(
            "PAS",
            &sender,
            &control_id,
            AckCode::Accept,
            diag.as_deref(),
        ),
    )
}

/// `POST /api/hl7/v2/admit` — ingest an `ADT^A01` message: create (or assume
/// the existence of) a patient from `PID`, look up the destination bed by
/// the code in `PV1-3.3` (the *bed* sub-component of the assigned location),
/// and call [`crate::adt::AdtService::admit`].
///
/// Response: a v2 ACK envelope. `AA` on successful admission; `AE` on any
/// PAS-level rejection (bed unavailable, PV1 missing, bed code not found);
/// `AR` on parse failure.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/admit",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — patient admitted to the bed.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — message rejected (bed code unknown, PV1 missing, etc.).", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — bed unavailable.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(&msg);
    if code != "ADT" || event != "A01" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected ADT^A01, got {code}^{event}")),
            ),
        );
    }

    let pid = match msg.segment("PID") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PID segment"),
                ),
            );
        }
    };
    let pv1 = match msg.segment("PV1") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PV1 segment"),
                ),
            );
        }
    };

    // PV1-3 is the assigned patient location. PV1-3.3 is the bed sub-component
    // (point_of_care^room^bed^facility^…).
    let bed_code = pv1.component(3, 3);
    if bed_code.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some("PV1-3.3 (bed code) must not be empty for ADT^A01"),
            ),
        );
    }

    let bed = match BedRepository::find_by_code(&state.db, bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("bed code {bed_code:?} not found")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            );
        }
    };

    let (patient, was_dedup) = match dedup_or_create_patient_from_pid(&state, pid).await {
        Ok(v) => v,
        Err((status, code, diag)) => {
            return (
                status,
                [CT],
                ack("PAS", &sender, &control_id, code, Some(&diag)),
            );
        }
    };

    match state.adt.admit(patient.id, bed.id, &ctx).await {
        Ok(_) => {
            let diag = if was_dedup {
                Some(format!("matched existing patient {}", patient.id))
            } else {
                None
            };
            (
                StatusCode::OK,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Accept,
                    diag.as_deref(),
                ),
            )
        }
        Err(e) => (
            match &e {
                Error::Conflict(_) => StatusCode::CONFLICT,
                Error::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("admit: {e}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/register` — ingest an `ADT^A04` message (register an
/// outpatient or emergency-department patient): dedup-or-create the patient
/// from `PID`, then create an `Encounter` in `Arrived` status. No bed is
/// allocated — A04 is for ambulatory / ED / day-case visits. The encounter
/// class is derived from `PV1-2` (`E` → `Emergency`, anything else →
/// `Outpatient`); `PV1-7` is parsed as the attending practitioner id when
/// present and resolvable, otherwise dropped silently.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/register",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — patient registered.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — message rejected (PV1 missing, etc.).", body = String, content_type = "application/hl7-v2"),
    )
)]
pub async fn hl7_v2_register(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    use crate::db::repositories::outbox::OutboxRepository;
    use crate::models::encounter::{Encounter, EncounterClass, EncounterStatus};
    use sea_orm::TransactionTrait;
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());
    let (code, event) = message_type(&msg);
    if code != "ADT" || event != "A04" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected ADT^A04, got {code}^{event}")),
            ),
        );
    }
    let pid = match msg.segment("PID") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PID segment"),
                ),
            );
        }
    };
    let pv1 = match msg.segment("PV1") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PV1 segment"),
                ),
            );
        }
    };
    // PV1-2 (Patient Class): "E" → Emergency, anything else (incl. "O",
    // empty, unknown) → Outpatient. A04 is by definition non-inpatient.
    let class = match pv1.field(2).trim() {
        "E" => EncounterClass::Emergency,
        _ => EncounterClass::Outpatient,
    };
    let reason = {
        // PV1-4 is admit type, but A04 commonly carries a chief-complaint
        // free-text in PV1-12 (preadmit test indicator) or PV1-7 attending
        // doctor. Prefer empty over inventing — let downstream observation
        // messages carry chief complaint.
        let v = pv1.field(4).trim();
        if v.is_empty() {
            None
        } else {
            Some(v.to_string())
        }
    };
    let (patient, was_dedup) = match dedup_or_create_patient_from_pid(&state, pid).await {
        Ok(v) => v,
        Err((status, code, diag)) => {
            return (
                status,
                [CT],
                ack("PAS", &sender, &control_id, code, Some(&diag)),
            );
        }
    };
    let mut enc = Encounter::new(patient.id, class);
    enc.status = EncounterStatus::Arrived;
    enc.reason = reason;
    let txn_res = state
        .db
        .transaction::<_, Encounter, Error>(|txn| {
            Box::pin(async move {
                let saved = EncounterRepository::create(txn, &enc).await?;
                AuditLogRepository::log(
                    txn,
                    "encounter",
                    saved.id,
                    "register_via_hl7v2_a04",
                    None,
                    Some(serde_json::json!({
                        "class": format!("{:?}", saved.class),
                        "status": format!("{:?}", saved.status),
                    })),
                    &ctx,
                )
                .await?;
                OutboxRepository::publish(
                    txn,
                    "EncounterRegistered",
                    &serde_json::json!({
                        "encounter_id": saved.id,
                        "patient_id": saved.patient_id,
                        "class": format!("{:?}", saved.class),
                        "source": "hl7v2_a04",
                    }),
                )
                .await?;
                Ok(saved)
            })
        })
        .await;
    match txn_res {
        Ok(saved) => {
            let diag = if was_dedup {
                format!(
                    "matched existing patient {} encounter={}",
                    patient.id, saved.id
                )
            } else {
                format!("created patient {} encounter={}", patient.id, saved.id)
            };
            (
                StatusCode::OK,
                [CT],
                ack("PAS", &sender, &control_id, AckCode::Accept, Some(&diag)),
            )
        }
        Err(sea_orm::TransactionError::Connection(c)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::Reject,
                Some(&c.to_string()),
            ),
        ),
        Err(sea_orm::TransactionError::Transaction(t)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("register: {t}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/cancel-pre-admit` — ingest an `ADT^A38` (cancel
/// pre-admit) message: release the bed reservation set by a prior
/// `ADT^A05` (or a REST `pre_admit`), and cancel the corresponding
/// Planned inpatient encounter. The patient is identified by PID-3.1
/// (MRN, must already exist — no dedup-or-create on cancel), the bed
/// by `PV1-3.3`. (*v0.34*)
#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-pre-admit",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — reservation released, encounter cancelled.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — message rejected.", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — bed / patient / planned encounter not found.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — bed is not Reserved.", body = String, content_type = "application/hl7-v2"),
    )
)]
pub async fn hl7_v2_cancel_pre_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());
    let (code, event) = message_type(&msg);
    if code != "ADT" || event != "A38" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected ADT^A38, got {code}^{event}")),
            ),
        );
    }
    let pid = match msg.segment("PID") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PID segment"),
                ),
            );
        }
    };
    let pv1 = match msg.segment("PV1") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PV1 segment"),
                ),
            );
        }
    };
    let mrn = pid.component(3, 1);
    if mrn.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some("PID-3.1 (MRN) must not be empty"),
            ),
        );
    }
    let bed_code = pv1.component(3, 3);
    if bed_code.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some("PV1-3.3 (bed code) must not be empty for ADT^A38"),
            ),
        );
    }
    let patient = match PatientRepository::find_by_identifier_value(&state.db, mrn).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("patient with MRN {mrn:?} not found")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    let bed = match BedRepository::find_by_code(&state.db, bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("bed code {bed_code:?} not found")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    match state
        .adt
        .cancel_pre_admit(patient.id, bed.id, Some("hl7v2_a38"), &ctx)
        .await
    {
        Ok(enc) => (
            StatusCode::OK,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::Accept,
                Some(&format!("encounter={} bed={} released", enc.id, bed.id)),
            ),
        ),
        Err(e) => (
            match &e {
                Error::Conflict(_) => StatusCode::CONFLICT,
                Error::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("cancel pre-admit: {e}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/pre-admit` — ingest an `ADT^A05` (pre-admit
/// patient) message: dedup-or-create the patient from `PID`, look up
/// the destination bed by `PV1-3.3`, reserve it (status → `Reserved`),
/// and open an inpatient `Encounter` in `Planned` status. No
/// `Admission` row or `BedAssignment` is created — the patient hasn't
/// physically arrived yet. (*v0.32*)
#[utoipa::path(
    post,
    path = "/api/hl7/v2/pre-admit",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — bed reserved, planned encounter created.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — message rejected (PV1 missing, bed code missing).", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — bed not found.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — bed not available for reservation.", body = String, content_type = "application/hl7-v2"),
    )
)]
pub async fn hl7_v2_pre_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());
    let (code, event) = message_type(&msg);
    if code != "ADT" || event != "A05" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected ADT^A05, got {code}^{event}")),
            ),
        );
    }
    let pid = match msg.segment("PID") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PID segment"),
                ),
            );
        }
    };
    let pv1 = match msg.segment("PV1") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PV1 segment"),
                ),
            );
        }
    };
    let bed_code = pv1.component(3, 3);
    if bed_code.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some("PV1-3.3 (bed code) must not be empty for ADT^A05"),
            ),
        );
    }
    let bed = match BedRepository::find_by_code(&state.db, bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("bed code {bed_code:?} not found")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    let (patient, was_dedup) = match dedup_or_create_patient_from_pid(&state, pid).await {
        Ok(v) => v,
        Err((status, code, diag)) => {
            return (
                status,
                [CT],
                ack("PAS", &sender, &control_id, code, Some(&diag)),
            );
        }
    };
    match state
        .adt
        .pre_admit(patient.id, bed.id, Some("hl7v2_a05"), &ctx)
        .await
    {
        Ok(enc) => {
            let diag = if was_dedup {
                format!(
                    "matched existing patient {} encounter={} bed={}",
                    patient.id, enc.id, bed.id
                )
            } else {
                format!(
                    "created patient {} encounter={} bed={}",
                    patient.id, enc.id, bed.id
                )
            };
            (
                StatusCode::OK,
                [CT],
                ack("PAS", &sender, &control_id, AckCode::Accept, Some(&diag)),
            )
        }
        Err(e) => (
            match &e {
                Error::Conflict(_) => StatusCode::CONFLICT,
                Error::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("pre-admit: {e}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/change-to-outpatient` — ingest an `ADT^A07`
/// (change inpatient to outpatient). Locates the patient's
/// currently-open admission by PID-3.1 (MRN), releases the bed
/// assignment (bed → `Cleaning`), and reclassifies the encounter
/// from `Inpatient` to `Outpatient`. The encounter status stays
/// `InProgress` — the patient is still in active care. The
/// `admissions` row is preserved as historical record. (*v0.43*)
#[utoipa::path(
    post,
    path = "/api/hl7/v2/change-to-outpatient",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — encounter demoted to outpatient.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — message rejected.", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — MRN unknown or no open admission.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_change_to_outpatient(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let (admission_id, sender, control_id) =
        match locate_open_admission_for_v2(&state, &msg, "A07").await {
            Ok(v) => v,
            Err(resp) => return resp,
        };
    match state
        .adt
        .change_to_outpatient(admission_id, Some("hl7v2_a07"), &ctx)
        .await
    {
        Ok(enc) => (
            StatusCode::OK,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::Accept,
                Some(&format!(
                    "encounter={} admission={} demoted to outpatient",
                    enc.id, admission_id
                )),
            ),
        ),
        Err(e) => (
            match &e {
                Error::Conflict(_) | Error::InvalidStateTransition(_) => StatusCode::CONFLICT,
                Error::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("change to outpatient: {e}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/change-to-inpatient` — ingest an `ADT^A06`
/// (change outpatient to inpatient). Looks up the patient by
/// PID-3.1 (MRN), finds their currently-active Outpatient /
/// Emergency encounter, allocates the bed from PV1-3.3 (must be
/// `Available`), reclassifies the encounter to `Inpatient`, and
/// writes a fresh `Admission` row + `BedAssignment`. (*v0.41*)
#[utoipa::path(
    post,
    path = "/api/hl7/v2/change-to-inpatient",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — encounter promoted to inpatient.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — message rejected.", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — patient / bed / ambulatory encounter not found.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — bed not available.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_change_to_inpatient(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());
    let (code, event) = message_type(&msg);
    if code != "ADT" || event != "A06" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected ADT^A06, got {code}^{event}")),
            ),
        );
    }
    let pid = match msg.segment("PID") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PID segment"),
                ),
            );
        }
    };
    let pv1 = match msg.segment("PV1") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PV1 segment"),
                ),
            );
        }
    };
    let mrn = pid.component(3, 1);
    if mrn.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some("PID-3.1 (MRN) must not be empty"),
            ),
        );
    }
    let bed_code = pv1.component(3, 3);
    if bed_code.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some("PV1-3.3 (bed code) must not be empty for ADT^A06"),
            ),
        );
    }
    let patient = match PatientRepository::find_by_identifier_value(&state.db, mrn).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("patient with MRN {mrn:?} not found")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    let bed = match BedRepository::find_by_code(&state.db, bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("bed code {bed_code:?} not found")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    match state
        .adt
        .change_to_inpatient(patient.id, bed.id, Some("hl7v2_a06"), &ctx)
        .await
    {
        Ok(res) => (
            StatusCode::OK,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::Accept,
                Some(&format!(
                    "encounter={} admission={} bed={}",
                    res.encounter.id, res.admission.id, bed.id
                )),
            ),
        ),
        Err(e) => (
            match &e {
                Error::Conflict(_) => StatusCode::CONFLICT,
                Error::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("change to inpatient: {e}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/delete-patient` — ingest an `ADT^A23` (delete
/// a patient record). Looks up the patient by PID-3.1 (MRN), refuses
/// (409 + AE) if the patient has any open admission, otherwise
/// soft-deletes the patient row (`deleted_at` set), drops the
/// patient from the Tantivy search index, and writes a
/// `PatientDeleted` outbox event tagged `source: "hl7v2_a23"`.
/// A23 is intended for "this patient record was created in error"
/// workflows — PAS will not destroy a patient who currently has an
/// active stay. (*v0.39*)
#[utoipa::path(
    post,
    path = "/api/hl7/v2/delete-patient",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — patient soft-deleted.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — message rejected.", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — patient not found.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — patient has an open admission.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_delete_patient(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::TransactionTrait;
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());
    let (code, event) = message_type(&msg);
    if code != "ADT" || event != "A23" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected ADT^A23, got {code}^{event}")),
            ),
        );
    }
    let pid = match msg.segment("PID") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PID segment"),
                ),
            );
        }
    };
    let mrn = pid.component(3, 1);
    if mrn.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some("PID-3.1 (MRN) must not be empty"),
            ),
        );
    }
    let patient = match PatientRepository::find_by_identifier_value(&state.db, mrn).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("patient with MRN {mrn:?} not found")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    // Safety check: refuse to delete a patient who has any open
    // admission. A23 is for records created in error, not for
    // discharging real patients.
    match AdmissionRepository::find_open_for_patient(&state.db, patient.id).await {
        Ok(Some(adm)) => {
            return (
                StatusCode::CONFLICT,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!(
                        "patient {} has open admission {}; refuse to delete",
                        patient.id, adm.id
                    )),
                ),
            );
        }
        Ok(None) => {}
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            );
        }
    }
    let patient_id = patient.id;
    let txn_res = state
        .db
        .transaction::<_, (), Error>(|txn| {
            Box::pin(async move {
                PatientRepository::soft_delete(txn, patient_id).await?;
                AuditLogRepository::log(
                    txn,
                    "patient",
                    patient_id,
                    "delete_via_hl7v2_a23",
                    None,
                    None,
                    &ctx,
                )
                .await?;
                OutboxRepository::publish(
                    txn,
                    "PatientDeleted",
                    &serde_json::json!({
                        "patient_id": patient_id,
                        "source": "hl7v2_a23",
                    }),
                )
                .await?;
                Ok(())
            })
        })
        .await;
    match txn_res {
        Ok(()) => {
            if let Some(search) = &state.search {
                let _ = search.delete_patient(patient_id);
            }
            (
                StatusCode::OK,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Accept,
                    Some(&format!("patient {patient_id} soft-deleted")),
                ),
            )
        }
        Err(sea_orm::TransactionError::Connection(c)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::Reject,
                Some(&c.to_string()),
            ),
        ),
        Err(sea_orm::TransactionError::Transaction(t)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("delete patient: {t}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/leave-start` — ingest an `ADT^A21` (patient
/// goes on leave of absence). Locates the patient's currently-open
/// admission by PID-3.1 (MRN), then transitions the encounter from
/// `InProgress` to `OnLeave`. The bed remains `Occupied`. (*v0.36*)
#[utoipa::path(
    post,
    path = "/api/hl7/v2/leave-start",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — encounter on leave.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — MRN unknown or no open admission.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — encounter not in InProgress.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_leave_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    handle_loa_transition(&state, &headers, &body, "A21", "leave start", true).await
}

/// `POST /api/hl7/v2/leave-end` — ingest an `ADT^A22` (patient
/// returns from leave of absence). Locates the patient's
/// currently-open admission by PID-3.1 (MRN), then transitions the
/// encounter from `OnLeave` back to `InProgress`. (*v0.36*)
#[utoipa::path(
    post,
    path = "/api/hl7/v2/leave-end",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — encounter back in progress.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — MRN unknown or no open admission.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — encounter not OnLeave.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_leave_end(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    handle_loa_transition(&state, &headers, &body, "A22", "leave end", false).await
}

async fn handle_loa_transition(
    state: &AppState,
    headers: &HeaderMap,
    body: &str,
    expected_event: &str,
    diag_label: &str,
    is_start: bool,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(headers);
    let msg = match parse_message(body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let (admission_id, sender, control_id) =
        match locate_open_admission_for_v2(state, &msg, expected_event).await {
            Ok(v) => v,
            Err(resp) => return resp,
        };
    let source_tag = if is_start { "hl7v2_a21" } else { "hl7v2_a22" };
    let result = if is_start {
        state
            .adt
            .start_leave(admission_id, Some(source_tag), &ctx)
            .await
    } else {
        state
            .adt
            .end_leave(admission_id, Some(source_tag), &ctx)
            .await
    };
    match result {
        Ok(_) => (
            StatusCode::OK,
            [CT],
            ack("PAS", &sender, &control_id, AckCode::Accept, None),
        ),
        Err(e) => (
            match &e {
                Error::Conflict(_) | Error::InvalidStateTransition(_) => StatusCode::CONFLICT,
                Error::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("{diag_label}: {e}")),
            ),
        ),
    }
}

use crate::db::repositories::admission::AdmissionRepository;

/// Result of locating an existing admission from an inbound ADT^A02/A03
/// message. Either yields the admission UUID or an HTTP response carrying
/// the appropriate AE ACK.
async fn locate_open_admission_for_v2(
    state: &AppState,
    msg: &crate::hl7v2::Message,
    expected_event: &str,
) -> std::result::Result<
    (Uuid, String, String),
    (
        StatusCode,
        [(axum::http::header::HeaderName, &'static str); 1],
        String,
    ),
> {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(msg);
    if code != "ADT" || event != expected_event {
        return Err((
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!(
                    "expected ADT^{expected_event}, got {code}^{event}"
                )),
            ),
        ));
    }
    let pid = match msg.segment("PID") {
        Some(p) => p,
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PID segment"),
                ),
            ));
        }
    };
    let mrn = pid.component(3, 1);
    if mrn.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some("PID-3.1 (MRN) must not be empty"),
            ),
        ));
    }
    let patient = match PatientRepository::find_by_identifier_value(&state.db, mrn).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("no patient found for MRN {mrn:?}")),
                ),
            ));
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            ));
        }
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, patient.id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!(
                        "patient {} has no currently-open admission",
                        patient.id
                    )),
                ),
            ));
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            ));
        }
    };
    Ok((admission.id, sender, control_id))
}

/// `POST /api/hl7/v2/transfer` — ingest an `ADT^A02` (patient transfer).
/// Identifies the open admission by MRN (PID-3.1) and transfers the patient
/// to the bed whose code matches `PV1-3.3`.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/transfer",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — patient transferred to the destination bed.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — PID/PV1 missing, MRN unknown, or no open admission.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — destination bed unavailable.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let (admission_id, sender, control_id) =
        match locate_open_admission_for_v2(&state, &msg, "A02").await {
            Ok(v) => v,
            Err(resp) => return resp,
        };

    let pv1 = match msg.segment("PV1") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PV1 segment"),
                ),
            );
        }
    };
    let bed_code = pv1.component(3, 3);
    if bed_code.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some("PV1-3.3 (destination bed code) must not be empty"),
            ),
        );
    }
    let new_bed = match BedRepository::find_by_code(&state.db, bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("destination bed code {bed_code:?} not found")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            );
        }
    };

    match state.adt.transfer(admission_id, new_bed.id, &ctx).await {
        Ok(_) => (
            StatusCode::OK,
            [CT],
            ack("PAS", &sender, &control_id, AckCode::Accept, None),
        ),
        Err(e) => (
            match &e {
                Error::Conflict(_) => StatusCode::CONFLICT,
                Error::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("transfer: {e}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/discharge` — ingest an `ADT^A03` (patient discharge).
/// Identifies the open admission by MRN (PID-3.1) and discharges the
/// patient. The PV1 segment is allowed but ignored.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/discharge",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — patient discharged.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — MRN unknown or no open admission.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_discharge(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let (admission_id, sender, control_id) =
        match locate_open_admission_for_v2(&state, &msg, "A03").await {
            Ok(v) => v,
            Err(resp) => return resp,
        };

    match state.adt.discharge(admission_id, &ctx).await {
        Ok(_) => (
            StatusCode::OK,
            [CT],
            ack("PAS", &sender, &control_id, AckCode::Accept, None),
        ),
        Err(e) => (
            match &e {
                Error::Conflict(_) => StatusCode::CONFLICT,
                Error::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("discharge: {e}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/update` — ingest an `ADT^A08` (update patient
/// information). Looks up the patient by PID-3.1 (MRN), projects a fresh
/// [`crate::models::patient::Patient`] from the inbound PID, and merges
/// the projected fields over the existing record:
///
/// **Overwritten** by A08: `identifiers`, `name`, `telecom`, `gender`,
/// `birth_date`, `addresses`, `updated_at`.
///
/// **Preserved** by A08: `id`, `mpi_id`, `additional_names`, `deceased`,
/// `deceased_datetime`, `emergency_contacts`, `marital_status`,
/// `created_at`, `active`. PID has no field for any of these.
///
/// On success: the patient row is updated, the search index is
/// refreshed, an `audit_log` row (`action = "update_via_hl7v2_a08"`) and
/// an `outbox_events` row (`event_type = "PatientUpdated"`) are written
/// in the same DB transaction, and an AA ACK is returned with diagnostic
/// `updated patient <uuid>`.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/update",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — patient updated.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE/AR ACK — message rejected (PID missing, MRN unknown, validation failure).", body = String, content_type = "application/hl7-v2"),
        (status = 500, description = "AR ACK — server error.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(&msg);
    if code != "ADT" || event != "A08" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected ADT^A08, got {code}^{event}")),
            ),
        );
    }
    let pid = match msg.segment("PID") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PID segment"),
                ),
            );
        }
    };

    // Project the inbound PID into a fresh Patient; we'll use it as a
    // source of overwriteable fields.
    let projected = match patient_from_pid(pid) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    let mrn = pid.component(3, 1);
    if mrn.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some("PID-3.1 (MRN) must not be empty for ADT^A08"),
            ),
        );
    }
    let existing = match PatientRepository::find_by_identifier_value(&state.db, mrn).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("no patient found for MRN {mrn:?}")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            );
        }
    };

    // Merge: PID-sourced fields replace the existing values; the rest
    // (mpi_id, additional_names, deceased*, emergency_contacts,
    // marital_status, created_at, active, id) are preserved.
    let mut merged = existing.clone();
    merged.identifiers = projected.identifiers;
    merged.name = projected.name;
    merged.telecom = projected.telecom;
    merged.gender = projected.gender;
    merged.birth_date = projected.birth_date;
    merged.addresses = projected.addresses;
    merged.updated_at = chrono::Utc::now();
    if let Err(e) = crate::validation::validate_patient(&merged) {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&e.to_string()),
            ),
        );
    }

    // Persist the update + audit + outbox in one transaction.
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::TransactionTrait;
    let merged_for_txn = merged.clone();
    let ctx_for_txn = ctx.clone();
    let txn_res = state
        .db
        .transaction::<_, crate::models::patient::Patient, Error>(|txn| {
            Box::pin(async move {
                let updated = PatientRepository::update(txn, &merged_for_txn).await?;
                AuditLogRepository::log(
                    txn,
                    "patient",
                    updated.id,
                    "update_via_hl7v2_a08",
                    None,
                    Some(serde_json::to_value(&updated).unwrap_or_default()),
                    &ctx_for_txn,
                )
                .await?;
                OutboxRepository::publish(
                    txn,
                    "PatientUpdated",
                    &serde_json::json!({
                        "patient_id": updated.id,
                        "source": "hl7v2_a08",
                    }),
                )
                .await?;
                Ok(updated)
            })
        })
        .await;
    let updated = match txn_res {
        Ok(p) => p,
        Err(sea_orm::TransactionError::Connection(c)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&c.to_string()),
                ),
            );
        }
        Err(sea_orm::TransactionError::Transaction(t)) => {
            return (
                match &t {
                    Error::NotFound(_) => StatusCode::NOT_FOUND,
                    Error::Validation(_) => StatusCode::BAD_REQUEST,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                },
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&t.to_string()),
                ),
            );
        }
    };
    if let Some(search) = &state.search {
        let _ = search.index_patient(&updated);
    }

    (
        StatusCode::OK,
        [CT],
        ack(
            "PAS",
            &sender,
            &control_id,
            AckCode::Accept,
            Some(&format!("updated patient {}", updated.id)),
        ),
    )
}

/// `POST /api/hl7/v2/cancel-admit` — ingest an `ADT^A11` (cancel admit /
/// visit notification). Identifies the patient's currently-open admission
/// by PID-3.1 (MRN) and reverses the admit: releases the active bed
/// assignment, flips the bed to `Cleaning` (same as discharge), and moves
/// the encounter to `Cancelled`. The `admissions` row is preserved so the
/// erroneous admit is still visible in the patient's history.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-admit",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — admission cancelled.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — MRN unknown or no open admission.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_cancel_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let (admission_id, sender, control_id) =
        match locate_open_admission_for_v2(&state, &msg, "A11").await {
            Ok(v) => v,
            Err(resp) => return resp,
        };

    match state
        .adt
        .cancel_admission(admission_id, Some("hl7v2_a11"), &ctx)
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            [CT],
            ack("PAS", &sender, &control_id, AckCode::Accept, None),
        ),
        Err(e) => (
            match &e {
                Error::Conflict(_) => StatusCode::CONFLICT,
                Error::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("cancel admit: {e}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/cancel-transfer` — ingest an `ADT^A12` (cancel
/// transfer). Identifies the patient's currently-open admission by
/// PID-3.1 (MRN) and reverses the most-recent transfer: releases the
/// active bed assignment on the destination bed, locks the origin bed
/// (must be `Available` or `Cleaning`), restores the patient to the
/// origin bed, and physically deletes the cancelled `transfers` row.
///
/// Constraints: the admission must have at least one transfer history
/// (`404 + AE` if not); the origin bed must currently be `Available`
/// or `Cleaning` (`409 + AE` if not). (*v0.30*)
#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-transfer",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — transfer cancelled, patient restored to origin bed.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — MRN unknown or no open admission.", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — admission has no transfer to cancel.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — origin bed is no longer available for restoration.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_cancel_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let (admission_id, sender, control_id) =
        match locate_open_admission_for_v2(&state, &msg, "A12").await {
            Ok(v) => v,
            Err(resp) => return resp,
        };

    match state
        .adt
        .cancel_transfer(admission_id, Some("hl7v2_a12"), &ctx)
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            [CT],
            ack("PAS", &sender, &control_id, AckCode::Accept, None),
        ),
        Err(e) => (
            match &e {
                Error::Conflict(_) => StatusCode::CONFLICT,
                Error::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("cancel transfer: {e}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/cancel-discharge` — ingest an `ADT^A13` (cancel
/// discharge). Identifies the patient's most-recently-discharged admission
/// by PID-3.1 (MRN), reinstates it, and places the patient back in the
/// original bed.
///
/// Constraints: the original bed must currently be in `Available` or
/// `Cleaning` status. Any other status (`Occupied` — someone else took
/// the bed; `Reserved`; `OutOfService`) means the discharge cannot be
/// safely undone — the handler returns `AE` with a diagnostic and the
/// PAS state is unchanged.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-discharge",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — discharge cancelled, patient reinstated.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — MRN unknown or patient has no discharge to cancel.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — original bed is no longer available for reinstatement.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_cancel_discharge(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(&msg);
    if code != "ADT" || event != "A13" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected ADT^A13, got {code}^{event}")),
            ),
        );
    }
    let pid = match msg.segment("PID") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PID segment"),
                ),
            );
        }
    };
    let mrn = pid.component(3, 1);
    if mrn.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some("PID-3.1 (MRN) must not be empty"),
            ),
        );
    }
    let patient = match PatientRepository::find_by_identifier_value(&state.db, mrn).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("no patient found for MRN {mrn:?}")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    let (admission, _discharge) =
        match AdmissionRepository::find_most_recently_discharged_for_patient(&state.db, patient.id)
            .await
        {
            Ok(Some(pair)) => pair,
            Ok(None) => {
                return (
                    StatusCode::BAD_REQUEST,
                    [CT],
                    ack(
                        "PAS",
                        &sender,
                        &control_id,
                        AckCode::AppError,
                        Some(&format!(
                            "patient {} has no discharge to cancel",
                            patient.id
                        )),
                    ),
                );
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [CT],
                    ack(
                        "PAS",
                        &sender,
                        &control_id,
                        AckCode::Reject,
                        Some(&e.to_string()),
                    ),
                );
            }
        };

    match state.adt.cancel_discharge(admission.id, &ctx).await {
        Ok(_) => (
            StatusCode::OK,
            [CT],
            ack("PAS", &sender, &control_id, AckCode::Accept, None),
        ),
        Err(e) => (
            match &e {
                Error::Conflict(_) => StatusCode::CONFLICT,
                Error::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("cancel discharge: {e}")),
            ),
        ),
    }
}

/// `POST /api/hl7/v2/schedule-book` — ingest an `SIU^S12` (notification of
/// new appointment) message: parse the patient off `PID` (dedup-on-MRN like
/// `A01/A28`), build an [`crate::models::appointment::Appointment`] from
/// the SCH segment (start datetime in `SCH-11`, duration in `SCH-9`,
/// reason in `SCH-7`), and persist via [`AppointmentRepository::create`].
///
/// The PAS appointment UUID is reported in the AA ACK's MSA-3 diagnostic
/// so the sender can record it as the filler appointment id. Returns
/// `400 + AE` if the patient projection fails, `409 + AE` if the patient
/// already has an overlapping appointment, `400 + AR` on parse failure.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-book",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — appointment booked; MSA-3 carries `filler=<uuid>`.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — SIU body rejected (missing SCH/PID, bad datetime).", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — patient has an overlapping appointment.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_schedule_book(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(&msg);
    if code != "SIU" || event != "S12" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected SIU^S12, got {code}^{event}")),
            ),
        );
    }
    let siu = match parse_siu(&msg) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    let pid = msg.segment("PID").expect("parse_siu would have failed");
    let (patient, _was_dedup) = match dedup_or_create_patient_from_pid(&state, pid).await {
        Ok(v) => v,
        Err((status, code, diag)) => {
            return (
                status,
                [CT],
                ack("PAS", &sender, &control_id, code, Some(&diag)),
            );
        }
    };
    let start = siu.start_datetime.expect("S12 always carries start");
    let end = siu.end_datetime.expect("S12 always carries end");

    // Block double-booking before insert. Matches `SchedulingService::book_slot`
    // semantics but without the slot constraint — SIU bookings often arrive
    // for slots that don't exist in PAS.
    match AppointmentRepository::find_overlapping_for_patient(&state.db, patient.id, start, end)
        .await
    {
        Ok(rows) if !rows.is_empty() => {
            return (
                StatusCode::CONFLICT,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!(
                        "patient {} has an overlapping appointment",
                        patient.id
                    )),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("overlap check: {e}")),
                ),
            );
        }
        _ => {}
    }

    let mut appt = crate::models::appointment::Appointment::new(patient.id, start, end);
    appt.status = crate::models::appointment::AppointmentStatus::Booked;
    appt.reason = siu.reason.clone();
    let created = match AppointmentRepository::create(&state.db, &appt).await {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("create appointment: {e}")),
                ),
            );
        }
    };
    let _ = AuditLogRepository::log(
        &state.db,
        "appointment",
        created.id,
        "book_via_hl7v2_s12",
        None,
        Some(serde_json::to_value(&created).unwrap_or_default()),
        &ctx,
    )
    .await;
    let _ = crate::db::repositories::outbox::OutboxRepository::publish(
        &state.db,
        "AppointmentBooked",
        &serde_json::json!({
            "appointment_id": created.id,
            "patient_id": patient.id,
            "start": created.start_datetime,
            "end": created.end_datetime,
            "source": "hl7v2_s12",
        }),
    )
    .await;
    let diag = format!(
        "appointment booked filler={} placer={}",
        created.id,
        siu.placer_appointment_id.as_deref().unwrap_or("")
    );
    (
        StatusCode::OK,
        [CT],
        ack("PAS", &sender, &control_id, AckCode::Accept, Some(&diag)),
    )
}

/// `POST /api/hl7/v2/schedule-cancel` — ingest an `SIU^S15` (notification
/// of appointment cancellation). Looks up the target appointment by the
/// filler id (`SCH-2`, a PAS UUID), flips its status to `Cancelled` via
/// [`AppointmentRepository::set_status_and_reason`], and writes the
/// usual audit + outbox.
///
/// Returns `400 + AE` if `SCH-2` is missing or not a UUID, `404 + AE`
/// if no PAS appointment matches, `409 + AE` if the appointment is in
/// a terminal state (`Cancelled`, `Fulfilled`, `NoShow`) and cannot be
/// re-cancelled.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-cancel",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — appointment cancelled.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — SIU body rejected (bad SCH-2 filler id).", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — no PAS appointment with that filler id.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — appointment is in a terminal state.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_schedule_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(&msg);
    if code != "SIU" || event != "S15" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected SIU^S15, got {code}^{event}")),
            ),
        );
    }
    let siu = match parse_siu(&msg) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    let filler = match siu.filler_appointment_id.as_deref() {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("SIU^S15 requires SCH-2 (filler appointment id)"),
                ),
            );
        }
    };
    let appt_id = match Uuid::parse_str(filler) {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("SCH-2 must be a PAS appointment UUID: {e}")),
                ),
            );
        }
    };

    // Pre-check existence + status before flipping, so we can return a
    // precise 404 / 409 rather than the repo's generic NotFound.
    let existing = match AppointmentRepository::find_by_id(&state.db, appt_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("no PAS appointment with filler id {appt_id}")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("lookup: {e}")),
                ),
            );
        }
    };
    use crate::models::appointment::AppointmentStatus;
    if matches!(
        existing.status,
        AppointmentStatus::Cancelled | AppointmentStatus::Fulfilled | AppointmentStatus::NoShow
    ) {
        return (
            StatusCode::CONFLICT,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!(
                    "appointment {appt_id} already in terminal status {:?}",
                    existing.status
                )),
            ),
        );
    }

    let cancelled = match AppointmentRepository::set_status_and_reason(
        &state.db,
        appt_id,
        AppointmentStatus::Cancelled,
        Some(CancellationReason::Other),
    )
    .await
    {
        Ok(Some(a)) => a,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("no PAS appointment with filler id {appt_id}")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("cancel: {e}")),
                ),
            );
        }
    };
    let _ = AuditLogRepository::log(
        &state.db,
        "appointment",
        cancelled.id,
        "cancel_via_hl7v2_s15",
        None,
        Some(serde_json::to_value(&cancelled).unwrap_or_default()),
        &ctx,
    )
    .await;
    let _ = crate::db::repositories::outbox::OutboxRepository::publish(
        &state.db,
        "AppointmentCancelled",
        &serde_json::json!({
            "appointment_id": cancelled.id,
            "patient_id": cancelled.patient_id,
            "source": "hl7v2_s15",
            "reason": siu.reason,
        }),
    )
    .await;
    let diag = format!("appointment {} cancelled", cancelled.id);
    (
        StatusCode::OK,
        [CT],
        ack("PAS", &sender, &control_id, AckCode::Accept, Some(&diag)),
    )
}

/// `POST /api/hl7/v2/merge` — ingest an `ADT^A40` (merge patient — patient
/// ID) message: parse the **survivor** patient off `PID` (dedup-on-MRN
/// like A01/A28 — created on the fly when unknown), locate the **source**
/// patient by the MRN in `MRG-1.1`, and apply the same merge logic that
/// powers `POST /api/patients/{id}/merge-into/{target_id}` (set
/// `replaced_by = survivor.id`, audit, outbox). The Tantivy index is
/// best-effort dropped post-commit so the source row stops appearing in
/// search results.
///
/// Returns `400 + AE` when MRG is missing / empty, `404 + AE` when no
/// PAS patient matches `MRG-1.1`, `409 + AE` if the source is already a
/// tombstone or if survivor and source resolve to the same row.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/merge",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — patient merged.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — ADT body rejected.", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — source MRN not found.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — source already merged, or self-merge.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_merge(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    use crate::db::repositories::outbox::OutboxRepository;
    use crate::models::identifier::IdentifierType;
    use sea_orm::TransactionTrait;
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(&msg);
    if code != "ADT" || event != "A40" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected ADT^A40, got {code}^{event}")),
            ),
        );
    }
    let pid = match msg.segment("PID") {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some("missing PID segment"),
                ),
            );
        }
    };
    let source_mrn = match parse_merge_source_mrn(&msg) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    // Survivor: dedup-on-MRN creates a row when the PAS doesn't know
    // about this patient yet, exactly like A01 / A28 do. The merge
    // makes more sense when both rows exist, but accepting a fresh
    // survivor lets PAS bootstrap from an EMR-driven merge.
    let (survivor, _was_dedup) = match dedup_or_create_patient_from_pid(&state, pid).await {
        Ok(v) => v,
        Err((status, code, diag)) => {
            return (
                status,
                [CT],
                ack("PAS", &sender, &control_id, code, Some(&diag)),
            );
        }
    };
    // Source: must already exist (look up by MRN — exact match, same
    // semantics as A01 / A28 dedup but reversed in intent).
    let source = match PatientRepository::find_by_identifier_value(&state.db, &source_mrn).await {
        Ok(Some(p))
            if p.identifiers
                .iter()
                .any(|i| i.identifier_type == IdentifierType::MRN && i.value == source_mrn) =>
        {
            p
        }
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("no PAS patient with MRN {source_mrn:?}")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("source MRN lookup: {e}")),
                ),
            );
        }
    };
    if source.id == survivor.id {
        return (
            StatusCode::CONFLICT,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!(
                    "MRG-1.1 resolves to the same patient as PID ({}) — cannot self-merge",
                    survivor.id
                )),
            ),
        );
    }
    if let Some(prior) = source.replaced_by {
        return (
            StatusCode::CONFLICT,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!(
                    "source patient {} is already merged into {prior}",
                    source.id
                )),
            ),
        );
    }

    let source_id = source.id;
    let target_id = survivor.id;
    let ctx_clone = ctx.clone();
    let txn_res = state
        .db
        .transaction::<_, (), Error>(|txn| {
            Box::pin(async move {
                PatientRepository::set_replaced_by(txn, source_id, target_id).await?;
                AuditLogRepository::log(
                    txn,
                    "patient",
                    source_id,
                    "merge_via_hl7v2_a40",
                    None,
                    Some(serde_json::json!({
                        "source_id": source_id,
                        "target_id": target_id,
                    })),
                    &ctx_clone,
                )
                .await?;
                OutboxRepository::publish(
                    txn,
                    "PatientMerged",
                    &serde_json::json!({
                        "source_id": source_id,
                        "target_id": target_id,
                        "source": "hl7v2_a40",
                    }),
                )
                .await?;
                Ok(())
            })
        })
        .await;
    match txn_res {
        Ok(()) => {}
        Err(sea_orm::TransactionError::Connection(c)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("db: {c}")),
                ),
            );
        }
        Err(sea_orm::TransactionError::Transaction(t)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("merge txn: {t}")),
                ),
            );
        }
    }
    // Best-effort drop from Tantivy — the source row stays in the DB
    // for audit, but search should hide it.
    if let Some(search) = &state.search {
        let _ = search.delete_patient(source_id);
    }
    let diag = format!("merged source={source_id} into target={target_id}");
    (
        StatusCode::OK,
        [CT],
        ack("PAS", &sender, &control_id, AckCode::Accept, Some(&diag)),
    )
}

/// `POST /api/hl7/v2/dft` — ingest a `DFT^P03` (detail financial
/// transaction — post detail) message. Parses the patient off `PID`
/// (dedup-on-MRN like A01 / A28; creates the row when unknown),
/// resolves an open billing account for the patient (creating one in
/// the FT1-11.2 currency when none exists), and posts a charge built
/// from FT1-7 (code), FT1-8 (description), FT1-11.1 (amount), and
/// FT1-11.2 (currency).
///
/// v0.19 accepts only `FT1-6 = CG` (charge); `PY` (payment) and `AJ`
/// (adjustment) AE-ACK as unsupported transaction types.
///
/// Writes the standard audit (`action = "post_via_hl7v2_p03"`) and
/// outbox event (`event_type = "ChargePosted"`, payload includes
/// `source: "hl7v2_p03"` so the outbound publisher can skip the
/// boomerang). The AA ACK reports `charge=<uuid> account=<uuid>` in
/// MSA-3 so the sender can stamp the charge in its own ledger.
///
/// Returns `400 + AE` on missing or malformed FT1 fields, `400 + AE`
/// on unsupported transaction type.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/dft",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — charge posted; MSA-3 carries `charge=<uuid> account=<uuid>`.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — DFT body rejected (missing FT1 fields, bad amount/currency, unsupported transaction type).", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_dft(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    use crate::db::repositories::billing::BillingRepository;
    use crate::db::repositories::outbox::OutboxRepository;
    use crate::models::billing::{Account, Charge};
    use crate::models::{Iso4217, Money};
    use sea_orm::TransactionTrait;
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(&msg);
    if code != "DFT" || event != "P03" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected DFT^P03, got {code}^{event}")),
            ),
        );
    }
    let dft = match parse_dft_p03(&msg) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    let pid = msg.segment("PID").expect("parse_dft_p03 would have failed");
    let (patient, _was_dedup) = match dedup_or_create_patient_from_pid(&state, pid).await {
        Ok(v) => v,
        Err((status, code, diag)) => {
            return (
                status,
                [CT],
                ack("PAS", &sender, &control_id, code, Some(&diag)),
            );
        }
    };

    // All FT1 segments must share the same currency. Mixing currencies
    // in a single DFT^P03 message would force PAS to split across
    // multiple billing accounts; that's an unusual sender shape and we
    // reject it rather than guess intent.
    let first_currency = &dft.items[0].currency;
    if let Some(bad) = dft.items.iter().find(|i| i.currency != *first_currency) {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!(
                    "all FT1 currencies must match; got {} and {}",
                    first_currency, bad.currency
                )),
            ),
        );
    }
    let currency = match Iso4217::new(first_currency) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&e.to_string()),
                ),
            );
        }
    };

    // Find or create an open account for the patient. We don't reopen
    // closed accounts here — the EMR is treated as authoritative about
    // wanting to post the charges.
    let account =
        match BillingRepository::find_open_account_for_patient(&state.db, patient.id).await {
            Ok(Some(a)) => a,
            Ok(None) => {
                let new_account = Account::new(patient.id, currency.clone());
                match BillingRepository::create_account(&state.db, &new_account).await {
                    Ok(a) => a,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            [CT],
                            ack(
                                "PAS",
                                &sender,
                                &control_id,
                                AckCode::Reject,
                                Some(&format!("create account: {e}")),
                            ),
                        );
                    }
                }
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [CT],
                    ack(
                        "PAS",
                        &sender,
                        &control_id,
                        AckCode::Reject,
                        Some(&format!("account lookup: {e}")),
                    ),
                );
            }
        };

    // All charges + audits + outbox events go in one DB transaction —
    // a half-applied DFT would leave the EMR's ledger out of sync with
    // PAS's, which is worse than a clean rollback + retry.
    let items = dft.items.clone();
    let currency_for_txn = currency.clone();
    let patient_id = patient.id;
    let account_id = account.id;
    let ctx_clone = ctx.clone();
    let txn_res = state
        .db
        .transaction::<_, Vec<Uuid>, Error>(|txn| {
            Box::pin(async move {
                let mut posted_ids: Vec<Uuid> = Vec::with_capacity(items.len());
                for item in &items {
                    let money = Money::new(item.amount, currency_for_txn.clone());
                    let mut charge = Charge::new(
                        account_id,
                        item.code.clone(),
                        item.description.clone(),
                        money,
                    );
                    if let Some(ts) = item.posted_at {
                        charge.posted_at = ts;
                    }
                    let posted = BillingRepository::create_charge(txn, &charge).await?;
                    AuditLogRepository::log(
                        txn,
                        "charge",
                        posted.id,
                        "post_via_hl7v2_p03",
                        None,
                        Some(serde_json::to_value(&posted).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "ChargePosted",
                        &serde_json::json!({
                            "charge_id": posted.id,
                            "account_id": account_id,
                            "patient_id": patient_id,
                            "source": "hl7v2_p03",
                        }),
                    )
                    .await?;
                    posted_ids.push(posted.id);
                }
                Ok(posted_ids)
            })
        })
        .await;
    let posted_ids = match txn_res {
        Ok(ids) => ids,
        Err(sea_orm::TransactionError::Connection(c)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("db: {c}")),
                ),
            );
        }
        Err(sea_orm::TransactionError::Transaction(t)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("post charges txn: {t}")),
                ),
            );
        }
    };
    // MSA-3 diag: for single-FT1 (the common case + the v0.19 contract)
    // report the assigned charge UUID inline. For multi-FT1, just give
    // the count + account; the sender is expected to read the full
    // assignment from a follow-up GET if needed.
    let diag = if posted_ids.len() == 1 {
        format!("charge={} account={}", posted_ids[0], account.id)
    } else {
        format!("charges_posted={} account={}", posted_ids.len(), account.id)
    };
    (
        StatusCode::OK,
        [CT],
        ack("PAS", &sender, &control_id, AckCode::Accept, Some(&diag)),
    )
}

/// `POST /api/hl7/v2/mfn-staff` — ingest an `MFN^M02` (Master File
/// Notification — Staff) message. The message carries one or more
/// MFE+STF pairs; each pair represents a single practitioner record
/// to add (`MFE-1 = MAD`), update (`MUP`), or soft-delete (`MDL`).
///
/// The EMR's staff id (MFE-4 with STF-1 fallback) is stored on the
/// practitioner row as `Identifier { type: Other, system:
/// "urn:hl7v2:staff:id" }` so subsequent `MUP` / `MDL` messages
/// can locate the same PAS row.
///
/// All MFE+STF pairs from one message are applied in a single DB
/// transaction (atomic per message — same contract as v0.20's
/// multi-FT1 DFT). Returns `400 + AE` on parse / validation
/// failure, `404 + AE` when a `MUP` / `MDL` references an unknown
/// staff id, `409 + AE` when an `MAD` would create a duplicate
/// (the staff id is already in use).
///
/// `MDL` is a **soft delete** via the practitioner's `active`
/// flag — practitioner rows hold downstream encounter / appointment
/// / schedule references and a hard delete would orphan them.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/mfn-staff",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — staff master file update applied.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — MFN body rejected.", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — MUP or MDL referenced an unknown staff id.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — MAD attempted on an already-known staff id.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_mfn_staff(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    use crate::db::repositories::outbox::OutboxRepository;
    use crate::db::repositories::practitioner::PractitionerRepository;
    use crate::models::identifier::{Identifier, IdentifierType, IdentifierUse};
    use crate::models::practitioner::Practitioner;
    use sea_orm::TransactionTrait;
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(&msg);
    if code != "MFN" || event != "M02" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected MFN^M02, got {code}^{event}")),
            ),
        );
    }
    let mfn = match parse_mfn_m02(&msg) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&e.to_string()),
                ),
            );
        }
    };

    // Pre-check existence per item so we can return a precise 404
    // (MUP/MDL on unknown id) or 409 (MAD on duplicate id) before
    // opening the transaction. Inside the transaction we redo the
    // lookup so we don't lose atomicity, but the pre-check gives us
    // a clean HTTP status without partial DB work.
    let items = mfn.items.clone();
    for (idx, item) in items.iter().enumerate() {
        let existing =
            match PractitionerRepository::find_by_identifier_value(&state.db, &item.primary_key)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        [CT],
                        ack(
                            "PAS",
                            &sender,
                            &control_id,
                            AckCode::Reject,
                            Some(&format!("MFE[{}] lookup: {e}", idx + 1)),
                        ),
                    );
                }
            };
        match item.event_code.as_str() {
            "MAD" if existing.is_some() => {
                return (
                    StatusCode::CONFLICT,
                    [CT],
                    ack(
                        "PAS",
                        &sender,
                        &control_id,
                        AckCode::AppError,
                        Some(&format!(
                            "MFE[{}] MAD: staff id {:?} already known",
                            idx + 1,
                            item.primary_key
                        )),
                    ),
                );
            }
            "MUP" | "MDL" if existing.is_none() => {
                return (
                    StatusCode::NOT_FOUND,
                    [CT],
                    ack(
                        "PAS",
                        &sender,
                        &control_id,
                        AckCode::AppError,
                        Some(&format!(
                            "MFE[{}] {}: no practitioner with staff id {:?}",
                            idx + 1,
                            item.event_code,
                            item.primary_key
                        )),
                    ),
                );
            }
            _ => {}
        }
    }

    // Apply all items in one transaction so a half-applied master
    // file update can't leave the practitioner directory partially
    // in sync.
    let ctx_clone = ctx.clone();
    let items_for_txn = items.clone();
    let txn_res = state
        .db
        .transaction::<_, Vec<(String, Uuid)>, Error>(|txn| {
            Box::pin(async move {
                let mut results: Vec<(String, Uuid)> = Vec::with_capacity(items_for_txn.len());
                for item in &items_for_txn {
                    let existing =
                        PractitionerRepository::find_by_identifier_value(txn, &item.primary_key)
                            .await?;
                    let (event, id) = match item.event_code.as_str() {
                        "MAD" => {
                            let id = Uuid::new_v4();
                            let now = chrono::Utc::now();
                            let identifier = Identifier {
                                use_type: Some(IdentifierUse::Official),
                                identifier_type: IdentifierType::Other,
                                system: "urn:hl7v2:staff:id".to_string(),
                                value: item.primary_key.clone(),
                                assigner: None,
                            };
                            let p = Practitioner {
                                id,
                                identifiers: vec![identifier],
                                active: item.active,
                                name: item.name.clone(),
                                telecom: Vec::new(),
                                addresses: Vec::new(),
                                gender: item.gender,
                                birth_date: item.birth_date,
                                created_at: now,
                                updated_at: now,
                            };
                            PractitionerRepository::create(txn, &p).await?;
                            ("PractitionerCreated", id)
                        }
                        "MUP" => {
                            let mut p = existing.expect("pre-check guarantees existence");
                            p.name = item.name.clone();
                            p.gender = item.gender;
                            p.birth_date = item.birth_date;
                            p.active = item.active;
                            p.updated_at = chrono::Utc::now();
                            PractitionerRepository::update(txn, &p).await?;
                            ("PractitionerUpdated", p.id)
                        }
                        "MDL" => {
                            let p = existing.expect("pre-check guarantees existence");
                            PractitionerRepository::set_active(txn, p.id, false).await?;
                            ("PractitionerDeactivated", p.id)
                        }
                        other => {
                            return Err(Error::validation(format!(
                                "unexpected MFE event code: {other}"
                            )));
                        }
                    };
                    AuditLogRepository::log(
                        txn,
                        "practitioner",
                        id,
                        &format!("{}_via_hl7v2_mfn_m02", item.event_code.to_lowercase()),
                        None,
                        Some(serde_json::json!({
                            "primary_key": item.primary_key,
                            "event_code": item.event_code,
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        event,
                        &serde_json::json!({
                            "practitioner_id": id,
                            "primary_key": item.primary_key,
                            "source": "hl7v2_mfn_m02",
                        }),
                    )
                    .await?;
                    results.push((item.event_code.clone(), id));
                }
                Ok(results)
            })
        })
        .await;
    let results = match txn_res {
        Ok(r) => r,
        Err(sea_orm::TransactionError::Connection(c)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("db: {c}")),
                ),
            );
        }
        Err(sea_orm::TransactionError::Transaction(t)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("mfn txn: {t}")),
                ),
            );
        }
    };
    let diag = if results.len() == 1 {
        format!("practitioner={} event={}", results[0].1, results[0].0)
    } else {
        format!("staff_records_applied={}", results.len())
    };
    (
        StatusCode::OK,
        [CT],
        ack("PAS", &sender, &control_id, AckCode::Accept, Some(&diag)),
    )
}

/// `POST /api/hl7/v2/mfn-location` — ingest an `MFN^M05` (Master File
/// Notification — Patient Location) message. PAS treats this as the
/// bed roster wire: each MFE+LOC pair adds (`MAD`), updates (`MUP`),
/// or soft-deletes (`MDL`) a `Bed` row, atomic per message.
///
/// LOC-1 is `PL`-typed: `LOC-1.1` is the parent **room code** (the
/// `code` field on the PAS `Room` row), `LOC-1.3` is the **bed code**
/// (the primary lookup key). LOC-2 is the free-form name.
///
/// `MDL` is soft-delete via `BedStatus = OutOfService`, applied via
/// `BedRepository::set_status_unchecked` because the bed state
/// machine doesn't model "any status → OutOfService" for occupied
/// beds — same operator-bypass pattern as the v0.4 ADT^A13 path.
///
/// Returns `400 + AE` on parse / validation failure, `404 + AE` when
/// `MUP` / `MDL` references an unknown bed code (or `MAD` references
/// an unknown room code), `409 + AE` when an `MAD` would create a
/// bed whose code is already taken.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/mfn-location",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — location master file update applied.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — MFN body rejected.", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — MUP / MDL on unknown bed code, or MAD on unknown room code.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — MAD on an already-known bed code.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_mfn_location(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    use crate::db::entities::room;
    use crate::db::repositories::bed::BedRepository;
    use crate::db::repositories::outbox::OutboxRepository;
    use crate::models::facility::{Bed, BedStatus};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(&msg);
    if code != "MFN" || event != "M05" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected MFN^M05, got {code}^{event}")),
            ),
        );
    }
    let mfn = match parse_mfn_m05(&msg) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&e.to_string()),
                ),
            );
        }
    };

    // Pre-check pass: validate each item's existence requirements
    // outside the transaction so we can return precise 404 / 409 HTTP
    // codes without partial DB work.
    for (idx, item) in mfn.items.iter().enumerate() {
        let existing_bed = match BedRepository::find_by_code(&state.db, &item.bed_code).await {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [CT],
                    ack(
                        "PAS",
                        &sender,
                        &control_id,
                        AckCode::Reject,
                        Some(&format!("MFE[{}] bed lookup: {e}", idx + 1)),
                    ),
                );
            }
        };
        match item.event_code.as_str() {
            "MAD" => {
                if existing_bed.is_some() {
                    return (
                        StatusCode::CONFLICT,
                        [CT],
                        ack(
                            "PAS",
                            &sender,
                            &control_id,
                            AckCode::AppError,
                            Some(&format!(
                                "MFE[{}] MAD: bed code {:?} already known",
                                idx + 1,
                                item.bed_code
                            )),
                        ),
                    );
                }
                let room_code = item
                    .room_code
                    .as_deref()
                    .expect("parser guarantees MAD room");
                let room_found = room::Entity::find()
                    .filter(room::Column::Code.eq(room_code))
                    .one(&state.db)
                    .await;
                let room_exists = match room_found {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            [CT],
                            ack(
                                "PAS",
                                &sender,
                                &control_id,
                                AckCode::Reject,
                                Some(&format!("MFE[{}] room lookup: {e}", idx + 1)),
                            ),
                        );
                    }
                };
                if !room_exists {
                    return (
                        StatusCode::NOT_FOUND,
                        [CT],
                        ack(
                            "PAS",
                            &sender,
                            &control_id,
                            AckCode::AppError,
                            Some(&format!(
                                "MFE[{}] MAD: no room with code {room_code:?}",
                                idx + 1
                            )),
                        ),
                    );
                }
            }
            "MUP" | "MDL" if existing_bed.is_none() => {
                return (
                    StatusCode::NOT_FOUND,
                    [CT],
                    ack(
                        "PAS",
                        &sender,
                        &control_id,
                        AckCode::AppError,
                        Some(&format!(
                            "MFE[{}] {}: no bed with code {:?}",
                            idx + 1,
                            item.event_code,
                            item.bed_code
                        )),
                    ),
                );
            }
            _ => {}
        }
    }

    // Apply all items in one transaction so a half-applied master
    // file update can't leave the bed roster partially in sync.
    let ctx_clone = ctx.clone();
    let items_for_txn = mfn.items.clone();
    let txn_res = state
        .db
        .transaction::<_, Vec<(String, Uuid)>, Error>(|txn| {
            Box::pin(async move {
                let mut results: Vec<(String, Uuid)> = Vec::with_capacity(items_for_txn.len());
                for item in &items_for_txn {
                    let (event, id) = match item.event_code.as_str() {
                        "MAD" => {
                            let room_code = item.room_code.as_deref().unwrap();
                            let room_id = room::Entity::find()
                                .filter(room::Column::Code.eq(room_code))
                                .one(txn)
                                .await
                                .map_err(Error::Database)?
                                .ok_or_else(|| Error::not_found(format!("room {room_code}")))?
                                .id;
                            let bed = Bed::new(
                                room_id,
                                item.name.clone().unwrap(),
                                item.bed_code.clone(),
                            );
                            let saved = BedRepository::create(txn, &bed).await?;
                            ("BedCreated", saved.id)
                        }
                        "MUP" => {
                            let mut bed = BedRepository::find_by_code(txn, &item.bed_code)
                                .await?
                                .expect("pre-check guarantees existence");
                            if let Some(name) = &item.name {
                                bed.name = name.clone();
                            }
                            if let Some(room_code) = &item.room_code {
                                let room_id = room::Entity::find()
                                    .filter(room::Column::Code.eq(room_code))
                                    .one(txn)
                                    .await
                                    .map_err(Error::Database)?
                                    .ok_or_else(|| {
                                        Error::not_found(format!("room {room_code} for MUP"))
                                    })?
                                    .id;
                                bed.room_id = room_id;
                            }
                            BedRepository::update(txn, &bed).await?;
                            ("BedUpdated", bed.id)
                        }
                        "MDL" => {
                            let bed = BedRepository::find_by_code(txn, &item.bed_code)
                                .await?
                                .expect("pre-check guarantees existence");
                            // Operator-authorised bypass: see comments
                            // in the v0.4 ADT^A13 path. The bed state
                            // machine doesn't model "any status →
                            // OutOfService" for occupied beds, but
                            // MFN-driven retirement comes from a
                            // trusted master-file source.
                            BedRepository::set_status_unchecked(
                                txn,
                                bed.id,
                                BedStatus::OutOfService,
                            )
                            .await?;
                            ("BedRetired", bed.id)
                        }
                        other => {
                            return Err(Error::validation(format!(
                                "unexpected MFE event code: {other}"
                            )));
                        }
                    };
                    AuditLogRepository::log(
                        txn,
                        "bed",
                        id,
                        &format!("{}_via_hl7v2_mfn_m05", item.event_code.to_lowercase()),
                        None,
                        Some(serde_json::json!({
                            "bed_code": item.bed_code,
                            "event_code": item.event_code,
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        event,
                        &serde_json::json!({
                            "bed_id": id,
                            "bed_code": item.bed_code,
                            "source": "hl7v2_mfn_m05",
                        }),
                    )
                    .await?;
                    results.push((item.event_code.clone(), id));
                }
                Ok(results)
            })
        })
        .await;
    let results = match txn_res {
        Ok(r) => r,
        Err(sea_orm::TransactionError::Connection(c)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("db: {c}")),
                ),
            );
        }
        Err(sea_orm::TransactionError::Transaction(t)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("mfn txn: {t}")),
                ),
            );
        }
    };
    let diag = if results.len() == 1 {
        format!("bed={} event={}", results[0].1, results[0].0)
    } else {
        format!("location_records_applied={}", results.len())
    };
    (
        StatusCode::OK,
        [CT],
        ack("PAS", &sender, &control_id, AckCode::Accept, Some(&diag)),
    )
}

/// `POST /api/hl7/v2/schedule-reschedule` — ingest an `SIU^S13`
/// (notification of appointment rescheduling) message. Identifies the
/// appointment by the filler id in `SCH-2` (PAS UUID), updates
/// `start_datetime` + `end_datetime` from the new `SCH-11` + `SCH-9`,
/// and runs the same overlap-protection check as S12 but excluding the
/// row being rescheduled (so the row's own current time window doesn't
/// flag itself).
///
/// Returns `400 + AE` on bad shape, `404 + AE` if no PAS appointment
/// matches, `409 + AE` if the appointment is in a terminal status or
/// if the *new* window collides with another live appointment.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-reschedule",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — appointment rescheduled.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — SIU body rejected.", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — no PAS appointment with that filler id.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — appointment is terminal, or new window overlaps another appointment.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_schedule_reschedule(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(&msg);
    if code != "SIU" || event != "S13" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected SIU^S13, got {code}^{event}")),
            ),
        );
    }
    let siu = match parse_siu(&msg) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    let filler = siu.filler_appointment_id.as_deref().unwrap();
    let appt_id = match Uuid::parse_str(filler) {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("SCH-2 must be a PAS appointment UUID: {e}")),
                ),
            );
        }
    };
    let new_start = siu.start_datetime.expect("S13 always carries start");
    let new_end = siu.end_datetime.expect("S13 always carries end");

    let mut existing = match AppointmentRepository::find_by_id(&state.db, appt_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("no PAS appointment with filler id {appt_id}")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("lookup: {e}")),
                ),
            );
        }
    };
    use crate::models::appointment::AppointmentStatus;
    if matches!(
        existing.status,
        AppointmentStatus::Cancelled | AppointmentStatus::Fulfilled | AppointmentStatus::NoShow
    ) {
        return (
            StatusCode::CONFLICT,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!(
                    "appointment {appt_id} already in terminal status {:?}",
                    existing.status
                )),
            ),
        );
    }
    // Overlap check against any OTHER live appointment for this patient.
    match AppointmentRepository::find_overlapping_for_patient_excluding(
        &state.db,
        existing.patient_id,
        new_start,
        new_end,
        appt_id,
    )
    .await
    {
        Ok(rows) if !rows.is_empty() => {
            return (
                StatusCode::CONFLICT,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!(
                        "new window overlaps another appointment for patient {}",
                        existing.patient_id
                    )),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("overlap check: {e}")),
                ),
            );
        }
        _ => {}
    }

    existing.start_datetime = new_start;
    existing.end_datetime = new_end;
    existing.updated_at = chrono::Utc::now();
    let updated = match AppointmentRepository::update(&state.db, &existing).await {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("update: {e}")),
                ),
            );
        }
    };
    let _ = AuditLogRepository::log(
        &state.db,
        "appointment",
        updated.id,
        "reschedule_via_hl7v2_s13",
        None,
        Some(serde_json::to_value(&updated).unwrap_or_default()),
        &ctx,
    )
    .await;
    let _ = crate::db::repositories::outbox::OutboxRepository::publish(
        &state.db,
        "AppointmentRescheduled",
        &serde_json::json!({
            "appointment_id": updated.id,
            "patient_id": updated.patient_id,
            "start": updated.start_datetime,
            "end": updated.end_datetime,
            "source": "hl7v2_s13",
        }),
    )
    .await;
    (
        StatusCode::OK,
        [CT],
        ack(
            "PAS",
            &sender,
            &control_id,
            AckCode::Accept,
            Some(&format!("appointment {} rescheduled", updated.id)),
        ),
    )
}

/// `POST /api/hl7/v2/schedule-modify` — ingest an `SIU^S14`
/// (notification of appointment modification) message. Identifies the
/// appointment by the filler id in `SCH-2` and updates `reason` from
/// `SCH-7`. Time fields are intentionally ignored — use SIU^S13 for
/// time changes.
///
/// Returns `400 + AE` on bad shape, `404 + AE` if no PAS appointment
/// matches, `409 + AE` if the appointment is in a terminal status.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-modify",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "AA ACK — appointment modified.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AE ACK — SIU body rejected.", body = String, content_type = "application/hl7-v2"),
        (status = 404, description = "AE ACK — no PAS appointment with that filler id.", body = String, content_type = "application/hl7-v2"),
        (status = 409, description = "AE ACK — appointment is in a terminal status.", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_schedule_modify(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );
    let ctx = user_context_from_headers(&headers);

    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 parse: {e}")),
                ),
            );
        }
    };
    let control_id = msg
        .segment("MSH")
        .map(|s| s.field(10).to_string())
        .unwrap_or_default();
    let sender = msg
        .segment("MSH")
        .map(|s| s.field(3).to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    let (code, event) = message_type(&msg);
    if code != "SIU" || event != "S14" {
        return (
            StatusCode::BAD_REQUEST,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!("expected SIU^S14, got {code}^{event}")),
            ),
        );
    }
    let siu = match parse_siu(&msg) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&e.to_string()),
                ),
            );
        }
    };
    let filler = siu.filler_appointment_id.as_deref().unwrap();
    let appt_id = match Uuid::parse_str(filler) {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("SCH-2 must be a PAS appointment UUID: {e}")),
                ),
            );
        }
    };

    let mut existing = match AppointmentRepository::find_by_id(&state.db, appt_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::AppError,
                    Some(&format!("no PAS appointment with filler id {appt_id}")),
                ),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("lookup: {e}")),
                ),
            );
        }
    };
    use crate::models::appointment::AppointmentStatus;
    if matches!(
        existing.status,
        AppointmentStatus::Cancelled | AppointmentStatus::Fulfilled | AppointmentStatus::NoShow
    ) {
        return (
            StatusCode::CONFLICT,
            [CT],
            ack(
                "PAS",
                &sender,
                &control_id,
                AckCode::AppError,
                Some(&format!(
                    "appointment {appt_id} already in terminal status {:?}",
                    existing.status
                )),
            ),
        );
    }
    existing.reason = siu.reason.clone();
    existing.updated_at = chrono::Utc::now();
    let updated = match AppointmentRepository::update(&state.db, &existing).await {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [CT],
                ack(
                    "PAS",
                    &sender,
                    &control_id,
                    AckCode::Reject,
                    Some(&format!("update: {e}")),
                ),
            );
        }
    };
    let _ = AuditLogRepository::log(
        &state.db,
        "appointment",
        updated.id,
        "modify_via_hl7v2_s14",
        None,
        Some(serde_json::to_value(&updated).unwrap_or_default()),
        &ctx,
    )
    .await;
    let _ = crate::db::repositories::outbox::OutboxRepository::publish(
        &state.db,
        "AppointmentModified",
        &serde_json::json!({
            "appointment_id": updated.id,
            "patient_id": updated.patient_id,
            "reason": updated.reason,
            "source": "hl7v2_s14",
        }),
    )
    .await;
    (
        StatusCode::OK,
        [CT],
        ack(
            "PAS",
            &sender,
            &control_id,
            AckCode::Accept,
            Some(&format!("appointment {} modified", updated.id)),
        ),
    )
}

/// `POST /api/hl7/v2/batch` — ingest one HL7 v2 batch envelope
/// (`FHS`/`BHS`/`BTS`/`FTS`) containing many ADT messages and respond with
/// a batch ACK envelope carrying one `MSA` per inbound message.
///
/// Semantics: each contained message is dispatched independently to the
/// matching single-message handler (`hl7_v2_admit`, `hl7_v2_update`, …).
/// One failure does **not** roll back the others — this mirrors how real
/// HL7 v2 batch processors behave, and matches the PAS FHIR `batch` Bundle
/// path (the FHIR `transaction` Bundle is the all-or-nothing option). Per-
/// message AA/AE/AR appears inside the batch envelope.
///
/// The endpoint always returns HTTP `200 OK` when the batch envelope
/// itself parses, regardless of whether any individual message succeeded
/// — the AA/AE/AR per message is the sender's signal. Returns
/// `400 BAD REQUEST` with a single `AR` ACK when the envelope is
/// malformed (no MSH/BHS/FHS, oversize, two BHS in one transmission,
/// non-standard delimiters).
#[utoipa::path(
    post,
    path = "/api/hl7/v2/batch",
    tag = "HL7v2",
    request_body(content = String, content_type = "application/hl7-v2"),
    responses(
        (status = 200, description = "Batch ACK envelope with per-message MSA blocks.", body = String, content_type = "application/hl7-v2"),
        (status = 400, description = "AR ACK — envelope malformed, oversize, or unsupported (multi-batch files).", body = String, content_type = "application/hl7-v2")
    )
)]
pub async fn hl7_v2_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    const CT: (axum::http::header::HeaderName, &str) = (
        axum::http::header::CONTENT_TYPE,
        "application/hl7-v2; charset=utf-8",
    );

    let batch = match crate::hl7v2::parse_batch(&body) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [CT],
                ack(
                    "PAS",
                    "UNKNOWN",
                    "UNKNOWN",
                    AckCode::Reject,
                    Some(&format!("HL7v2 batch parse: {e}")),
                ),
            );
        }
    };

    // Sender + batch control id come from BHS when present, else from the
    // first message's MSH (covers the bare-message-list shape we accept).
    let (sender, batch_control_id) = batch
        .bhs
        .as_ref()
        .map(|b| (b.field(3).to_string(), b.field(11).to_string()))
        .or_else(|| {
            batch.messages.first().and_then(|m| {
                m.segment("MSH")
                    .map(|s| (s.field(3).to_string(), s.field(10).to_string()))
            })
        })
        .unwrap_or_else(|| ("UNKNOWN".into(), "UNKNOWN".into()));

    let mut acks: Vec<String> = Vec::with_capacity(batch.messages.len());
    for msg in &batch.messages {
        let msg_body = crate::hl7v2::encode_message(msg);
        let ack_body = dispatch_single_v2_message(&state, &headers, msg_body).await;
        acks.push(ack_body);
    }

    let batch_ack =
        crate::hl7v2::encode_batch_ack("PAS", &sender, &format!("ACK-{batch_control_id}"), &acks);
    (StatusCode::OK, [CT], batch_ack)
}

/// Internal helper: dispatch one v2 message body to the matching
/// single-message handler and return just the ACK string. Used by the
/// batch handler.
///
/// Routing mirrors the MLLP listener: looks at `MSH-9` and picks the
/// matching `hl7_v2_*` handler. Messages with no recognized type fall
/// through to `hl7_v2_patient`, which itself returns an `AE` for non-
/// ADT codes — consistent with the single-message HTTP path.
async fn dispatch_single_v2_message(
    state: &AppState,
    headers: &HeaderMap,
    msg_body: String,
) -> String {
    let parsed = match parse_message(&msg_body) {
        Ok(m) => m,
        Err(e) => {
            return ack(
                "PAS",
                "UNKNOWN",
                "UNKNOWN",
                AckCode::Reject,
                Some(&format!("HL7v2 parse: {e}")),
            );
        }
    };
    let (code, event) = message_type(&parsed);
    let (_status, _ct, ack_body) = match (code.as_str(), event.as_str()) {
        ("ADT", "A01") => hl7_v2_admit(State(state.clone()), headers.clone(), msg_body).await,
        ("ADT", "A02") => hl7_v2_transfer(State(state.clone()), headers.clone(), msg_body).await,
        ("ADT", "A03") => hl7_v2_discharge(State(state.clone()), headers.clone(), msg_body).await,
        ("ADT", "A08") => hl7_v2_update(State(state.clone()), headers.clone(), msg_body).await,
        ("ADT", "A11") => {
            hl7_v2_cancel_admit(State(state.clone()), headers.clone(), msg_body).await
        }
        ("ADT", "A13") => {
            hl7_v2_cancel_discharge(State(state.clone()), headers.clone(), msg_body).await
        }
        // ADT^A28 + everything else falls through to the generic patient
        // handler (which itself rejects non-ADT codes with an AE).
        _ => hl7_v2_patient(State(state.clone()), msg_body).await,
    };
    ack_body
}

// ---------------------------------------------------------------------------
// Interchange — bulk JSON/XML/TSV import + export
// ---------------------------------------------------------------------------

use crate::interchange::{self, PatientRow};

const EXPORT_LIMIT: u64 = 10_000;

async fn load_patient_rows(state: &AppState) -> Result<Vec<PatientRow>> {
    let patients = PatientRepository::list_active(&state.db, EXPORT_LIMIT).await?;
    Ok(patients.iter().map(PatientRow::from).collect())
}

/// `GET /api/patients/export.json` — bulk patient export as a JSON array.
#[utoipa::path(
    get,
    path = "/api/patients/export.json",
    tag = "Interchange",
    responses(
        (status = 200, description = "Active patient roster (up to 10k rows) as a `PatientRow[]` payload wrapped in the standard `ApiResponse` envelope.", body = [PatientRow])
    )
)]
pub async fn export_patients_json(State(state): State<AppState>) -> HandlerResult {
    let rows = match load_patient_rows(&state).await {
        Ok(r) => r,
        Err(e) => return error_to_response(e),
    };
    match interchange::json::patients_to_json_compact(&rows) {
        Ok(body) => match serde_json::from_str::<Value>(&body) {
            Ok(v) => ok_response(v),
            Err(e) => error_to_response(Error::internal(format!("JSON: {e}"))),
        },
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/patients/export.xml` — bulk patient export as XML.
#[utoipa::path(
    get,
    path = "/api/patients/export.xml",
    tag = "Interchange",
    responses(
        (
            status = 200,
            description = "Active patient roster as `<patients><patient>…</patient></patients>` XML document.",
            body = String,
            content_type = "application/xml"
        )
    )
)]
pub async fn export_patients_xml(
    State(state): State<AppState>,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    let rows = match load_patient_rows(&state).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(
                    axum::http::header::CONTENT_TYPE,
                    "text/plain; charset=utf-8",
                )],
                e.to_string(),
            );
        }
    };
    match interchange::xml::patients_to_xml(&rows) {
        Ok(body) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/xml; charset=utf-8",
            )],
            body,
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            e.to_string(),
        ),
    }
}

/// `GET /api/patients/export.csv` — bulk patient export as CSV (RFC 4180).
#[utoipa::path(
    get,
    path = "/api/patients/export.csv",
    tag = "Interchange",
    responses(
        (
            status = 200,
            description = "Active patient roster as RFC-4180-style CSV with a fixed header row.",
            body = String,
            content_type = "text/csv"
        )
    )
)]
pub async fn export_patients_csv(
    State(state): State<AppState>,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    let rows = match load_patient_rows(&state).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(
                    axum::http::header::CONTENT_TYPE,
                    "text/plain; charset=utf-8",
                )],
                e.to_string(),
            );
        }
    };
    match interchange::csv::patients_to_csv(&rows) {
        Ok(body) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/csv; charset=utf-8")],
            body,
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            e.to_string(),
        ),
    }
}

/// `GET /api/patients/export.tsv` — bulk patient export as TSV.
#[utoipa::path(
    get,
    path = "/api/patients/export.tsv",
    tag = "Interchange",
    responses(
        (
            status = 200,
            description = "Active patient roster as tab-separated values with a fixed header row.",
            body = String,
            content_type = "text/tab-separated-values"
        )
    )
)]
pub async fn export_patients_tsv(
    State(state): State<AppState>,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    let rows = match load_patient_rows(&state).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(
                    axum::http::header::CONTENT_TYPE,
                    "text/plain; charset=utf-8",
                )],
                e.to_string(),
            );
        }
    };
    match interchange::tsv::patients_to_tsv(&rows) {
        Ok(body) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/tab-separated-values; charset=utf-8",
            )],
            body,
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            e.to_string(),
        ),
    }
}

/// Result of a bulk patient import. Returned as the `data` field of an
/// `ApiResponse`.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct ImportSummary {
    pub inserted: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// `POST /api/patients/import` — bulk import from JSON, XML, or TSV.
///
/// Format is picked from `Content-Type`:
/// - `application/xml`, `text/xml` → XML
/// - `text/tab-separated-values`, `text/tsv` → TSV
/// - anything else → JSON
///
/// Rows whose `id` already exists in the database are *skipped* (not
/// overwritten). Returns a summary `{ inserted, skipped, failed }`.
#[utoipa::path(
    post,
    path = "/api/patients/import",
    tag = "Interchange",
    request_body(
        content = String,
        description = "Patient rows in JSON / XML / TSV / CSV. Format picked from `Content-Type`.",
        content_type = "application/json"
    ),
    responses(
        (status = 200, description = "Counts of inserted/skipped/failed rows.", body = ImportSummary),
        (status = 400, description = "Body could not be parsed in the requested format.")
    )
)]
pub async fn import_patients(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> HandlerResult {
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_ascii_lowercase();

    let rows: Vec<PatientRow> = if content_type.contains("xml") {
        match interchange::xml::patients_from_xml(&body) {
            Ok(r) => r,
            Err(e) => return error_to_response(e),
        }
    } else if content_type.contains("tab-separated") || content_type.contains("tsv") {
        match interchange::tsv::patients_from_tsv(&body) {
            Ok(r) => r,
            Err(e) => return error_to_response(e),
        }
    } else if content_type.contains("csv") {
        match interchange::csv::patients_from_csv(&body) {
            Ok(r) => r,
            Err(e) => return error_to_response(e),
        }
    } else {
        match interchange::json::patients_from_json(&body) {
            Ok(r) => r,
            Err(e) => return error_to_response(e),
        }
    };

    let mut summary = ImportSummary {
        inserted: 0,
        skipped: 0,
        failed: 0,
    };
    for row in &rows {
        let patient = row.to_patient();
        match PatientRepository::find_by_id(&state.db, patient.id).await {
            Ok(Some(_)) => {
                summary.skipped += 1;
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                summary.failed += 1;
                continue;
            }
        }
        match PatientRepository::create(&state.db, &patient).await {
            Ok(p) => {
                if let Some(search) = &state.search {
                    let _ = search.index_patient(&p);
                }
                summary.inserted += 1;
            }
            Err(_) => summary.failed += 1,
        }
    }
    match serde_json::to_value(&summary) {
        Ok(v) => ok_response(v),
        Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
    }
}

// ---------------------------------------------------------------------------
// Appointment series — v0.9.0 recurring appointments
// ---------------------------------------------------------------------------

use crate::scheduling::CreateSeriesInput;

/// Wire shape for POST /api/appointment-series and
/// /api/appointment-series/preview. Mirrors `CreateSeriesInput` but the
/// JSON form is what the API exposes.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateSeriesRequest {
    pub patient_id: Uuid,
    #[serde(default)]
    pub practitioner_id: Option<Uuid>,
    pub service_type: String,
    pub start_datetime: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: u32,
    /// Whole recurrence rule as JSON. See `models::appointment_series`
    /// for the supported field shape (FREQ daily/weekly/monthly,
    /// INTERVAL >= 1, BYDAY for Weekly, COUNT|UNTIL termination).
    #[schema(value_type = serde_json::Value)]
    pub rule: crate::models::appointment_series::RecurrenceRule,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CancelSeriesRequest {
    /// Optional cancellation reason recorded against each cancelled
    /// occurrence. Defaults to `provider_request` when omitted, which
    /// matches the most common "operator pulled the plug" case.
    #[serde(default)]
    #[schema(value_type = String)]
    pub reason: Option<crate::models::appointment::CancellationReason>,
}

impl From<CreateSeriesRequest> for CreateSeriesInput {
    fn from(r: CreateSeriesRequest) -> Self {
        Self {
            patient_id: r.patient_id,
            practitioner_id: r.practitioner_id,
            service_type: r.service_type,
            start_datetime: r.start_datetime,
            duration_minutes: r.duration_minutes,
            rule: r.rule,
            reason: r.reason,
        }
    }
}

/// `POST /api/appointment-series/preview` — dry-run a recurrence rule and
/// return the list of concrete datetimes it would expand to, *without*
/// writing anything. Use this from the UI to let the user sanity-check
/// the schedule before committing. Validates the rule and rejects
/// oversize series the same way `create` does.
#[utoipa::path(post, path = "/api/appointment-series/preview", tag = "Scheduling",
    request_body = CreateSeriesRequest,
    responses(
        (status = 200, description = "Computed occurrence datetimes."),
        (status = 400, description = "Rule failed validation (interval=0, oversize count, until-before-start, by_weekday on non-Weekly, …).")
    ))]
pub async fn preview_appointment_series(
    State(state): State<AppState>,
    Json(req): Json<CreateSeriesRequest>,
) -> HandlerResult {
    let input: CreateSeriesInput = req.into();
    match state.series.preview(&input) {
        Ok(p) => match serde_json::to_value(&p) {
            Ok(v) => ok_response(v),
            Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
        },
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointment-series` — create a recurring series + every
/// occurrence in one DB transaction. Any per-patient overlap on any
/// occurrence rolls back the whole thing (so a 26-week series is either
/// fully booked or fully refused — never partially).
#[utoipa::path(post, path = "/api/appointment-series", tag = "Scheduling",
    request_body = CreateSeriesRequest,
    responses(
        (status = 200, description = "Series + every concrete Appointment row that landed."),
        (status = 400, description = "Rule failed validation."),
        (status = 409, description = "One or more occurrences overlap an existing appointment for this patient.")
    ))]
pub async fn create_appointment_series(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateSeriesRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    let input: CreateSeriesInput = req.into();
    match state.series.create(input, &ctx).await {
        Ok(r) => match serde_json::to_value(&r) {
            Ok(v) => ok_response(v),
            Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
        },
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/appointment-series/{id}` — fetch one series with every
/// linked appointment (any status, ordered by start).
#[utoipa::path(get, path = "/api/appointment-series/{id}", tag = "Scheduling",
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "Series record + every appointment with this series_id."),
        (status = 404, description = "Series id not found.")
    ))]
pub async fn get_appointment_series(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state.series.get_with_occurrences(id).await {
        Ok(Some(r)) => match serde_json::to_value(&r) {
            Ok(v) => ok_response(v),
            Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
        },
        Ok(None) => error_to_response(Error::not_found(format!("appointment_series {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/appointment-series/{id}/cancel` — cancel the series and
/// every future / not-yet-fulfilled occurrence (`Proposed` or `Booked`).
/// Past occurrences (`Arrived`, `Fulfilled`, `NoShow`, already
/// `Cancelled`) are left exactly as the audit trail recorded them.
#[utoipa::path(post, path = "/api/appointment-series/{id}/cancel", tag = "Scheduling",
    params(("id" = Uuid, Path)),
    request_body = CancelSeriesRequest,
    responses(
        (status = 200, description = "Series cancelled; response carries the updated occurrences."),
        (status = 404, description = "Series id not found.")
    ))]
pub async fn cancel_appointment_series(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<CancelSeriesRequest>,
) -> HandlerResult {
    let ctx = user_context_from_headers(&headers);
    let reason = req
        .reason
        .unwrap_or(crate::models::appointment::CancellationReason::ProviderRequest);
    match state.series.cancel(id, reason, &ctx).await {
        Ok(r) => match serde_json::to_value(&r) {
            Ok(v) => ok_response(v),
            Err(e) => error_to_response(Error::internal(format!("serialize: {e}"))),
        },
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/patients/{id}/appointment-series` — every series owned by
/// a patient, newest-first. Series rows only — call
/// `GET /api/appointment-series/{id}` to expand one.
#[utoipa::path(get, path = "/api/patients/{id}/appointment-series", tag = "Scheduling",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Series rows for this patient.")))]
pub async fn list_patient_appointment_series(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match state.series.list_by_patient(id).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

// ---------------------------------------------------------------------------
// Coverage — insurance / payer record (v0.10.0)
// ---------------------------------------------------------------------------

use crate::db::repositories::coverage::CoverageRepository;
use crate::models::coverage::{Coverage, CoverageKind, CoverageStatus};

/// Request shape for `POST /api/coverages`. The handler defaults
/// missing optional fields to the sensible production values
/// documented on each field.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateCoverageRequest {
    pub patient_id: Uuid,
    pub payor_name: String,
    pub policy_number: String,
    #[serde(default)]
    pub account_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = String)]
    pub kind: Option<CoverageKind>,
    #[serde(default)]
    #[schema(value_type = String)]
    pub status: Option<CoverageStatus>,
    #[serde(default)]
    pub subscriber_id: Option<Uuid>,
    #[serde(default)]
    pub payor_identifier: Option<String>,
    #[serde(default)]
    pub group_number: Option<String>,
    /// Defaults to `"self"`.
    #[serde(default)]
    pub relationship: Option<String>,
    /// Defaults to today (UTC) when omitted.
    #[serde(default)]
    pub start_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub end_date: Option<chrono::NaiveDate>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateCoverageRequest {
    #[serde(default)]
    pub account_id: Option<Option<Uuid>>,
    #[serde(default)]
    #[schema(value_type = String)]
    pub status: Option<CoverageStatus>,
    #[serde(default)]
    #[schema(value_type = String)]
    pub kind: Option<CoverageKind>,
    #[serde(default)]
    pub subscriber_id: Option<Option<Uuid>>,
    #[serde(default)]
    pub payor_name: Option<String>,
    #[serde(default)]
    pub payor_identifier: Option<Option<String>>,
    #[serde(default)]
    pub policy_number: Option<String>,
    #[serde(default)]
    pub group_number: Option<Option<String>>,
    #[serde(default)]
    pub relationship: Option<String>,
    #[serde(default)]
    pub start_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub end_date: Option<Option<chrono::NaiveDate>>,
}

/// `POST /api/coverages` — record a new insurance / self-pay / other
/// coverage row for a patient. Optionally pre-link to a billing account
/// via `account_id` (or leave unattached and link later via `PUT`).
#[utoipa::path(post, path = "/api/coverages", tag = "Billing",
    request_body = CreateCoverageRequest,
    responses(
        (status = 200, description = "Coverage created."),
        (status = 400, description = "Validation failure (missing required fields).")
    ))]
pub async fn create_coverage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateCoverageRequest>,
) -> HandlerResult {
    use crate::db::repositories::outbox::OutboxRepository;
    let ctx = user_context_from_headers(&headers);
    let mut c = Coverage::new(req.patient_id, req.payor_name, req.policy_number);
    c.account_id = req.account_id;
    if let Some(k) = req.kind {
        c.kind = k;
    }
    if let Some(s) = req.status {
        c.status = s;
    }
    c.subscriber_id = req.subscriber_id;
    c.payor_identifier = req.payor_identifier;
    c.group_number = req.group_number;
    if let Some(r) = req.relationship {
        c.relationship = r;
    }
    if let Some(d) = req.start_date {
        c.start_date = d;
    }
    c.end_date = req.end_date;

    let created = match CoverageRepository::create(&state.db, &c).await {
        Ok(v) => v,
        Err(e) => return error_to_response(e),
    };
    let _ = AuditLogRepository::log(
        &state.db,
        "coverage",
        created.id,
        "create",
        None,
        Some(serde_json::to_value(&created).unwrap_or_default()),
        &ctx,
    )
    .await;
    let _ = OutboxRepository::publish(
        &state.db,
        "CoverageCreated",
        &serde_json::json!({
            "coverage_id": created.id,
            "patient_id": created.patient_id,
            "account_id": created.account_id,
        }),
    )
    .await;
    finish(Ok(created))
}

#[utoipa::path(get, path = "/api/coverages/{id}", tag = "Billing",
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "Coverage record."),
        (status = 404, description = "Not found.")
    ))]
pub async fn get_coverage(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match CoverageRepository::find_by_id(&state.db, id).await {
        Ok(Some(c)) => finish(Ok(c)),
        Ok(None) => error_to_response(Error::not_found(format!("coverage {id}"))),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(put, path = "/api/coverages/{id}", tag = "Billing",
    params(("id" = Uuid, Path)),
    request_body = UpdateCoverageRequest,
    responses(
        (status = 200, description = "Selective field update applied."),
        (status = 404, description = "Not found.")
    ))]
pub async fn update_coverage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<UpdateCoverageRequest>,
) -> HandlerResult {
    use crate::db::repositories::outbox::OutboxRepository;
    let ctx = user_context_from_headers(&headers);
    let mut c = match CoverageRepository::find_by_id(&state.db, id).await {
        Ok(Some(v)) => v,
        Ok(None) => return error_to_response(Error::not_found(format!("coverage {id}"))),
        Err(e) => return error_to_response(e),
    };
    if let Some(account_id) = req.account_id {
        c.account_id = account_id;
    }
    if let Some(s) = req.status {
        c.status = s;
    }
    if let Some(k) = req.kind {
        c.kind = k;
    }
    if let Some(subscriber_id) = req.subscriber_id {
        c.subscriber_id = subscriber_id;
    }
    if let Some(name) = req.payor_name {
        c.payor_name = name;
    }
    if let Some(payor_id) = req.payor_identifier {
        c.payor_identifier = payor_id;
    }
    if let Some(policy) = req.policy_number {
        c.policy_number = policy;
    }
    if let Some(group) = req.group_number {
        c.group_number = group;
    }
    if let Some(r) = req.relationship {
        c.relationship = r;
    }
    if let Some(d) = req.start_date {
        c.start_date = d;
    }
    if let Some(d) = req.end_date {
        c.end_date = d;
    }
    c.updated_at = chrono::Utc::now();
    let updated = match CoverageRepository::update(&state.db, &c).await {
        Ok(v) => v,
        Err(e) => return error_to_response(e),
    };
    let _ = AuditLogRepository::log(
        &state.db,
        "coverage",
        updated.id,
        "update",
        None,
        Some(serde_json::to_value(&updated).unwrap_or_default()),
        &ctx,
    )
    .await;
    let _ = OutboxRepository::publish(
        &state.db,
        "CoverageUpdated",
        &serde_json::json!({
            "coverage_id": updated.id,
            "patient_id": updated.patient_id,
        }),
    )
    .await;
    finish(Ok(updated))
}

/// `DELETE /api/coverages/{id}` — flip status to `Cancelled`. Coverage
/// rows are **never** hard-deleted (invariant §5.3 keeps soft-delete to
/// patients/encounters/appointments; coverage carries its own
/// retirement status). Audit + outbox written.
#[utoipa::path(delete, path = "/api/coverages/{id}", tag = "Billing",
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "Coverage cancelled."),
        (status = 404, description = "Not found.")
    ))]
pub async fn delete_coverage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    use crate::db::repositories::outbox::OutboxRepository;
    let ctx = user_context_from_headers(&headers);
    let updated =
        match CoverageRepository::set_status(&state.db, id, CoverageStatus::Cancelled).await {
            Ok(v) => v,
            Err(e) => return error_to_response(e),
        };
    let _ = AuditLogRepository::log(
        &state.db,
        "coverage",
        id,
        "cancel",
        None,
        Some(serde_json::json!({ "coverage_id": id })),
        &ctx,
    )
    .await;
    let _ = OutboxRepository::publish(
        &state.db,
        "CoverageCancelled",
        &serde_json::json!({
            "coverage_id": id,
            "patient_id": updated.patient_id,
        }),
    )
    .await;
    finish(Ok(updated))
}

#[utoipa::path(get, path = "/api/patients/{id}/coverages", tag = "Billing",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Coverage rows for the named patient, newest-first.")))]
pub async fn list_patient_coverages(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match CoverageRepository::list_by_patient(&state.db, id).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

#[utoipa::path(get, path = "/api/accounts/{id}/coverages", tag = "Billing",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Coverage rows attached to the named account, newest-first.")))]
pub async fn list_account_coverages(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match CoverageRepository::list_by_account(&state.db, id).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}

// ---------------------------------------------------------------------------
// Patient merge / replaced_by tombstone (v0.11.0)
// ---------------------------------------------------------------------------

/// `POST /api/patients/{id}/merge-into/{target_id}` — record that the
/// patient identified by `id` has been merged into the patient
/// identified by `target_id`. The source row becomes a merge
/// **tombstone**: `replaced_by = target_id`, `active = false`. Both
/// rows survive in the database for audit; default lists and the
/// Tantivy search index drop the source from view.
///
/// Validation rules:
/// - `id != target_id` (no self-merge) — 400.
/// - source must exist and have `replaced_by IS NULL` (no chained
///   merges) — 404 / 409.
/// - target must exist — 404.
///
/// One DB transaction: flip the source row + audit + outbox. After
/// commit, drop the source from the Tantivy index best-effort.
/// Outbox event: `PatientMerged { source_id, target_id }`.
#[utoipa::path(post, path = "/api/patients/{id}/merge-into/{target_id}", tag = "Patient",
    params(
        ("id" = Uuid, Path, description = "Source patient — becomes the tombstone."),
        ("target_id" = Uuid, Path, description = "Survivor patient — the target of the link.")
    ),
    responses(
        (status = 200, description = "Source row now carries `replaced_by = target_id`."),
        (status = 400, description = "Source and target are the same patient."),
        (status = 404, description = "Either source or target was not found."),
        (status = 409, description = "Source is already a tombstone (already merged into someone).")
    ))]
pub async fn merge_patient_into(
    State(state): State<AppState>,
    Path((id, target_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> HandlerResult {
    use crate::db::repositories::outbox::OutboxRepository;
    use sea_orm::TransactionTrait;
    if id == target_id {
        return error_to_response(Error::validation("cannot merge a patient into themselves"));
    }
    let ctx = user_context_from_headers(&headers);

    // Read both rows up front so we can give the right 404 / 409
    // before opening the transaction.
    let source = match PatientRepository::find_by_id(&state.db, id).await {
        Ok(Some(p)) => p,
        Ok(None) => return error_to_response(Error::not_found(format!("patient {id}"))),
        Err(e) => return error_to_response(e),
    };
    if let Some(prior) = source.replaced_by {
        return error_to_response(Error::conflict(format!(
            "patient {id} is already merged into {prior}"
        )));
    }
    match PatientRepository::find_by_id(&state.db, target_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return error_to_response(Error::not_found(format!("target patient {target_id}")));
        }
        Err(e) => return error_to_response(e),
    }

    let ctx_clone = ctx.clone();
    let txn_res = state
        .db
        .transaction::<_, crate::models::patient::Patient, Error>(|txn| {
            Box::pin(async move {
                let updated = PatientRepository::set_replaced_by(txn, id, target_id).await?;
                AuditLogRepository::log(
                    txn,
                    "patient",
                    id,
                    "merge_into",
                    None,
                    Some(serde_json::json!({
                        "source_id": id,
                        "target_id": target_id,
                    })),
                    &ctx_clone,
                )
                .await?;
                OutboxRepository::publish(
                    txn,
                    "PatientMerged",
                    &serde_json::json!({
                        "source_id": id,
                        "target_id": target_id,
                    }),
                )
                .await?;
                Ok(updated)
            })
        })
        .await;
    let updated = match txn_res {
        Ok(p) => p,
        Err(sea_orm::TransactionError::Connection(c)) => {
            return error_to_response(Error::Database(c));
        }
        Err(sea_orm::TransactionError::Transaction(t)) => return error_to_response(t),
    };
    // Best-effort drop from Tantivy so the source stops appearing in
    // search results. The DB row itself remains (audit preservation).
    if let Some(search) = &state.search {
        let _ = search.delete_patient(id);
    }
    finish(Ok(updated))
}

/// `GET /api/patients/{id}/replaces` — list every patient row that
/// has been merged **into** the patient identified by `id` (i.e. every
/// tombstone whose `replaced_by` equals this id). Newest-first.
/// Returns an empty array when the patient has never received a merge.
#[utoipa::path(get, path = "/api/patients/{id}/replaces", tag = "Patient",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Patient rows that were merged into this one.")))]
pub async fn list_patient_replaces(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    match PatientRepository::list_replaces_for(&state.db, id).await {
        Ok(v) => finish(Ok(v)),
        Err(e) => error_to_response(e),
    }
}
