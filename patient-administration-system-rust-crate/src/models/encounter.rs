//! encounter model
//!
//! An `Encounter` is the administrative record of a patient's interaction
//! with the healthcare system: an outpatient visit, an inpatient stay, an
//! emergency-department attendance, a virtual consultation, etc.
//!
//! The PAS owns the *who/when/where/what-kind* of the encounter. Clinical
//! content (diagnoses, orders, results) lives in downstream systems and is
//! intentionally out of scope.
//!
//! The `EncounterStatus` enum is a state machine. Use
//! [`EncounterStatus::try_transition_to`] to validate moves between states;
//! invalid transitions return [`crate::Error::InvalidStateTransition`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The administrative classification of an encounter.
///
/// FHIR-aligned but kept narrow for v0.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterClass {
    /// Ambulatory clinic visit; patient is not admitted.
    Outpatient,
    /// Patient is formally admitted to a bed.
    Inpatient,
    /// Emergency department attendance.
    Emergency,
    /// Same-day admission and discharge (planned).
    DayCase,
    /// Care delivered in the patient's home.
    HomeCare,
    /// Telehealth/video/phone consultation.
    Virtual,
}

/// Lifecycle status of an encounter.
///
/// The valid transitions are:
///
/// - `Planned`    -> `Arrived` | `Cancelled`
/// - `Arrived`    -> `InProgress` | `Cancelled`
/// - `InProgress` -> `OnLeave` | `Finished` | `Cancelled`
/// - `OnLeave`    -> `InProgress` | `Finished` | `Cancelled`
/// - `Finished`   -> (terminal)
/// - `Cancelled`  -> (terminal)
///
/// A same-state transition (e.g. `Arrived -> Arrived`) is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterStatus {
    /// Encounter is scheduled but the patient has not yet arrived.
    Planned,
    /// Patient has arrived; care has not yet begun.
    Arrived,
    /// Care is actively being delivered.
    InProgress,
    /// Patient is temporarily away (e.g. day leave during inpatient stay).
    OnLeave,
    /// Encounter has concluded normally.
    Finished,
    /// Encounter was cancelled before completion.
    Cancelled,
}

impl EncounterStatus {
    /// Validate a transition from `self` to `next`.
    ///
    /// Returns the new status on success, or
    /// [`crate::Error::InvalidStateTransition`] if the move is not allowed.
    /// Same-state transitions are rejected.
    pub fn try_transition_to(self, next: EncounterStatus) -> crate::Result<EncounterStatus> {
        use EncounterStatus::*;
        let ok = matches!(
            (self, next),
            (Planned, Arrived)
                | (Planned, Cancelled)
                | (Arrived, InProgress)
                | (Arrived, Cancelled)
                | (InProgress, OnLeave)
                | (InProgress, Finished)
                | (InProgress, Cancelled)
                | (OnLeave, InProgress)
                | (OnLeave, Finished)
                | (OnLeave, Cancelled)
        );
        if ok {
            Ok(next)
        } else {
            Err(crate::Error::invalid_transition(format!(
                "Encounter: {self:?} -> {next:?}"
            )))
        }
    }
}

