//! schedule

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, Result};

/// The owner of a schedule — what resource the schedule is for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "id")]
pub enum ScheduleOwner {
    Practitioner(Uuid),
    Bed(Uuid),
    Room(Uuid),
}

/// A schedule for a practitioner, bed, or room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: Uuid,
    pub owner: ScheduleOwner,
    pub service_type: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Schedule {
    pub fn new(owner: ScheduleOwner, service_type: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            owner,
            service_type,
            active: true,
            created_at: now,
            updated_at: now,
        }
    }
}

/// The status of a slot in a schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotStatus {
    Free,
    Busy,
    BlockedOut,
}

impl SlotStatus {
    /// Attempt to transition from `self` to `next`. Returns `Err` if the
    /// transition is not allowed (including same-state transitions).
    pub fn try_transition_to(self, next: SlotStatus) -> Result<SlotStatus> {
        if self == next {
            return Err(Error::invalid_transition(format!(
                "SlotStatus already {:?}",
                self
            )));
        }
        match (self, next) {
            (SlotStatus::Free, SlotStatus::Busy)
            | (SlotStatus::Free, SlotStatus::BlockedOut)
            | (SlotStatus::Busy, SlotStatus::Free)
            | (SlotStatus::BlockedOut, SlotStatus::Free) => Ok(next),
            _ => Err(Error::invalid_transition(format!(
                "Cannot transition SlotStatus from {:?} to {:?}",
                self, next
            ))),
        }
    }
}

/// A bookable slot within a schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub start_datetime: DateTime<Utc>,
    pub end_datetime: DateTime<Utc>,
    pub status: SlotStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Slot {
    pub fn new(schedule_id: Uuid, start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            schedule_id,
            start_datetime: start,
            end_datetime: end,
            status: SlotStatus::Free,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn duration(&self) -> Duration {
        self.end_datetime - self.start_datetime
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_schedule_new_defaults() {
        let owner = ScheduleOwner::Practitioner(Uuid::new_v4());
        let s = Schedule::new(owner.clone(), "general".to_string());
        assert_eq!(s.owner, owner);
        assert_eq!(s.service_type, "general");
        assert!(s.active);
        assert_eq!(s.created_at, s.updated_at);
    }

    #[test]
    fn test_slot_new_status_free() {
        let schedule_id = Uuid::new_v4();
        let start = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap();
        let slot = Slot::new(schedule_id, start, end);
        assert_eq!(slot.schedule_id, schedule_id);
        assert_eq!(slot.start_datetime, start);
        assert_eq!(slot.end_datetime, end);
        assert_eq!(slot.status, SlotStatus::Free);
        assert_eq!(slot.created_at, slot.updated_at);
    }

    #[test]
    fn test_slot_duration() {
        let schedule_id = Uuid::new_v4();
        let start = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 5, 20, 10, 30, 0).unwrap();
        let slot = Slot::new(schedule_id, start, end);
        assert_eq!(slot.duration(), Duration::minutes(90));
    }

    #[test]
    fn test_slot_status_transition_free_to_busy_ok() {
        let result = SlotStatus::Free.try_transition_to(SlotStatus::Busy);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), SlotStatus::Busy);
    }

    #[test]
    fn test_slot_status_transition_same_state_fails() {
        assert!(
            SlotStatus::Free
                .try_transition_to(SlotStatus::Free)
                .is_err()
        );
        assert!(
            SlotStatus::Busy
                .try_transition_to(SlotStatus::Busy)
                .is_err()
        );
        assert!(
            SlotStatus::BlockedOut
                .try_transition_to(SlotStatus::BlockedOut)
                .is_err()
        );
    }

    #[test]
    fn test_slot_status_transition_free_to_blocked_out_ok() {
        let result = SlotStatus::Free.try_transition_to(SlotStatus::BlockedOut);
        assert!(result.is_ok());
    }

    #[test]
    fn test_slot_status_transition_busy_to_free_ok() {
        let result = SlotStatus::Busy.try_transition_to(SlotStatus::Free);
        assert!(result.is_ok());
    }

    #[test]
    fn test_slot_status_transition_blocked_out_to_free_ok() {
        let result = SlotStatus::BlockedOut.try_transition_to(SlotStatus::Free);
        assert!(result.is_ok());
    }

    #[test]
    fn test_slot_status_transition_busy_to_blocked_out_fails() {
        assert!(
            SlotStatus::Busy
                .try_transition_to(SlotStatus::BlockedOut)
                .is_err()
        );
    }

    #[test]
    fn test_schedule_owner_serde_roundtrip_practitioner() {
        let id = Uuid::new_v4();
        let owner = ScheduleOwner::Practitioner(id);
        let json = serde_json::to_string(&owner).expect("serialize");
        let back: ScheduleOwner = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, owner);
        assert!(json.contains("Practitioner"));
        assert!(json.contains("kind"));
    }

    #[test]
    fn test_schedule_owner_serde_roundtrip_bed() {
        let id = Uuid::new_v4();
        let owner = ScheduleOwner::Bed(id);
        let json = serde_json::to_string(&owner).expect("serialize");
        let back: ScheduleOwner = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, owner);
        assert!(json.contains("Bed"));
    }

    #[test]
    fn test_schedule_owner_serde_roundtrip_room() {
        let id = Uuid::new_v4();
        let owner = ScheduleOwner::Room(id);
        let json = serde_json::to_string(&owner).expect("serialize");
        let back: ScheduleOwner = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, owner);
        assert!(json.contains("Room"));
    }
}
