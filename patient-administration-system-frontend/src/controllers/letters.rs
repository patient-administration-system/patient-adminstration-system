//! Letter composer controller.
//!
//! `GET /letters/new` renders the compose form (template + patient + channel
//! picker). `POST /letters/new` calls the PAS Axum API
//! (`POST /api/letters/generate`) so the actual letter generation runs
//! through the PAS service layer — audit row + outbox event + Tera render
//! all happen server-side in the system of record. The Loco UI just
//! ferries form data.

use axum_extra::extract::cookie::CookieJar;
use loco_rs::prelude::*;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::csrf;
use crate::models::{letter_template::list_active_templates, patient::list_active_patients};
use crate::services::pas_api::{GenerateLetterRequest, PasApiError, generate_letter};

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/letters")
        .add("/new", get(new_form).post(submit))
}

/// `GET /letters/new` — compose form.
pub async fn new_form(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    jar: CookieJar,
) -> Result<(CookieJar, Response)> {
    let (csrf_token, jar) = csrf::ensure_token(jar);
    let templates = list_active_templates(&ctx.db).await.map_err(Error::wrap)?;
    let patients = list_active_patients(&ctx.db, 200)
        .await
        .map_err(Error::wrap)?;
    let resp = format::render().view(
        &v,
        "letters/new.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "templates": templates,
            "patients": patients,
            "csrf_token": csrf_token,
            "error": serde_json::Value::Null,
            "result": serde_json::Value::Null,
        }),
    )?;
    Ok((jar, resp))
}

/// Form payload for `POST /letters/new`. `channel` is a plain string
/// because the underlying PAS API expects the lowercase enum tag.
#[derive(Debug, Deserialize)]
pub struct ComposeForm {
    pub template_id: Uuid,
    pub patient_id: Uuid,
    pub channel: String,
    pub csrf_token: String,
}

/// `POST /letters/new` — verify CSRF, call the PAS API to generate the
/// letter, then re-render the form page with either the rendered letter
/// inline or an error banner.
pub async fn submit(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    jar: CookieJar,
    Form(form): Form<ComposeForm>,
) -> Result<(CookieJar, Response)> {
    csrf::verify_token(&jar, &form.csrf_token)?;
    let (csrf_token, jar) = csrf::ensure_token(jar);
    let req = GenerateLetterRequest {
        template_id: form.template_id,
        patient_id: form.patient_id,
        appointment_id: None,
        channel: form.channel,
        extra: serde_json::json!({}),
    };
    let (error, result) = match generate_letter(&req).await {
        Ok(letter) => (
            serde_json::Value::Null,
            json!({
                "id": letter.id,
                "subject": letter.rendered_subject,
                "body": letter.rendered_body,
                "channel": letter.channel,
                "status": letter.status,
            }),
        ),
        Err(e) => {
            let msg = match e {
                PasApiError::Http(err) => format!("PAS API HTTP: {err}"),
                PasApiError::Status { status, message } => {
                    format!("PAS API returned {status}: {message}")
                }
                PasApiError::MalformedResponse => "PAS API: malformed response".to_string(),
            };
            (serde_json::Value::String(msg), serde_json::Value::Null)
        }
    };

    let templates = list_active_templates(&ctx.db).await.map_err(Error::wrap)?;
    let patients = list_active_patients(&ctx.db, 200)
        .await
        .map_err(Error::wrap)?;
    let resp = format::render().view(
        &v,
        "letters/new.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "templates": templates,
            "patients": patients,
            "csrf_token": csrf_token,
            "error": error,
            "result": result,
        }),
    )?;
    Ok((jar, resp))
}
