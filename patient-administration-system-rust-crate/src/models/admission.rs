//! admission model
//!
//! ADT (admission / discharge / transfer) records associated with an
//! inpatient [`crate::models::encounter::Encounter`]. These structs are
//! pure data; the transactional state machine that orchestrates them lives
//! in `src/adt/`.
//!
//! - [`Admission`] records that a patient was placed in a bed under an
//!   encounter.
//! - [`Transfer`] records a single bed-to-bed move within one admission.
//! - [`Discharge`] records the end of an admission.
//! - [`BedAssignment`] tracks the current/historical bed occupation for an
//!   encounter and is the row queried for "who is in which bed right now".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A patient's placement into a bed at the start of an inpatient stay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Admission {
    pub id: Uuid,
    pub encounter_id: Uuid,
    pub bed_id: Uuid,
    pub admitting_practitioner_id: Option<Uuid>,
    pub admitted_at: DateTime<Utc>,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Admission {
    /// Build a new admission for the given encounter and bed. The
    /// `admitted_at` timestamp and bookkeeping fields are set to
    /// `Utc::now()`. The admitting practitioner and reason are unset.
    pub fn new(encounter_id: Uuid, bed_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            encounter_id,
            bed_id,
            admitting_practitioner_id: None,
            admitted_at: now,
            reason: None,
            created_at: now,
            updated_at: now,
        }
    }
}

/// A bed-to-bed move within a single admission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    pub id: Uuid,
    pub admission_id: Uuid,
    pub from_bed_id: Uuid,
    pub to_bed_id: Uuid,
    pub reason: Option<String>,
    pub transferred_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl Transfer {
    /// Build a new transfer record. `transferred_at` and `created_at` are
    /// set to `Utc::now()`; reason is unset.
    pub fn new(admission_id: Uuid, from_bed_id: Uuid, to_bed_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            admission_id,
            from_bed_id,
            to_bed_id,
            reason: None,
            transferred_at: now,
            created_at: now,
        }
    }
}

/// The end of an admission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discharge {
    pub id: Uuid,
    pub admission_id: Uuid,
    pub discharging_practitioner_id: Option<Uuid>,
    pub discharged_at: DateTime<Utc>,
    pub disposition: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Discharge {
    /// Build a new discharge record. `discharged_at` and `created_at` are
    /// set to `Utc::now()`; practitioner, disposition, and notes are unset.
    pub fn new(admission_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            admission_id,
            discharging_practitioner_id: None,
            discharged_at: now,
            disposition: None,
            notes: None,
            created_at: now,
        }
    }
}

/// An occupancy interval for a bed under an encounter.
///
/// A `BedAssignment` with `released_at = None` is the currently active
/// assignment. Transfers close the old assignment and open a new one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedAssignment {
    pub id: Uuid,
    pub encounter_id: Uuid,
    pub bed_id: Uuid,
    pub assigned_at: DateTime<Utc>,
    pub released_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl BedAssignment {
    /// Build a new active bed assignment. `assigned_at` is `Utc::now()`
    /// and `released_at` is `None`.
    pub fn new(encounter_id: Uuid, bed_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            encounter_id,
            bed_id,
            assigned_at: now,
            released_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Mark the assignment as released. Sets `released_at` and
    /// `updated_at` to `Utc::now()`. Idempotency is not enforced at the
    /// model layer; callers (the ADT service) are responsible for not
    /// double-releasing.
    pub fn release(&mut self) {
        let now = Utc::now();
        self.released_at = Some(now);
        self.updated_at = now;
    }

    /// True if this assignment has not yet been released.
    pub fn is_active(&self) -> bool {
        self.released_at.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admission_new_fields() {
        let encounter_id = Uuid::new_v4();
        let bed_id = Uuid::new_v4();
        let a = Admission::new(encounter_id, bed_id);
        assert_eq!(a.encounter_id, encounter_id);
        assert_eq!(a.bed_id, bed_id);
        assert!(a.admitting_practitioner_id.is_none());
        assert!(a.reason.is_none());
        assert_eq!(a.created_at, a.updated_at);
        assert_eq!(a.admitted_at, a.created_at);
    }

    #[test]
    fn test_transfer_new_fields() {
        let admission_id = Uuid::new_v4();
        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        let t = Transfer::new(admission_id, from, to);
        assert_eq!(t.admission_id, admission_id);
        assert_eq!(t.from_bed_id, from);
        assert_eq!(t.to_bed_id, to);
        assert!(t.reason.is_none());
        assert_eq!(t.transferred_at, t.created_at);
    }

    #[test]
    fn test_discharge_new_fields() {
        let admission_id = Uuid::new_v4();
        let d = Discharge::new(admission_id);
        assert_eq!(d.admission_id, admission_id);
        assert!(d.discharging_practitioner_id.is_none());
        assert!(d.disposition.is_none());
        assert!(d.notes.is_none());
        assert_eq!(d.discharged_at, d.created_at);
    }

    #[test]
    fn test_bed_assignment_is_active_before_release() {
        let ba = BedAssignment::new(Uuid::new_v4(), Uuid::new_v4());
        assert!(ba.is_active());
        assert!(ba.released_at.is_none());
        assert_eq!(ba.created_at, ba.updated_at);
        assert_eq!(ba.assigned_at, ba.created_at);
    }

    #[test]
    fn test_bed_assignment_is_active_after_release() {
        let mut ba = BedAssignment::new(Uuid::new_v4(), Uuid::new_v4());
        let original_assigned_at = ba.assigned_at;
        ba.release();
        assert!(!ba.is_active());
        assert!(ba.released_at.is_some());
        // `assigned_at` is preserved across release.
        assert_eq!(ba.assigned_at, original_assigned_at);
        // `updated_at` advances to the release time.
        assert_eq!(ba.updated_at, ba.released_at.unwrap());
    }

    #[test]
    fn test_admission_serde_roundtrip() {
        let mut a = Admission::new(Uuid::new_v4(), Uuid::new_v4());
        a.admitting_practitioner_id = Some(Uuid::new_v4());
        a.reason = Some("planned surgery".into());
        let json = serde_json::to_string(&a).expect("serialize");
        let back: Admission = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, a.id);
        assert_eq!(back.encounter_id, a.encounter_id);
        assert_eq!(back.bed_id, a.bed_id);
        assert_eq!(back.admitting_practitioner_id, a.admitting_practitioner_id);
        assert_eq!(back.reason.as_deref(), Some("planned surgery"));
        assert_eq!(back.admitted_at, a.admitted_at);
    }

    #[test]
    fn test_bed_assignment_serde_roundtrip() {
        let mut ba = BedAssignment::new(Uuid::new_v4(), Uuid::new_v4());
        ba.release();
        let json = serde_json::to_string(&ba).expect("serialize");
        let back: BedAssignment = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, ba.id);
        assert_eq!(back.encounter_id, ba.encounter_id);
        assert_eq!(back.bed_id, ba.bed_id);
        assert_eq!(back.assigned_at, ba.assigned_at);
        assert_eq!(back.released_at, ba.released_at);
        assert!(!back.is_active());
    }
}
