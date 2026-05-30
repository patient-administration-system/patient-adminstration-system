//! facility resource hierarchy: Facility → Ward → Room → Bed
//!
//! Plus the `BedStatus` state machine governing valid bed transitions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::Address;

/// Bed status enum. Drives the bed-lifecycle state machine.
///
/// Valid transitions:
/// - Available → Occupied | Reserved | OutOfService
/// - Occupied → Cleaning | OutOfService
/// - Cleaning → Available | OutOfService
/// - Reserved → Occupied | Available | OutOfService
/// - OutOfService → Available
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BedStatus {
    Available,
    Occupied,
    Reserved,
    OutOfService,
    Cleaning,
}

impl BedStatus {
    /// Attempt a transition from `self` to `next`.
    ///
    /// Returns `Ok(next)` if the transition is allowed by the bed-status
    /// state machine, otherwise returns `Err(Error::InvalidStateTransition)`.
    ///
    /// Same-state transitions (e.g. `Available -> Available`) are rejected
    /// as invalid: status changes are state-machine events, not no-ops.
    pub fn try_transition_to(self, next: BedStatus) -> crate::Result<BedStatus> {
        use BedStatus::*;
        let allowed = matches!(
            (self, next),
            (Available, Occupied)
                | (Available, Reserved)
                | (Available, OutOfService)
                | (Occupied, Cleaning)
                | (Occupied, OutOfService)
                | (Cleaning, Available)
                | (Cleaning, OutOfService)
                | (Reserved, Occupied)
                | (Reserved, Available)
                | (Reserved, OutOfService)
                | (OutOfService, Available)
        );
        if allowed {
            Ok(next)
        } else {
            Err(crate::Error::invalid_transition(format!(
                "Bed: {self:?} -> {next:?}"
            )))
        }
    }
}

/// A physical hospital facility (campus, building).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Facility {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub address: Address,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Facility {
    pub fn new(name: String, code: String, address: Address) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            code,
            address,
            active: true,
            created_at: now,
            updated_at: now,
        }
    }
}

/// A ward within a facility (e.g. "Cardiology", "Maternity").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ward {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub name: String,
    pub code: String,
    pub capacity: u32,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Ward {
    pub fn new(facility_id: Uuid, name: String, code: String, capacity: u32) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            facility_id,
            name,
            code,
            capacity,
            active: true,
            created_at: now,
            updated_at: now,
        }
    }
}

/// A room within a ward.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: Uuid,
    pub ward_id: Uuid,
    pub name: String,
    pub code: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Room {
    pub fn new(ward_id: Uuid, name: String, code: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            ward_id,
            name,
            code,
            active: true,
            created_at: now,
            updated_at: now,
        }
    }
}

