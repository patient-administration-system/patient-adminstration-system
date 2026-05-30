//! Operational dashboard.
//!
//! `GET /dashboard` renders the full page server-side via Tera; HTMX in the
//! page then polls four fragment endpoints every few seconds to keep each
//! panel live:
//!
//! - `GET /dashboard/wards`    → ward occupancy table
//! - `GET /dashboard/breaches` → RTT breach table
//! - `GET /dashboard/outbox`   → unpublished-event count
//! - `GET /dashboard/audit`    → last 10 audit rows
//!
//! The initial page render embeds each fragment inline (so the dashboard
//! works without JavaScript on first load); HTMX swaps the `.panel-body`
//! contents on the polling cadence.
//!
//! The templates are compile-time-baked via `include_str!`. Each request
//! runs Tera's `one_off` renderer — no template engine in `AppState`.

use axum::{extract::State, response::Html};
use sea_orm::EntityTrait;
use tera::{Context, Tera};
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::db::entities::ward;
use crate::db::repositories::audit::AuditLogRepository;
use crate::db::repositories::outbox::OutboxRepository;
use crate::models::rtt::compute_active_weeks;

const TEMPLATE_PAGE: &str = include_str!("../../templates/dashboard.html");
const TEMPLATE_WARDS: &str = include_str!("../../templates/dashboard_wards.html");
const TEMPLATE_BREACHES: &str = include_str!("../../templates/dashboard_breaches.html");
const TEMPLATE_OUTBOX: &str = include_str!("../../templates/dashboard_outbox.html");
const TEMPLATE_AUDIT: &str = include_str!("../../templates/dashboard_audit.html");

/// HTMX polling cadence in seconds. Kept short enough to feel live, long
/// enough that four fragment fetches don't hammer the DB.
const REFRESH_SECONDS: u32 = 10;

/// Compact view of [`crate::resources::WardOccupancy`] for the template.
#[derive(serde::Serialize)]
struct WardRow {
    name: String,
    code: String,
    total_beds: usize,
    occupied: usize,
    available: usize,
    /// Everything that isn't `Occupied` or `Available` — cleaning, reserved,
    /// out-of-service. Collapsed into one column for screen brevity.
    other: usize,
}

#[derive(serde::Serialize)]
struct BreachRow {
    target_service: String,
    /// First 8 chars of the patient UUID — enough for an operator to spot a
    /// row, short enough to fit on screen.
    patient_id_short: String,
    weeks_waiting: u32,
    breach_weeks: u32,
}

#[derive(serde::Serialize)]
struct AuditRow {
    at_short: String,
    entity_type: String,
    action: String,
    user_id: Option<String>,
}

/// `GET /dashboard` — render the full dashboard page (no-JS-friendly: each
/// panel ships with its current snapshot baked in, then HTMX live-refreshes
/// on a poll).
pub async fn dashboard_page(State(state): State<AppState>) -> Html<String> {
    let wards = collect_wards(&state).await;
    let breaches = collect_breaches(&state).await;
    let unpublished = collect_unpublished(&state).await;
    let audit = collect_audit(&state).await;

    let wards_html = render_or_error(TEMPLATE_WARDS, &wards_ctx(&wards));
    let breaches_html = render_or_error(TEMPLATE_BREACHES, &breaches_ctx(&breaches));
    let outbox_html = render_or_error(TEMPLATE_OUTBOX, &outbox_ctx(unpublished));
    let audit_html = render_or_error(TEMPLATE_AUDIT, &audit_ctx(&audit));

    let mut ctx = Context::new();
    ctx.insert("version", env!("CARGO_PKG_VERSION"));
    ctx.insert(
        "rendered_at",
        &chrono::Utc::now()
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string(),
    );
    ctx.insert("refresh_seconds", &REFRESH_SECONDS);
    ctx.insert("wards_html", &wards_html);
    ctx.insert("breaches_html", &breaches_html);
    ctx.insert("outbox_html", &outbox_html);
    ctx.insert("audit_html", &audit_html);

    Html(render_or_error(TEMPLATE_PAGE, &ctx))
}

