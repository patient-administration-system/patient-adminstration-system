//! rtt repository — RTTPathway and RTTClockEvent

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::{rtt_clock_event, rtt_pathway};
use crate::models::rtt::{RTTClockEvent, RTTEventKind, RTTPathway, RTTStatus};
use crate::{Error, Result};

pub struct RttRepository;

impl RttRepository {
    pub async fn create_pathway<C: ConnectionTrait>(
        conn: &C,
        p: &RTTPathway,
    ) -> Result<RTTPathway> {
        let am = pathway_to_active_model(p)?;
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(p.clone())
    }

    pub async fn find_pathway_by_id<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<RTTPathway>> {
        let m = rtt_pathway::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(pathway_from_model).transpose()
    }

    pub async fn list_pathways_by_patient<C: ConnectionTrait>(
        conn: &C,
        patient_id: Uuid,
    ) -> Result<Vec<RTTPathway>> {
        let rows = rtt_pathway::Entity::find()
            .filter(rtt_pathway::Column::PatientId.eq(patient_id))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(pathway_from_model).collect()
    }

    pub async fn create_event<C: ConnectionTrait>(
        conn: &C,
        e: &RTTClockEvent,
    ) -> Result<RTTClockEvent> {
        let am = event_to_active_model(e);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(e.clone())
    }

    pub async fn list_events_for_pathway<C: ConnectionTrait>(
        conn: &C,
        pathway_id: Uuid,
    ) -> Result<Vec<RTTClockEvent>> {
        let rows = rtt_clock_event::Entity::find()
            .filter(rtt_clock_event::Column::PathwayId.eq(pathway_id))
            .order_by_asc(rtt_clock_event::Column::EventAt)
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(event_from_model).collect()
    }
}

// --- conversion helpers ---

pub(crate) fn rtt_status_to_str(s: RTTStatus) -> &'static str {
    match s {
        RTTStatus::Active => "active",
        RTTStatus::Paused => "paused",
        RTTStatus::Stopped => "stopped",
    }
}

pub(crate) fn rtt_status_from_str(s: &str) -> Result<RTTStatus> {
    match s {
        "active" => Ok(RTTStatus::Active),
        "paused" => Ok(RTTStatus::Paused),
        "stopped" => Ok(RTTStatus::Stopped),
        other => Err(Error::internal(format!("unknown rtt status: {other}"))),
    }
}

pub(crate) fn rtt_event_kind_to_str(k: RTTEventKind) -> &'static str {
    match k {
        RTTEventKind::Started => "started",
        RTTEventKind::Paused => "paused",
        RTTEventKind::Resumed => "resumed",
        RTTEventKind::Stopped => "stopped",
    }
}

pub(crate) fn rtt_event_kind_from_str(s: &str) -> Result<RTTEventKind> {
    match s {
        "started" => Ok(RTTEventKind::Started),
        "paused" => Ok(RTTEventKind::Paused),
        "resumed" => Ok(RTTEventKind::Resumed),
        "stopped" => Ok(RTTEventKind::Stopped),
        other => Err(Error::internal(format!("unknown rtt event kind: {other}"))),
    }
}

fn pathway_to_active_model(p: &RTTPathway) -> Result<rtt_pathway::ActiveModel> {
    let breach_weeks_i32: i32 = p.breach_weeks.try_into().map_err(|_| {
        Error::internal(format!(
            "breach_weeks {} does not fit in i32",
            p.breach_weeks
        ))
    })?;
    Ok(rtt_pathway::ActiveModel {
        id: Set(p.id),
        patient_id: Set(p.patient_id),
        target_service: Set(p.target_service.clone()),
        breach_weeks: Set(breach_weeks_i32),
        status: Set(rtt_status_to_str(p.status).to_string()),
        started_at: Set(p.started_at.fixed_offset()),
        stopped_at: Set(p.stopped_at.map(|t| t.fixed_offset())),
        created_at: Set(p.created_at.fixed_offset()),
        updated_at: Set(p.updated_at.fixed_offset()),
    })
}

fn pathway_from_model(m: rtt_pathway::Model) -> Result<RTTPathway> {
    let breach_weeks: u32 = m.breach_weeks.try_into().map_err(|_| {
        Error::internal(format!(
            "breach_weeks {} is negative; cannot convert to u32",
            m.breach_weeks
        ))
    })?;
    Ok(RTTPathway {
        id: m.id,
        patient_id: m.patient_id,
        target_service: m.target_service,
        breach_weeks,
        status: rtt_status_from_str(&m.status)?,
        started_at: m.started_at.with_timezone(&chrono::Utc),
        stopped_at: m.stopped_at.map(|t| t.with_timezone(&chrono::Utc)),
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

fn event_to_active_model(e: &RTTClockEvent) -> rtt_clock_event::ActiveModel {
    rtt_clock_event::ActiveModel {
        id: Set(e.id),
        pathway_id: Set(e.pathway_id),
        kind: Set(rtt_event_kind_to_str(e.kind).to_string()),
        reason: Set(e.reason.clone()),
        event_at: Set(e.event_at.fixed_offset()),
        created_at: Set(e.created_at.fixed_offset()),
    }
}

fn event_from_model(m: rtt_clock_event::Model) -> Result<RTTClockEvent> {
    Ok(RTTClockEvent {
        id: m.id,
        pathway_id: m.pathway_id,
        kind: rtt_event_kind_from_str(&m.kind)?,
        reason: m.reason,
        event_at: m.event_at.with_timezone(&chrono::Utc),
        created_at: m.created_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtt_pathway_roundtrip_via_active_model() {
        let p = RTTPathway::new(Uuid::new_v4(), "cardiology");
        let am = pathway_to_active_model(&p).expect("to active model");
        let m = rtt_pathway::Model {
            id: am.id.clone().unwrap(),
            patient_id: am.patient_id.clone().unwrap(),
            target_service: am.target_service.clone().unwrap(),
            breach_weeks: am.breach_weeks.clone().unwrap(),
            status: am.status.clone().unwrap(),
            started_at: am.started_at.clone().unwrap(),
            stopped_at: am.stopped_at.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = pathway_from_model(m).expect("from model");
        assert_eq!(back.id, p.id);
        assert_eq!(back.breach_weeks, 18);
        assert_eq!(back.status, RTTStatus::Active);
        assert_eq!(back.target_service, "cardiology");
    }
}
