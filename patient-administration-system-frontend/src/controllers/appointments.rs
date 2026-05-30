//! Appointment booking — `GET /appointments/new` (form) + `POST` (submit).
//!
//! The form lists every free slot from every active schedule in the next
//! `SLOT_HORIZON_DAYS` days, labeled with `service · date time → time`,
//! and a patient picker. Submit calls `POST /api/slots/{slot_id}/book` on
//! the PAS Axum API so the transactional booking path runs in the system
//! of record.
//!
//! No JS-driven schedule→slot dependent dropdown; one flat picker per
//! booking. This keeps the page Loco-prelude-only (no axum::extract::Query
//! re-export needed) and avoids round-tripping through the GET form.

use axum_extra::extract::cookie::CookieJar;
use loco_rs::prelude::*;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::csrf;
use crate::models::{
    patient::list_active_patients,
    schedule::{list_active_schedules, list_free_slots},
};
use crate::services::pas_api::{BookSlotRequest, PasApiError, book_slot};

const SLOT_HORIZON_DAYS: i64 = 14;

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/appointments")
        .add("/new", get(new_form).post(submit))
}

#[derive(Debug, serde::Serialize)]
struct LabeledSlot {
    id: Uuid,
    label: String,
}

/// `GET /appointments/new` — render the picker form.
pub async fn new_form(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    jar: CookieJar,
) -> Result<(CookieJar, Response)> {
    let (csrf_token, jar) = csrf::ensure_token(jar);
    let resp = render_form(&v, &ctx, &csrf_token, None, None).await?;
    Ok((jar, resp))
}

#[derive(Debug, Deserialize)]
pub struct BookForm {
    pub slot_id: Uuid,
    pub patient_id: Uuid,
    pub csrf_token: String,
}

/// `POST /appointments/new` — verify CSRF then submit the booking.
pub async fn submit(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    jar: CookieJar,
    Form(form): Form<BookForm>,
) -> Result<(CookieJar, Response)> {
    csrf::verify_token(&jar, &form.csrf_token)?;
    let (csrf_token, jar) = csrf::ensure_token(jar);
    let req = BookSlotRequest {
        patient_id: form.patient_id,
    };
    let (error, result) = match book_slot(form.slot_id, &req).await {
        Ok(out) => (
            None,
            Some(json!({
                "appointment_id": out.id,
                "patient_id": out.patient_id,
                "start": out.start_datetime,
                "end": out.end_datetime,
                "status": out.status,
            })),
        ),
        Err(e) => (Some(format_err(e)), None),
    };
    let resp = render_form(&v, &ctx, &csrf_token, error, result).await?;
    Ok((jar, resp))
}

async fn render_form(
    v: &TeraView,
    ctx: &AppContext,
    csrf_token: &str,
    error: Option<String>,
    result: Option<serde_json::Value>,
) -> Result<Response> {
    let schedules = list_active_schedules(&ctx.db).await.map_err(Error::wrap)?;
    let mut labeled: Vec<LabeledSlot> = Vec::new();
    for s in &schedules {
        let slots = list_free_slots(&ctx.db, s.id, SLOT_HORIZON_DAYS)
            .await
            .map_err(Error::wrap)?;
        for slot in slots {
            labeled.push(LabeledSlot {
                id: slot.id,
                label: format!("{} · {}", s.service_type, slot.label),
            });
        }
    }
    let patients = list_active_patients(&ctx.db, 200)
        .await
        .map_err(Error::wrap)?;
    format::render().view(
        v,
        "appointments/new.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "slots": labeled,
            "patients": patients,
            "horizon_days": SLOT_HORIZON_DAYS,
            "csrf_token": csrf_token,
            "error": error,
            "result": result,
        }),
    )
}

fn format_err(e: PasApiError) -> String {
    match e {
        PasApiError::Http(err) => format!("PAS API HTTP: {err}"),
        PasApiError::Status { status, message } => format!("PAS API returned {status}: {message}"),
        PasApiError::MalformedResponse => "PAS API: malformed response".to_string(),
    }
}
