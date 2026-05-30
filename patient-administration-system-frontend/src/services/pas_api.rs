//! Thin reqwest client wrapping the PAS Axum REST API.
//!
//! Used for *write* flows where patient-administration-system-frontend can't (or shouldn't) bypass
//! the PAS service layer — e.g. letter generation, which must write an
//! audit row, persist the generated letter, and emit a `LetterGenerated`
//! outbox event.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Base URL of the PAS Axum API. Configurable via the `PAS_API_URL` env
/// var; defaults to the conventional dev port the PAS binary listens on.
fn base_url() -> String {
    std::env::var("PAS_API_URL").unwrap_or_else(|_| "http://localhost:8080".to_string())
}

fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
}

#[derive(Debug, Serialize)]
pub struct GenerateLetterRequest {
    pub template_id: Uuid,
    pub patient_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appointment_id: Option<Uuid>,
    pub channel: String,
    pub extra: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct GeneratedLetter {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub template_id: Uuid,
    pub rendered_subject: String,
    pub rendered_body: String,
    pub channel: String,
    pub status: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PasApiError {
    #[error("PAS API HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("PAS API returned status {status}: {message}")]
    Status { status: u16, message: String },
    #[error("PAS API response missing expected `data` field")]
    MalformedResponse,
}

/// Shared helper: POST `body` to `path` on the PAS binary, unwrap the
/// `ApiResponse` envelope, and return the `data` field as a `Value` (so
/// the caller can deserialize into a typed struct).
async fn post_envelope<B: Serialize>(
    path: &str,
    body: &B,
) -> Result<serde_json::Value, PasApiError> {
    let url = format!("{}{path}", base_url());
    let resp = client()?.post(&url).json(body).send().await?;
    let status = resp.status();
    let payload: serde_json::Value = resp.json().await?;
    if !status.is_success() {
        let message = payload
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("(no message)")
            .to_string();
        return Err(PasApiError::Status {
            status: status.as_u16(),
            message,
        });
    }
    payload
        .get("data")
        .cloned()
        .ok_or(PasApiError::MalformedResponse)
}

#[derive(Debug, Serialize)]
pub struct AdmitRequest {
    pub patient_id: Uuid,
    pub bed_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct AdmissionResult {
    pub admission: AdmissionStub,
    pub encounter: EncounterStub,
}

#[derive(Debug, Deserialize)]
pub struct AdmissionStub {
    pub id: Uuid,
    pub bed_id: Uuid,
    pub encounter_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct EncounterStub {
    pub id: Uuid,
    pub patient_id: Uuid,
}

/// `POST /api/admissions` on the PAS Axum binary.
pub async fn admit(req: &AdmitRequest) -> Result<AdmissionResult, PasApiError> {
    let data = post_envelope("/api/admissions", req).await?;
    serde_json::from_value(data).map_err(|_| PasApiError::MalformedResponse)
}

#[derive(Debug, Serialize)]
pub struct BookSlotRequest {
    pub patient_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct BookSlotResult {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub slot_id: Option<Uuid>,
    pub start_datetime: String,
    pub end_datetime: String,
    pub status: String,
}

/// `POST /api/slots/{slot_id}/book` on the PAS Axum binary.
pub async fn book_slot(
    slot_id: Uuid,
    req: &BookSlotRequest,
) -> Result<BookSlotResult, PasApiError> {
    let data = post_envelope(&format!("/api/slots/{slot_id}/book"), req).await?;
    serde_json::from_value(data).map_err(|_| PasApiError::MalformedResponse)
}

/// `POST /api/letters/generate` on the PAS Axum binary.
pub async fn generate_letter(req: &GenerateLetterRequest) -> Result<GeneratedLetter, PasApiError> {
    let url = format!("{}/api/letters/generate", base_url());
    let resp = client()?.post(&url).json(req).send().await?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await?;
    if !status.is_success() {
        let message = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("(no message)")
            .to_string();
        return Err(PasApiError::Status {
            status: status.as_u16(),
            message,
        });
    }
    let data = body
        .get("data")
        .ok_or(PasApiError::MalformedResponse)?
        .clone();
    let letter: GeneratedLetter =
        serde_json::from_value(data).map_err(|_| PasApiError::MalformedResponse)?;
    Ok(letter)
}
