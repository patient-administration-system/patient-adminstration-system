//! Dashboard controller — ports the PAS Axum dashboard handler into Loco's
//! controller/view shape.
//!
//! The full page lives at `/dashboard`; each panel has its own
//! HTMX-targeted fragment endpoint so the dashboard refreshes per-panel
//! without a full page swap.

use loco_rs::prelude::*;
use serde::Serialize;
use serde_json::json;

use crate::models::{audit::list_recent_audit, outbox::count_unpublished, ward::list_active_wards};

const REFRESH_SECONDS: u32 = 10;

#[derive(Serialize)]
struct WardRow {
    id: uuid::Uuid,
    name: String,
    code: String,
    total_beds: usize,
    occupied: usize,
    available: usize,
    other: usize,
}

#[derive(Serialize)]
struct AuditRow {
    at_short: String,
    entity_type: String,
    action: String,
    user_id: Option<String>,
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/dashboard")
        .add("/", get(index))
        .add("/wards", get(wards_fragment))
        .add("/outbox", get(outbox_fragment))
        .add("/audit", get(audit_fragment))
}

/// `GET /dashboard` — render the full Tera page with each panel inlined,
/// so the dashboard works without JavaScript. HTMX layers on as
/// progressive enhancement.
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let wards = load_wards(&ctx).await;
    let unpublished = count_unpublished(&ctx.db).await.unwrap_or(0);
    let audit = load_audit(&ctx).await;

    format::render().view(
        &v,
        "dashboard/index.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "rendered_at": chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            "refresh_seconds": REFRESH_SECONDS,
            "wards": wards,
            "unpublished_count": unpublished,
            "audit": audit,
        }),
    )
}

/// `GET /dashboard/wards` — HTMX fragment.
pub async fn wards_fragment(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let wards = load_wards(&ctx).await;
    format::render().view(&v, "dashboard/_wards.html", json!({ "wards": wards }))
}

/// `GET /dashboard/outbox` — HTMX fragment: single big-stat unpublished count.
pub async fn outbox_fragment(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let unpublished = count_unpublished(&ctx.db).await.unwrap_or(0);
    format::render().view(
        &v,
        "dashboard/_outbox.html",
        json!({ "unpublished_count": unpublished }),
    )
}

/// `GET /dashboard/audit` — HTMX fragment: last 10 audit rows.
pub async fn audit_fragment(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let audit = load_audit(&ctx).await;
    format::render().view(&v, "dashboard/_audit.html", json!({ "audit": audit }))
}

// ---- data loaders ---------------------------------------------------------

async fn load_wards(ctx: &AppContext) -> Vec<WardRow> {
    let mut rows = Vec::new();
    let wards = list_active_wards(&ctx.db).await.unwrap_or_default();
    for w in wards {
        let beds = crate::models::bed::list_by_ward(&ctx.db, w.id)
            .await
            .unwrap_or_default();
        let total_beds = beds.len();
        let mut occupied = 0;
        let mut available = 0;
        let mut other = 0;
        for b in &beds {
            match b.status.as_str() {
                "occupied" => occupied += 1,
                "available" => available += 1,
                _ => other += 1,
            }
        }
        rows.push(WardRow {
            id: w.id,
            name: w.name,
            code: w.code,
            total_beds,
            occupied,
            available,
            other,
        });
    }
    rows.sort_by(|a, b| a.code.cmp(&b.code));
    rows
}

async fn load_audit(ctx: &AppContext) -> Vec<AuditRow> {
    list_recent_audit(&ctx.db, 10)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|m| AuditRow {
            at_short: m.at.format("%Y-%m-%d %H:%M:%S").to_string(),
            entity_type: m.entity_type,
            action: m.action,
            user_id: m.user_id,
        })
        .collect()
}
