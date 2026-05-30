//! appointment

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, Result};

/// The lifecycle status of an appointment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppointmentStatus {
    Proposed,
    Booked,
    Arrived,
    Fulfilled,
    Cancelled,
    NoShow,
}

impl AppointmentStatus {
    /// Attempt to transition from `self` to `next`. Returns `Err` if the
    /// transition is not allowed (including same-state transitions and
    /// transitions from terminal states).
    pub fn try_transition_to(self, next: AppointmentStatus) -> Result<AppointmentStatus> {
        if self == next {
            return Err(Error::invalid_transition(format!(
                "AppointmentStatus already {:?}",
                self
            )));
        }
        let ok = matches!(
            (self, next),
            (AppointmentStatus::Proposed, AppointmentStatus::Booked)
                | (AppointmentStatus::Proposed, AppointmentStatus::Cancelled)
                | (AppointmentStatus::Booked, AppointmentStatus::Arrived)
                | (AppointmentStatus::Booked, AppointmentStatus::Cancelled)
                | (AppointmentStatus::Booked, AppointmentStatus::NoShow)
                | (AppointmentStatus::Arrived, AppointmentStatus::Fulfilled)
                | (AppointmentStatus::Arrived, AppointmentStatus::Cancelled)
        );
        if ok {
            Ok(next)
        } else {
            Err(Error::invalid_transition(format!(
                "Cannot transition AppointmentStatus from {:?} to {:?}",
                self, next
            )))
        }
    }
}

/// Reason an appointment was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationReason {
    PatientRequest,
    ProviderRequest,
    NoShow,
    Rescheduled,
    Other,
}