/// A patient's administrative interaction with the healthcare system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Encounter {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub class: EncounterClass,
    pub status: EncounterStatus,
    pub period_start: DateTime<Utc>,
    pub period_end: Option<DateTime<Utc>>,
    pub practitioner_id: Option<Uuid>,
    pub department_id: Option<Uuid>,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Encounter {
    /// Build a new encounter in the `Planned` state for the given patient
    /// and class. `period_start` and the timestamps are set to `Utc::now()`.
    pub fn new(patient_id: Uuid, class: EncounterClass) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            patient_id,
            class,
            status: EncounterStatus::Planned,
            period_start: now,
            period_end: None,
            practitioner_id: None,
            department_id: None,
            reason: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encounter_new_defaults() {
        let patient_id = Uuid::new_v4();
        let e = Encounter::new(patient_id, EncounterClass::Inpatient);
        assert_eq!(e.patient_id, patient_id);
        assert_eq!(e.class, EncounterClass::Inpatient);
        assert_eq!(e.status, EncounterStatus::Planned);
        assert!(e.period_end.is_none());
        assert!(e.practitioner_id.is_none());
        assert!(e.department_id.is_none());
        assert!(e.reason.is_none());
        assert_eq!(e.created_at, e.updated_at);
        assert_eq!(e.period_start, e.created_at);
    }

    #[test]
    fn test_transition_planned_to_arrived_ok() {
        let next = EncounterStatus::Planned
            .try_transition_to(EncounterStatus::Arrived)
            .expect("planned -> arrived is valid");
        assert_eq!(next, EncounterStatus::Arrived);
    }

    #[test]
    fn test_transition_planned_to_finished_fails() {
        let err = EncounterStatus::Planned
            .try_transition_to(EncounterStatus::Finished)
            .expect_err("planned -> finished is invalid");
        assert!(matches!(err, crate::Error::InvalidStateTransition(_)));
    }

    #[test]
    fn test_transition_in_progress_to_finished_ok() {
        let next = EncounterStatus::InProgress
            .try_transition_to(EncounterStatus::Finished)
            .expect("in_progress -> finished is valid");
        assert_eq!(next, EncounterStatus::Finished);
    }

    #[test]
    fn test_transition_finished_to_anything_fails() {
        for target in [
            EncounterStatus::Planned,
            EncounterStatus::Arrived,
            EncounterStatus::InProgress,
            EncounterStatus::OnLeave,
            EncounterStatus::Finished,
            EncounterStatus::Cancelled,
        ] {
            assert!(
                EncounterStatus::Finished.try_transition_to(target).is_err(),
                "Finished -> {target:?} should be invalid"
            );
        }
    }

    #[test]
    fn test_transition_cancelled_is_terminal() {
        for target in [
            EncounterStatus::Planned,
            EncounterStatus::Arrived,
            EncounterStatus::InProgress,
            EncounterStatus::OnLeave,
            EncounterStatus::Finished,
            EncounterStatus::Cancelled,
        ] {
            assert!(
                EncounterStatus::Cancelled
                    .try_transition_to(target)
                    .is_err(),
                "Cancelled -> {target:?} should be invalid"
            );
        }
    }

    #[test]
    fn test_transition_same_state_is_invalid() {
        for s in [
            EncounterStatus::Planned,
            EncounterStatus::Arrived,
            EncounterStatus::InProgress,
            EncounterStatus::OnLeave,
            EncounterStatus::Finished,
            EncounterStatus::Cancelled,
        ] {
            assert!(
                s.try_transition_to(s).is_err(),
                "{s:?} -> {s:?} should be invalid"
            );
        }
    }

    #[test]
    fn test_transition_on_leave_round_trip() {
        let to_leave = EncounterStatus::InProgress
            .try_transition_to(EncounterStatus::OnLeave)
            .expect("in_progress -> on_leave");
        assert_eq!(to_leave, EncounterStatus::OnLeave);
        let back = EncounterStatus::OnLeave
            .try_transition_to(EncounterStatus::InProgress)
            .expect("on_leave -> in_progress");
        assert_eq!(back, EncounterStatus::InProgress);
    }

    #[test]
    fn test_encounter_class_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&EncounterClass::DayCase).unwrap(),
            "\"day_case\""
        );
        assert_eq!(
            serde_json::to_string(&EncounterClass::HomeCare).unwrap(),
            "\"home_care\""
        );
        assert_eq!(
            serde_json::to_string(&EncounterClass::Outpatient).unwrap(),
            "\"outpatient\""
        );
    }

    #[test]
    fn test_encounter_status_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&EncounterStatus::InProgress).unwrap(),
            "\"in_progress\""
        );
        assert_eq!(
            serde_json::to_string(&EncounterStatus::OnLeave).unwrap(),
            "\"on_leave\""
        );
    }

    #[test]
    fn test_encounter_serde_roundtrip() {
        let mut e = Encounter::new(Uuid::new_v4(), EncounterClass::Emergency);
        e.reason = Some("chest pain".into());
        e.practitioner_id = Some(Uuid::new_v4());
        e.department_id = Some(Uuid::new_v4());
        let json = serde_json::to_string(&e).expect("serialize");
        let back: Encounter = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, e.id);
        assert_eq!(back.patient_id, e.patient_id);
        assert_eq!(back.class, EncounterClass::Emergency);
        assert_eq!(back.status, EncounterStatus::Planned);
        assert_eq!(back.reason.as_deref(), Some("chest pain"));
        assert_eq!(back.practitioner_id, e.practitioner_id);
        assert_eq!(back.department_id, e.department_id);
    }
}