/// `GET /dashboard/wards` — HTMX fragment: ward occupancy panel body only.
pub async fn dashboard_wards(State(state): State<AppState>) -> Html<String> {
    let wards = collect_wards(&state).await;
    Html(render_or_error(TEMPLATE_WARDS, &wards_ctx(&wards)))
}

/// `GET /dashboard/breaches` — HTMX fragment: RTT breach panel body only.
pub async fn dashboard_breaches(State(state): State<AppState>) -> Html<String> {
    let breaches = collect_breaches(&state).await;
    Html(render_or_error(TEMPLATE_BREACHES, &breaches_ctx(&breaches)))
}

/// `GET /dashboard/outbox` — HTMX fragment: outbox unpublished count.
pub async fn dashboard_outbox(State(state): State<AppState>) -> Html<String> {
    let unpublished = collect_unpublished(&state).await;
    Html(render_or_error(TEMPLATE_OUTBOX, &outbox_ctx(unpublished)))
}

/// `GET /dashboard/audit` — HTMX fragment: last 10 audit rows.
pub async fn dashboard_audit(State(state): State<AppState>) -> Html<String> {
    let audit = collect_audit(&state).await;
    Html(render_or_error(TEMPLATE_AUDIT, &audit_ctx(&audit)))
}

// ---------------------------------------------------------------------------
// Data collection
// ---------------------------------------------------------------------------

async fn collect_wards(state: &AppState) -> Vec<WardRow> {
    let wards = ward::Entity::find()
        .all(&state.db)
        .await
        .unwrap_or_default();
    let mut rows: Vec<WardRow> = Vec::with_capacity(wards.len());
    for w in &wards {
        match state.resources.ward_occupancy(w.id).await {
            Ok(occ) => rows.push(WardRow {
                name: w.name.clone(),
                code: w.code.clone(),
                total_beds: occ.total_beds,
                occupied: occ.occupied,
                available: occ.available,
                other: occ.cleaning + occ.reserved + occ.out_of_service,
            }),
            Err(e) => {
                tracing::warn!(target: "pas::dashboard", "ward {} occupancy lookup: {e}", w.id);
            }
        }
    }
    rows.sort_by(|a, b| a.code.cmp(&b.code));
    rows
}

async fn collect_breaches(state: &AppState) -> Vec<BreachRow> {
    use crate::db::entities::rtt_pathway;
    use crate::db::repositories::rtt::RttRepository;
    use sea_orm::{ColumnTrait, QueryFilter};

    let pathways = rtt_pathway::Entity::find()
        .filter(rtt_pathway::Column::Status.ne("stopped"))
        .all(&state.db)
        .await
        .unwrap_or_default();
    let now = chrono::Utc::now();
    let mut out = Vec::new();
    for p in pathways {
        let events = match RttRepository::list_events_for_pathway(&state.db, p.id).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let weeks = compute_active_weeks(&events, now);
        let threshold = p.breach_weeks.max(0) as u32;
        if weeks > threshold {
            out.push(BreachRow {
                target_service: p.target_service,
                patient_id_short: short_uuid(p.patient_id),
                weeks_waiting: weeks,
                breach_weeks: threshold,
            });
        }
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.weeks_waiting));
    out
}

async fn collect_unpublished(state: &AppState) -> usize {
    OutboxRepository::fetch_unpublished(&state.db, 500)
        .await
        .map(|v| v.len())
        .unwrap_or(0)
}