/// A bed within a room. Carries a `BedStatus` driven by the state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bed {
    pub id: Uuid,
    pub room_id: Uuid,
    pub name: String,
    pub code: String,
    pub status: BedStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Bed {
    pub fn new(room_id: Uuid, name: String, code: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            room_id,
            name,
            code,
            status: BedStatus::Available,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Address, AddressUse};

    fn sample_address() -> Address {
        Address {
            use_type: Some(AddressUse::Work),
            line1: Some("123 Hospital Way".into()),
            line2: None,
            city: Some("Springfield".into()),
            state: Some("IL".into()),
            postal_code: Some("62701".into()),
            country: Some("US".into()),
        }
    }

    #[test]
    fn test_facility_new_defaults() {
        let f = Facility::new("General".into(), "GEN".into(), sample_address());
        assert!(f.active);
        assert_eq!(f.name, "General");
        assert_eq!(f.code, "GEN");
        assert_eq!(f.created_at, f.updated_at);
    }

    #[test]
    fn test_ward_new_defaults() {
        let w = Ward::new(Uuid::new_v4(), "Cardiology".into(), "CARD".into(), 24);
        assert!(w.active);
        assert_eq!(w.capacity, 24);
        assert_eq!(w.name, "Cardiology");
        assert_eq!(w.code, "CARD");
        assert_eq!(w.created_at, w.updated_at);
    }

    #[test]
    fn test_room_new_defaults() {
        let r = Room::new(Uuid::new_v4(), "Room 101".into(), "R101".into());
        assert!(r.active);
        assert_eq!(r.name, "Room 101");
        assert_eq!(r.code, "R101");
        assert_eq!(r.created_at, r.updated_at);
    }

    #[test]
    fn test_bed_new_defaults() {
        let b = Bed::new(Uuid::new_v4(), "Bed A".into(), "B-A".into());
        assert_eq!(b.status, BedStatus::Available);
        assert_eq!(b.name, "Bed A");
        assert_eq!(b.code, "B-A");
        assert_eq!(b.created_at, b.updated_at);
    }

    #[test]
    fn test_bed_status_available_to_occupied_succeeds() {
        let next = BedStatus::Available
            .try_transition_to(BedStatus::Occupied)
            .expect("Available -> Occupied should succeed");
        assert_eq!(next, BedStatus::Occupied);
    }

    #[test]
    fn test_bed_status_available_to_cleaning_fails() {
        let err = BedStatus::Available
            .try_transition_to(BedStatus::Cleaning)
            .expect_err("Available -> Cleaning should fail");
        assert!(matches!(err, crate::Error::InvalidStateTransition(_)));
    }

    #[test]
    fn test_bed_status_occupied_to_available_fails() {
        // Must go through Cleaning.
        let err = BedStatus::Occupied
            .try_transition_to(BedStatus::Available)
            .expect_err("Occupied -> Available should fail (must go via Cleaning)");
        assert!(matches!(err, crate::Error::InvalidStateTransition(_)));
    }

    #[test]
    fn test_bed_status_same_state_is_invalid() {
        let err = BedStatus::Available
            .try_transition_to(BedStatus::Available)
            .expect_err("Same state transition must be invalid");
        assert!(matches!(err, crate::Error::InvalidStateTransition(_)));
    }

    #[test]
    fn test_bed_status_full_happy_cycle() {
        // Available -> Occupied -> Cleaning -> Available
        let s = BedStatus::Available
            .try_transition_to(BedStatus::Occupied)
            .unwrap();
        let s = s.try_transition_to(BedStatus::Cleaning).unwrap();
        let s = s.try_transition_to(BedStatus::Available).unwrap();
        assert_eq!(s, BedStatus::Available);
    }

    #[test]
    fn test_bed_status_reserved_paths() {
        assert!(
            BedStatus::Available
                .try_transition_to(BedStatus::Reserved)
                .is_ok()
        );
        assert!(
            BedStatus::Reserved
                .try_transition_to(BedStatus::Occupied)
                .is_ok()
        );
        assert!(
            BedStatus::Reserved
                .try_transition_to(BedStatus::Available)
                .is_ok()
        );
        assert!(
            BedStatus::Reserved
                .try_transition_to(BedStatus::OutOfService)
                .is_ok()
        );
        // Reserved -> Cleaning is invalid.
        assert!(
            BedStatus::Reserved
                .try_transition_to(BedStatus::Cleaning)
                .is_err()
        );
    }

    #[test]
    fn test_bed_status_out_of_service_recovery() {
        assert!(
            BedStatus::OutOfService
                .try_transition_to(BedStatus::Available)
                .is_ok()
        );
        // OutOfService -> Occupied is invalid.
        assert!(
            BedStatus::OutOfService
                .try_transition_to(BedStatus::Occupied)
                .is_err()
        );
    }

    #[test]
    fn test_bed_status_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&BedStatus::OutOfService).unwrap(),
            "\"out_of_service\""
        );
        assert_eq!(
            serde_json::to_string(&BedStatus::Available).unwrap(),
            "\"available\""
        );
        let back: BedStatus = serde_json::from_str("\"out_of_service\"").unwrap();
        assert_eq!(back, BedStatus::OutOfService);
    }

    #[test]
    fn test_bed_serde_roundtrip() {
        let b = Bed::new(Uuid::new_v4(), "Bed 1".into(), "B1".into());
        let json = serde_json::to_string(&b).expect("serialize");
        let back: Bed = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, b.id);
        assert_eq!(back.room_id, b.room_id);
        assert_eq!(back.name, "Bed 1");
        assert_eq!(back.code, "B1");
        assert_eq!(back.status, BedStatus::Available);
    }

    #[test]
    fn test_ward_serde_roundtrip() {
        let w = Ward::new(Uuid::new_v4(), "Maternity".into(), "MAT".into(), 12);
        let json = serde_json::to_string(&w).expect("serialize");
        let back: Ward = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, w.id);
        assert_eq!(back.facility_id, w.facility_id);
        assert_eq!(back.name, "Maternity");
        assert_eq!(back.code, "MAT");
        assert_eq!(back.capacity, 12);
        assert!(back.active);
    }
}
