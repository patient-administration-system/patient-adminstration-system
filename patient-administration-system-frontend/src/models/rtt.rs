//! RTT (Referral-To-Treatment) pathways — read-side projection for the
//! `/rtt` cockpit.
//!
//! Loads every non-stopped pathway, walks its `rtt_clock_events` to compute
//! weeks-waiting, and joins to the patient's name. The weeks-waiting
//! formula mirrors `crate::models::rtt::compute_active_weeks` on the PAS
//! Axum side — sum of unpaused intervals from `started`/`resumed` events
//! to either the next `paused`/`stopped` event or `now`.

use chrono::{DateTime, FixedOffset, Utc};
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use std::collections::HashMap;
use uuid::Uuid;

mod pathway_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "rtt_pathways")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub patient_id: Uuid,
        pub target_service: String,
        pub breach_weeks: i32,
        pub status: String,
        pub started_at: chrono::DateTime<chrono::FixedOffset>,
        pub stopped_at: Option<chrono::DateTime<chrono::FixedOffset>>,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod event_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "rtt_clock_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub pathway_id: Uuid,
        pub kind: String,
        pub reason: Option<String>,
        pub event_at: chrono::DateTime<chrono::FixedOffset>,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod patient_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "patients")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub mpi_id: Option<Uuid>,
        pub active: bool,
        pub name: serde_json::Value,
        pub additional_names: serde_json::Value,
        pub identifiers: serde_json::Value,
        pub telecom: serde_json::Value,
        pub addresses: serde_json::Value,
        pub gender: String,
        pub birth_date: Option<chrono::NaiveDate>,
        pub deceased: bool,
        pub deceased_datetime: Option<chrono::DateTime<chrono::FixedOffset>>,
        pub emergency_contacts: serde_json::Value,
        pub marital_status: Option<String>,
        pub deleted_at: Option<chrono::DateTime<chrono::FixedOffset>>,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// One row in the `/rtt` cockpit.
#[derive(Debug, Clone, Serialize)]
pub struct PathwayRow {
    pub pathway_id: Uuid,
    pub patient_id: Uuid,
    pub patient_family: String,
    pub patient_given: String,
    pub target_service: String,
    pub status: String,
    pub weeks_waiting: u32,
    pub breach_weeks: u32,
    pub is_breaching: bool,
}

/// Load every non-stopped pathway, compute weeks-waiting per pathway, and
/// return rows sorted worst-breach-first. The page renders the result
/// straight to a Lily `data-table`.
pub async fn list_active_pathways(db: &DatabaseConnection) -> Result<Vec<PathwayRow>, DbErr> {
    let pathways = pathway_entity::Entity::find()
        .filter(pathway_entity::Column::Status.ne("stopped"))
        .all(db)
        .await?;
    if pathways.is_empty() {
        return Ok(Vec::new());
    }

    // Patient name lookup table.
    let patient_ids: Vec<Uuid> = pathways.iter().map(|p| p.patient_id).collect();
    let patients = patient_entity::Entity::find()
        .filter(patient_entity::Column::Id.is_in(patient_ids))
        .all(db)
        .await?;
    let patient_by_id: HashMap<Uuid, patient_entity::Model> =
        patients.into_iter().map(|p| (p.id, p)).collect();

    let now = Utc::now();
    let mut rows = Vec::with_capacity(pathways.len());
    for p in pathways {
        // Pull events for this pathway in chronological order.
        let events = event_entity::Entity::find()
            .filter(event_entity::Column::PathwayId.eq(p.id))
            .order_by_asc(event_entity::Column::EventAt)
            .all(db)
            .await?;
        let weeks = compute_active_weeks_from_models(&events, now);
        let breach_weeks = p.breach_weeks.max(0) as u32;
        let (family, given) = patient_by_id
            .get(&p.patient_id)
            .map(|m| name_parts(&m.name))
            .unwrap_or_else(|| ("(unknown)".into(), String::new()));
        rows.push(PathwayRow {
            pathway_id: p.id,
            patient_id: p.patient_id,
            patient_family: family,
            patient_given: given,
            target_service: p.target_service,
            status: p.status,
            weeks_waiting: weeks,
            breach_weeks,
            is_breaching: weeks > breach_weeks,
        });
    }

    // Worst breaches first (largest excess over threshold), then by raw
    // weeks waiting, then alphabetical on service.
    rows.sort_by(|a, b| {
        let a_excess = a.weeks_waiting as i64 - a.breach_weeks as i64;
        let b_excess = b.weeks_waiting as i64 - b.breach_weeks as i64;
        b_excess
            .cmp(&a_excess)
            .then_with(|| b.weeks_waiting.cmp(&a.weeks_waiting))
            .then_with(|| a.target_service.cmp(&b.target_service))
    });
    Ok(rows)
}

/// Sum unpaused intervals from `started`/`resumed` events to either the
/// next `paused`/`stopped` event or `now`, then floor-divide by a week.
/// Mirrors `compute_active_weeks` from the PAS Axum lib.
fn compute_active_weeks_from_models(events: &[event_entity::Model], now: DateTime<Utc>) -> u32 {
    if events.is_empty() {
        return 0;
    }
    let mut total_seconds: i64 = 0;
    let mut active_since: Option<DateTime<FixedOffset>> = None;
    for e in events {
        match e.kind.as_str() {
            "started" | "resumed" => active_since = Some(e.event_at),
            "paused" | "stopped" => {
                if let Some(start) = active_since.take() {
                    total_seconds += (e.event_at - start).num_seconds();
                }
            }
            _ => {} // unknown kind — ignore
        }
    }
    if let Some(start) = active_since {
        let now_fixed: DateTime<FixedOffset> =
            now.with_timezone(&FixedOffset::east_opt(0).unwrap());
        total_seconds += (now_fixed - start).num_seconds();
    }
    const SECONDS_PER_WEEK: i64 = 7 * 24 * 3600;
    if total_seconds < 0 {
        return 0;
    }
    (total_seconds / SECONDS_PER_WEEK) as u32
}

fn name_parts(v: &serde_json::Value) -> (String, String) {
    let family = v
        .get("family")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let given = v
        .get("given")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    (family, given)
}
