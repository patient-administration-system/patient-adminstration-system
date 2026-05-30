//! Patients controller — list + detail pages.

use loco_rs::prelude::*;
use serde_json::json;
use uuid::Uuid;

use crate::models::patient::{find_patient_by_id, list_active_patients};

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/patients")
        .add("/", get(index))
        .add("/{id}", get(show))
}

/// `GET /patients` — list active patients.
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let patients = list_active_patients(&ctx.db, 100).await.unwrap_or_default();
    format::render().view(
        &v,
        "patients/index.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "patients": patients,
        }),
    )
}

/// `GET /patients/:id` — patient detail.
pub async fn show(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let Some(p) = find_patient_by_id(&ctx.db, id).await? else {
        return Err(Error::NotFound);
    };
    format::render().view(
        &v,
        "patients/show.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "patient": p,
        }),
    )
}
