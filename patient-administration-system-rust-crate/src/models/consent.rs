//! Consent records for GDPR/HIPAA compliance.
//!
//! A [`Consent`] captures one patient's authorization for a particular
//! [`ConsentType`]. Consents move through a small lifecycle ([`ConsentStatus`])
//! and may carry optional `expiry_date` and `revoked_date` markers.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Category of consent granted by a patient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentType {
    /// Consent for general data processing.
    DataProcessing,
    /// Consent for sharing data with third parties.
    DataSharing,
    /// Consent for marketing communications.
    Marketing,
    /// Consent for research use of data.
    Research,
    /// Consent for emergency access to data.
    EmergencyAccess,
}

/// Lifecycle state of a [`Consent`] record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentStatus {
    /// Consent is active and may be relied upon.
    Active,
    /// Consent was explicitly revoked by the patient.
    Revoked,
    /// Consent has reached its `expiry_date`.
    Expired,
}

/// A consent record granted by a patient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consent {
    /// Internal consent identifier.
    pub id: Uuid,
    /// Patient who granted (or revoked) the consent.
    pub patient_id: Uuid,
    /// Category of consent.
    pub consent_type: ConsentType,
    /// Current lifecycle state.
    pub status: ConsentStatus,
    /// Date the consent was granted.
    pub granted_date: NaiveDate,
    /// Date the consent expires, if applicable.
    pub expiry_date: Option<NaiveDate>,
    /// Date the consent was revoked, if applicable.
    pub revoked_date: Option<NaiveDate>,
    /// Free-text description of the purpose for which consent applies.
    pub purpose: Option<String>,
    /// How consent was obtained (e.g., "written", "electronic", "verbal").
    pub method: Option<String>,
    /// Row creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

impl Consent {
    /// Create a new active consent record granted on `granted_date`.
    pub fn new(patient_id: Uuid, consent_type: ConsentType, granted_date: NaiveDate) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            patient_id,
            consent_type,
            status: ConsentStatus::Active,
            granted_date,
            expiry_date: None,
            revoked_date: None,
            purpose: None,
            method: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Whether this consent is in force on `today`.
    ///
    /// Returns `false` if any of the following hold:
    /// - the status is not [`ConsentStatus::Active`];
    /// - the `granted_date` is in the future relative to `today`;
    /// - an `expiry_date` is set and is strictly before `today`.
    pub fn is_active(&self, today: NaiveDate) -> bool {
        if self.status != ConsentStatus::Active {
            return false;
        }
        if self.granted_date > today {
            return false;
        }
        if self.expiry_date.is_some_and(|d| d < today) {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn test_consent_new_defaults() {
        let patient_id = Uuid::new_v4();
        let granted = date(2026, 1, 1);
        let c = Consent::new(patient_id, ConsentType::DataProcessing, granted);
        assert_eq!(c.patient_id, patient_id);
        assert_eq!(c.consent_type, ConsentType::DataProcessing);
        assert_eq!(c.status, ConsentStatus::Active);
        assert_eq!(c.granted_date, granted);
        assert!(c.expiry_date.is_none());
        assert!(c.revoked_date.is_none());
        assert!(c.purpose.is_none());
        assert!(c.method.is_none());
        assert_eq!(c.created_at, c.updated_at);
    }

    #[test]
    fn test_is_active_true_when_active_and_no_expiry() {
        let c = Consent::new(
            Uuid::new_v4(),
            ConsentType::DataProcessing,
            date(2026, 1, 1),
        );
        assert!(c.is_active(date(2026, 5, 20)));
    }

    #[test]
    fn test_is_active_false_when_revoked() {
        let mut c = Consent::new(Uuid::new_v4(), ConsentType::Marketing, date(2026, 1, 1));
        c.status = ConsentStatus::Revoked;
        assert!(!c.is_active(date(2026, 5, 20)));
    }

    #[test]
    fn test_is_active_false_when_expired_status() {
        let mut c = Consent::new(Uuid::new_v4(), ConsentType::Research, date(2026, 1, 1));
        c.status = ConsentStatus::Expired;
        assert!(!c.is_active(date(2026, 5, 20)));
    }

    #[test]
    fn test_is_active_false_when_expiry_date_in_past() {
        let mut c = Consent::new(Uuid::new_v4(), ConsentType::DataSharing, date(2026, 1, 1));
        c.expiry_date = Some(date(2026, 3, 1));
        assert!(!c.is_active(date(2026, 5, 20)));
    }

    #[test]
    fn test_is_active_true_when_expiry_date_is_today() {
        // The rule is `expiry_date < today` -> false. On the expiry day itself,
        // the consent is still active.
        let mut c = Consent::new(Uuid::new_v4(), ConsentType::DataSharing, date(2026, 1, 1));
        c.expiry_date = Some(date(2026, 5, 20));
        assert!(c.is_active(date(2026, 5, 20)));
    }

    #[test]
    fn test_is_active_false_when_granted_date_in_future() {
        let c = Consent::new(
            Uuid::new_v4(),
            ConsentType::DataProcessing,
            date(2026, 12, 1),
        );
        assert!(!c.is_active(date(2026, 5, 20)));
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut c = Consent::new(
            Uuid::new_v4(),
            ConsentType::EmergencyAccess,
            date(2026, 1, 1),
        );
        c.expiry_date = Some(date(2027, 1, 1));
        c.purpose = Some("ED treatment".to_string());
        c.method = Some("electronic".to_string());

        let json = serde_json::to_string(&c).expect("serialize");
        let back: Consent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, c.id);
        assert_eq!(back.patient_id, c.patient_id);
        assert_eq!(back.consent_type, c.consent_type);
        assert_eq!(back.status, c.status);
        assert_eq!(back.granted_date, c.granted_date);
        assert_eq!(back.expiry_date, c.expiry_date);
        assert_eq!(back.purpose, c.purpose);
        assert_eq!(back.method, c.method);
        // Verify snake_case rendering for enums.
        assert!(json.contains("\"consent_type\":\"emergency_access\""));
        assert!(json.contains("\"status\":\"active\""));
    }
}