async fn collect_audit(state: &AppState) -> Vec<AuditRow> {
    AuditLogRepository::list_recent(&state.db, 10)
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

// ---------------------------------------------------------------------------
// Tera context builders
// ---------------------------------------------------------------------------

fn wards_ctx(wards: &[WardRow]) -> Context {
    let mut ctx = Context::new();
    ctx.insert("wards", wards);
    ctx
}

fn breaches_ctx(breaches: &[BreachRow]) -> Context {
    let mut ctx = Context::new();
    ctx.insert("breaches", breaches);
    ctx
}

fn outbox_ctx(count: usize) -> Context {
    let mut ctx = Context::new();
    ctx.insert("unpublished_count", &count);
    ctx
}

fn audit_ctx(audit: &[AuditRow]) -> Context {
    let mut ctx = Context::new();
    ctx.insert("audit", audit);
    ctx
}

fn render_or_error(template: &str, ctx: &Context) -> String {
    Tera::one_off(template, ctx, true)
        .unwrap_or_else(|e| format!("<div class=\"empty\">render error: {e}</div>"))
}

fn short_uuid(id: Uuid) -> String {
    let s = id.simple().to_string();
    s.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_uuid_is_eight_chars() {
        let id = Uuid::new_v4();
        let s = short_uuid(id);
        assert_eq!(s.len(), 8);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_wards_fragment_empty_state() {
        let html = render_or_error(TEMPLATE_WARDS, &wards_ctx(&[]));
        assert!(html.contains("No wards configured"));
    }

    #[test]
    fn test_wards_fragment_populated() {
        let rows = vec![WardRow {
            name: "Ward A".into(),
            code: "WARD-A".into(),
            total_beds: 10,
            occupied: 7,
            available: 2,
            other: 1,
        }];
        let html = render_or_error(TEMPLATE_WARDS, &wards_ctx(&rows));
        assert!(html.contains("Ward A"));
        assert!(html.contains("WARD-A"));
        assert!(html.contains(">7<"));
        // Lily semantic markup: data-table family + badge.
        assert!(html.contains("class=\"data-table\""));
        assert!(html.contains("class=\"data-table-head\""));
        assert!(html.contains("class=\"data-table-body\""));
        assert!(html.contains("class=\"badge\""));
        assert!(html.contains("data-status=\"warn\""));
        assert!(html.contains("data-status=\"ok\""));
    }

    #[test]
    fn test_breaches_fragment_empty_state() {
        let html = render_or_error(TEMPLATE_BREACHES, &breaches_ctx(&[]));
        assert!(html.contains("No active pathways"));
    }

    #[test]
    fn test_breaches_fragment_populated() {
        let rows = vec![BreachRow {
            target_service: "cardiology".into(),
            patient_id_short: "abc12345".into(),
            weeks_waiting: 24,
            breach_weeks: 18,
        }];
        let html = render_or_error(TEMPLATE_BREACHES, &breaches_ctx(&rows));
        assert!(html.contains("cardiology"));
        assert!(html.contains("abc12345"));
        assert!(html.contains(">24<"));
        // Lily semantic markup: data-table family + badge + code.
        assert!(html.contains("class=\"data-table\""));
        assert!(html.contains("class=\"badge\""));
        assert!(html.contains("data-status=\"error\""));
        assert!(html.contains("class=\"code\""));
    }

    #[test]
    fn test_outbox_fragment_zero() {
        let html = render_or_error(TEMPLATE_OUTBOX, &outbox_ctx(0));
        assert!(html.contains("All events delivered"));
    }

    #[test]
    fn test_outbox_fragment_nonzero_uses_warn_badge_and_plural() {
        let html = render_or_error(TEMPLATE_OUTBOX, &outbox_ctx(3));
        assert!(html.contains("unpublished events"));
        // Lily `badge` carries the status via data-status; the warn-color
        // styling sits in the page-level CSS.
        assert!(html.contains("data-status=\"warn\""));
        assert!(html.contains(">3<"));
    }

    #[test]
    fn test_outbox_fragment_single_is_singular() {
        let html = render_or_error(TEMPLATE_OUTBOX, &outbox_ctx(1));
        assert!(html.contains("unpublished event"));
        assert!(!html.contains("unpublished events"));
    }

    #[test]
    fn test_audit_fragment_empty_state() {
        let html = render_or_error(TEMPLATE_AUDIT, &audit_ctx(&[]));
        assert!(html.contains("No audit entries"));
    }

    #[test]
    fn test_audit_fragment_populated() {
        let rows = vec![AuditRow {
            at_short: "2026-05-23 11:00:00".into(),
            entity_type: "patient".into(),
            action: "create".into(),
            user_id: Some("alice".into()),
        }];
        let html = render_or_error(TEMPLATE_AUDIT, &audit_ctx(&rows));
        assert!(html.contains("alice"));
        assert!(html.contains("patient"));
        assert!(html.contains("create"));
        // Lily semantic markup.
        assert!(html.contains("class=\"data-table\""));
        assert!(html.contains("class=\"badge\""));
        assert!(html.contains("class=\"code\""));
    }

    #[test]
    fn test_page_uses_lily_class_names() {
        // The main page should render Lily `header` and `footer` on the
        // top-level chrome and `panel` (with `role="region"`) for each card.
        let mut ctx = Context::new();
        ctx.insert("version", "0.3.2");
        ctx.insert("rendered_at", "2026-05-23 12:00:00 UTC");
        ctx.insert("refresh_seconds", &REFRESH_SECONDS);
        ctx.insert("wards_html", "");
        ctx.insert("breaches_html", "");
        ctx.insert("outbox_html", "");
        ctx.insert("audit_html", "");
        let html = render_or_error(TEMPLATE_PAGE, &ctx);
        assert!(html.contains("class=\"header\""));
        assert!(html.contains("class=\"footer\""));
        // Every panel is a Lily `panel` with role="region".
        let panels = html.matches("class=\"panel\"").count();
        assert_eq!(panels, 4, "expected 4 Lily panels");
        let regions = html.matches("role=\"region\"").count();
        assert_eq!(regions, 4, "expected 4 ARIA regions");
        // Footer mentions Lily lineage.
        assert!(html.contains("Lily Design System"));
    }

    #[test]
    fn test_page_template_embeds_all_fragments_and_htmx() {
        // Render the full page with sentinel fragments and assert that
        // (a) HTMX is loaded, (b) each panel carries the right hx-get URL,
        // (c) the inlined fragment markers survive `| safe` rendering.
        let mut ctx = Context::new();
        ctx.insert("version", "0.3.1");
        ctx.insert("rendered_at", "2026-05-23 12:00:00 UTC");
        ctx.insert("refresh_seconds", &REFRESH_SECONDS);
        ctx.insert("wards_html", "<!--WARDS-FRAGMENT-->");
        ctx.insert("breaches_html", "<!--BREACHES-FRAGMENT-->");
        ctx.insert("outbox_html", "<!--OUTBOX-FRAGMENT-->");
        ctx.insert("audit_html", "<!--AUDIT-FRAGMENT-->");
        let html = render_or_error(TEMPLATE_PAGE, &ctx);

        // HTMX script reference.
        assert!(
            html.contains("htmx.org@"),
            "expected HTMX CDN reference in page"
        );
        // One hx-get per panel pointing at its fragment endpoint.
        for path in [
            "hx-get=\"/dashboard/wards\"",
            "hx-get=\"/dashboard/breaches\"",
            "hx-get=\"/dashboard/outbox\"",
            "hx-get=\"/dashboard/audit\"",
        ] {
            assert!(html.contains(path), "missing {path} in page");
        }
        // The polling cadence is interpolated from the const.
        assert!(html.contains("every 10s"));
        // Inlined fragments survive `| safe` (no HTML-escaping of the
        // comment markers we passed through).
        for marker in [
            "<!--WARDS-FRAGMENT-->",
            "<!--BREACHES-FRAGMENT-->",
            "<!--OUTBOX-FRAGMENT-->",
            "<!--AUDIT-FRAGMENT-->",
        ] {
            assert!(html.contains(marker), "missing fragment marker {marker}");
        }
    }
}