/// An appointment scheduled for a patient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Appointment {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub slot_id: Option<Uuid>,
    pub practitioner_id: Option<Uuid>,
    pub start_datetime: DateTime<Utc>,
    pub end_datetime: DateTime<Utc>,
    pub status: AppointmentStatus,
    pub reason: Option<String>,
    pub from_waitlist_entry_id: Option<Uuid>,
    pub cancellation_reason: Option<CancellationReason>,
    /// Backlink to an [`crate::models::appointment_series::AppointmentSeries`]
    /// when this appointment was generated as one occurrence of a
    /// recurring series (v0.9). `None` for singleton appointments.
    #[serde(default)]
    pub series_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Appointment {
    pub fn new(
        patient_id: Uuid,
        start_datetime: DateTime<Utc>,
        end_datetime: DateTime<Utc>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            patient_id,
            slot_id: None,
            practitioner_id: None,
            start_datetime,
            end_datetime,
            status: AppointmentStatus::Proposed,
            reason: None,
            from_waitlist_entry_id: None,
            cancellation_reason: None,
            series_id: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_appointment_new_defaults() {
        let patient_id = Uuid::new_v4();
        let start = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap();
        let appt = Appointment::new(patient_id, start, end);
        assert_eq!(appt.patient_id, patient_id);
        assert!(appt.slot_id.is_none());
        assert!(appt.practitioner_id.is_none());
        assert_eq!(appt.start_datetime, start);
        assert_eq!(appt.end_datetime, end);
        assert_eq!(appt.status, AppointmentStatus::Proposed);
        assert!(appt.reason.is_none());
        assert!(appt.from_waitlist_entry_id.is_none());
        assert!(appt.cancellation_reason.is_none());
        assert_eq!(appt.created_at, appt.updated_at);
    }

    #[test]
    fn test_appointment_status_proposed_to_booked_ok() {
        let result = AppointmentStatus::Proposed.try_transition_to(AppointmentStatus::Booked);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), AppointmentStatus::Booked);
    }

    #[test]
    fn test_appointment_status_booked_to_fulfilled_fails() {
        // Must go through Arrived first.
        let result = AppointmentStatus::Booked.try_transition_to(AppointmentStatus::Fulfilled);
        assert!(result.is_err());
    }

    #[test]
    fn test_appointment_status_cancelled_is_terminal() {
        for next in [
            AppointmentStatus::Proposed,
            AppointmentStatus::Booked,
            AppointmentStatus::Arrived,
            AppointmentStatus::Fulfilled,
            AppointmentStatus::Cancelled,
            AppointmentStatus::NoShow,
        ] {
            let result = AppointmentStatus::Cancelled.try_transition_to(next);
            assert!(
                result.is_err(),
                "Cancelled should not transition to {:?}",
                next
            );
        }
    }

    #[test]
    fn test_appointment_status_fulfilled_is_terminal() {
        for next in [
            AppointmentStatus::Proposed,
            AppointmentStatus::Booked,
            AppointmentStatus::Arrived,
            AppointmentStatus::Fulfilled,
            AppointmentStatus::Cancelled,
            AppointmentStatus::NoShow,
        ] {
            let result = AppointmentStatus::Fulfilled.try_transition_to(next);
            assert!(
                result.is_err(),
                "Fulfilled should not transition to {:?}",
                next
            );
        }
    }

    #[test]
    fn test_appointment_status_no_show_is_terminal() {
        for next in [
            AppointmentStatus::Proposed,
            AppointmentStatus::Booked,
            AppointmentStatus::Arrived,
            AppointmentStatus::Fulfilled,
            AppointmentStatus::Cancelled,
            AppointmentStatus::NoShow,
        ] {
            let result = AppointmentStatus::NoShow.try_transition_to(next);
            assert!(
                result.is_err(),
                "NoShow should not transition to {:?}",
                next
            );
        }
    }

    #[test]
    fn test_appointment_status_same_state_fails() {
        assert!(
            AppointmentStatus::Proposed
                .try_transition_to(AppointmentStatus::Proposed)
                .is_err()
        );
        assert!(
            AppointmentStatus::Booked
                .try_transition_to(AppointmentStatus::Booked)
                .is_err()
        );
    }

    #[test]
    fn test_appointment_status_booked_transitions() {
        assert!(
            AppointmentStatus::Booked
                .try_transition_to(AppointmentStatus::Arrived)
                .is_ok()
        );
        assert!(
            AppointmentStatus::Booked
                .try_transition_to(AppointmentStatus::Cancelled)
                .is_ok()
        );
        assert!(
            AppointmentStatus::Booked
                .try_transition_to(AppointmentStatus::NoShow)
                .is_ok()
        );
    }

    #[test]
    fn test_appointment_status_arrived_transitions() {
        assert!(
            AppointmentStatus::Arrived
                .try_transition_to(AppointmentStatus::Fulfilled)
                .is_ok()
        );
        assert!(
            AppointmentStatus::Arrived
                .try_transition_to(AppointmentStatus::Cancelled)
                .is_ok()
        );
    }

    #[test]
    fn test_appointment_serde_roundtrip() {
        let patient_id = Uuid::new_v4();
        let start = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap();
        let mut appt = Appointment::new(patient_id, start, end);
        appt.reason = Some("Routine checkup".to_string());
        appt.cancellation_reason = Some(CancellationReason::PatientRequest);
        appt.practitioner_id = Some(Uuid::new_v4());
        appt.slot_id = Some(Uuid::new_v4());
        appt.from_waitlist_entry_id = Some(Uuid::new_v4());

        let json = serde_json::to_string(&appt).expect("serialize");
        let back: Appointment = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, appt.id);
        assert_eq!(back.patient_id, appt.patient_id);
        assert_eq!(back.slot_id, appt.slot_id);
        assert_eq!(back.practitioner_id, appt.practitioner_id);
        assert_eq!(back.status, appt.status);
        assert_eq!(back.reason, appt.reason);
        assert_eq!(back.from_waitlist_entry_id, appt.from_waitlist_entry_id);
        assert_eq!(back.cancellation_reason, appt.cancellation_reason);
        // Verify snake_case rename on CancellationReason.
        assert!(json.contains("patient_request"));
        // Verify snake_case rename on AppointmentStatus.
        assert!(json.contains("proposed"));
    }

    #[test]
    fn test_cancellation_reason_serde_all_variants() {
        for r in [
            CancellationReason::PatientRequest,
            CancellationReason::ProviderRequest,
            CancellationReason::NoShow,
            CancellationReason::Rescheduled,
            CancellationReason::Other,
        ] {
            let json = serde_json::to_string(&r).expect("serialize");
            let back: CancellationReason = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, r);
        }
        assert_eq!(
            serde_json::to_string(&CancellationReason::PatientRequest).unwrap(),
            "\"patient_request\""
        );
        assert_eq!(
            serde_json::to_string(&CancellationReason::NoShow).unwrap(),
            "\"no_show\""
        );
    }
}
