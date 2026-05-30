//! waitlist

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Clinical priority for a waitlist entry.
///
/// Variant order is meaningful: `Routine` is the lowest priority and
/// `Emergency` is the highest. `PartialOrd`/`Ord` are derived, so comparisons
/// respect the declaration order — that is,
/// `Routine < Urgent < TwoWeekWait < Emergency`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Routine,
    Urgent,
    TwoWeekWait,
    Emergency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitlistStatus {
    Waiting,
    Booked,
    Removed,
    Treated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Referral {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub referring_practitioner_id: Option<Uuid>,
    pub target_service: String,
    pub reason: Option<String>,
    pub received_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl Referral {
    pub fn new(patient_id: Uuid, target_service: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            patient_id,
            referring_practitioner_id: None,
            target_service: target_service.into(),
            reason: None,
            received_at: now,
            created_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitlistEntry {
    pub id: Uuid,
    pub referral_id: Option<Uuid>,
    pub patient_id: Uuid,
    pub target_service: String,
    pub priority: Priority,
    pub status: WaitlistStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WaitlistEntry {
    pub fn new(patient_id: Uuid, target_service: impl Into<String>, priority: Priority) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            referral_id: None,
            patient_id,
            target_service: target_service.into(),
            priority,
            status: WaitlistStatus::Waiting,
            created_at: now,
            updated_at: now,
        }
    }

    /// Days elapsed between `created_at` and `now` (floor, in whole days).
    pub fn days_waiting(&self, now: DateTime<Utc>) -> i64 {
        (now - self.created_at).num_days()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_referral_new_defaults() {
        let patient_id = Uuid::new_v4();
        let r = Referral::new(patient_id, "cardiology");
        assert_eq!(r.patient_id, patient_id);
        assert_eq!(r.target_service, "cardiology");
        assert!(r.referring_practitioner_id.is_none());
        assert!(r.reason.is_none());
        assert_eq!(r.received_at, r.created_at);
    }

    #[test]
    fn test_waitlist_entry_new_defaults() {
        let patient_id = Uuid::new_v4();
        let e = WaitlistEntry::new(patient_id, "orthopedics", Priority::Routine);
        assert_eq!(e.patient_id, patient_id);
        assert_eq!(e.target_service, "orthopedics");
        assert_eq!(e.priority, Priority::Routine);
        assert_eq!(e.status, WaitlistStatus::Waiting);
        assert!(e.referral_id.is_none());
        assert_eq!(e.created_at, e.updated_at);
    }

    #[test]
    fn test_priority_ord_emergency_greater_than_routine() {
        assert!(Priority::Emergency > Priority::Routine);
        assert!(Priority::Emergency > Priority::Urgent);
        assert!(Priority::Emergency > Priority::TwoWeekWait);
        assert!(Priority::TwoWeekWait > Priority::Urgent);
        assert!(Priority::Urgent > Priority::Routine);
    }

    #[test]
    fn test_priority_sort_ascending() {
        let mut v = vec![
            Priority::Emergency,
            Priority::Routine,
            Priority::TwoWeekWait,
            Priority::Urgent,
        ];
        v.sort();
        assert_eq!(
            v,
            vec![
                Priority::Routine,
                Priority::Urgent,
                Priority::TwoWeekWait,
                Priority::Emergency,
            ]
        );
    }

    #[test]
    fn test_days_waiting_computes_correctly() {
        let mut e = WaitlistEntry::new(Uuid::new_v4(), "ent", Priority::Routine);
        // Pin created_at so the result is deterministic.
        let start = Utc::now();
        e.created_at = start;
        let now = start + Duration::days(10);
        assert_eq!(e.days_waiting(now), 10);

        // Less than a full day should round down to 0.
        let now_partial = start + Duration::hours(23);
        assert_eq!(e.days_waiting(now_partial), 0);
    }

    #[test]
    fn test_priority_serde_roundtrip() {
        for p in [
            Priority::Routine,
            Priority::Urgent,
            Priority::TwoWeekWait,
            Priority::Emergency,
        ] {
            let json = serde_json::to_string(&p).expect("serialize");
            let back: Priority = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, p);
        }
        assert_eq!(
            serde_json::to_string(&Priority::TwoWeekWait).unwrap(),
            "\"two_week_wait\""
        );
    }

    #[test]
    fn test_waitlist_status_serde_roundtrip() {
        for s in [
            WaitlistStatus::Waiting,
            WaitlistStatus::Booked,
            WaitlistStatus::Removed,
            WaitlistStatus::Treated,
        ] {
            let json = serde_json::to_string(&s).expect("serialize");
            let back: WaitlistStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, s);
        }
    }

    #[test]
    fn test_referral_serde_roundtrip() {
        let r = Referral::new(Uuid::new_v4(), "cardiology");
        let json = serde_json::to_string(&r).expect("serialize");
        let back: Referral = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, r.id);
        assert_eq!(back.patient_id, r.patient_id);
        assert_eq!(back.target_service, "cardiology");
    }

    #[test]
    fn test_waitlist_entry_serde_roundtrip() {
        let e = WaitlistEntry::new(Uuid::new_v4(), "ent", Priority::Urgent);
        let json = serde_json::to_string(&e).expect("serialize");
        let back: WaitlistEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, e.id);
        assert_eq!(back.patient_id, e.patient_id);
        assert_eq!(back.priority, Priority::Urgent);
        assert_eq!(back.status, WaitlistStatus::Waiting);
    }
}
