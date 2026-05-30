//! waitlist repository

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::waitlist_entry;
use crate::models::waitlist::{Priority, WaitlistEntry, WaitlistStatus};
use crate::{Error, Result};

pub struct WaitlistRepository;

impl WaitlistRepository {
    pub async fn create_entry<C: ConnectionTrait>(
        conn: &C,
        e: &WaitlistEntry,
    ) -> Result<WaitlistEntry> {
        let am = to_active_model(e);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(e.clone())
    }

    pub async fn find_entry_by_id<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<WaitlistEntry>> {
        let m = waitlist_entry::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    pub async fn update_entry<C: ConnectionTrait>(
        conn: &C,
        e: &WaitlistEntry,
    ) -> Result<WaitlistEntry> {
        let mut am = to_active_model(e);
        am.created_at = NotSet;
        am.update(conn).await.map_err(Error::Database)?;
        Ok(e.clone())
    }

    pub async fn list_by_service<C: ConnectionTrait>(
        conn: &C,
        service: &str,
    ) -> Result<Vec<WaitlistEntry>> {
        let rows = waitlist_entry::Entity::find()
            .filter(waitlist_entry::Column::TargetService.eq(service))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    pub async fn list_by_priority<C: ConnectionTrait>(
        conn: &C,
        priority: Priority,
    ) -> Result<Vec<WaitlistEntry>> {
        let rows = waitlist_entry::Entity::find()
            .filter(waitlist_entry::Column::Priority.eq(priority_to_str(priority)))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Returns all waitlist entries whose corresponding RTT pathways are
    /// in breach. Implementation note: v0.1 delegates the RTT computation
    /// to `RTTRepository::is_pathway_breaching` once available; here we
    /// just return the empty list as a v0.1 placeholder. Real
    /// implementation joins waitlist_entries → rtt_pathways → events.
    pub async fn list_breaches<C: ConnectionTrait>(
        _conn: &C,
        _now: chrono::DateTime<chrono::Utc>,
        _threshold_weeks: u32,
    ) -> Result<Vec<WaitlistEntry>> {
        // Joining across waitlist_entries → rtt_pathways → rtt_clock_events
        // requires a multi-step query; the higher-level `waitlist` service
        // (Layer 3) composes the two repos. The repository alone returns
        // empty.
        Ok(Vec::new())
    }
}

// --- conversion helpers ---

pub(crate) fn priority_to_str(p: Priority) -> &'static str {
    match p {
        Priority::Routine => "routine",
        Priority::Urgent => "urgent",
        Priority::TwoWeekWait => "two_week_wait",
        Priority::Emergency => "emergency",
    }
}

pub(crate) fn priority_from_str(s: &str) -> Result<Priority> {
    match s {
        "routine" => Ok(Priority::Routine),
        "urgent" => Ok(Priority::Urgent),
        "two_week_wait" => Ok(Priority::TwoWeekWait),
        "emergency" => Ok(Priority::Emergency),
        other => Err(Error::internal(format!("unknown priority: {other}"))),
    }
}

pub(crate) fn waitlist_status_to_str(s: WaitlistStatus) -> &'static str {
    match s {
        WaitlistStatus::Waiting => "waiting",
        WaitlistStatus::Booked => "booked",
        WaitlistStatus::Removed => "removed",
        WaitlistStatus::Treated => "treated",
    }
}

pub(crate) fn waitlist_status_from_str(s: &str) -> Result<WaitlistStatus> {
    match s {
        "waiting" => Ok(WaitlistStatus::Waiting),
        "booked" => Ok(WaitlistStatus::Booked),
        "removed" => Ok(WaitlistStatus::Removed),
        "treated" => Ok(WaitlistStatus::Treated),
        other => Err(Error::internal(format!("unknown waitlist status: {other}"))),
    }
}

fn to_active_model(e: &WaitlistEntry) -> waitlist_entry::ActiveModel {
    waitlist_entry::ActiveModel {
        id: Set(e.id),
        referral_id: Set(e.referral_id),
        patient_id: Set(e.patient_id),
        target_service: Set(e.target_service.clone()),
        priority: Set(priority_to_str(e.priority).to_string()),
        status: Set(waitlist_status_to_str(e.status).to_string()),
        created_at: Set(e.created_at.fixed_offset()),
        updated_at: Set(e.updated_at.fixed_offset()),
    }
}

fn from_model(m: waitlist_entry::Model) -> Result<WaitlistEntry> {
    Ok(WaitlistEntry {
        id: m.id,
        referral_id: m.referral_id,
        patient_id: m.patient_id,
        target_service: m.target_service,
        priority: priority_from_str(&m.priority)?,
        status: waitlist_status_from_str(&m.status)?,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_waitlist_entry_roundtrip_via_active_model() {
        let e = WaitlistEntry::new(Uuid::new_v4(), "cardiology", Priority::Urgent);
        let am = to_active_model(&e);
        let m = waitlist_entry::Model {
            id: am.id.clone().unwrap(),
            referral_id: am.referral_id.clone().unwrap(),
            patient_id: am.patient_id.clone().unwrap(),
            target_service: am.target_service.clone().unwrap(),
            priority: am.priority.clone().unwrap(),
            status: am.status.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, e.id);
        assert_eq!(back.priority, Priority::Urgent);
        assert_eq!(back.status, WaitlistStatus::Waiting);
        assert_eq!(back.target_service, "cardiology");
    }
}
