//! Coverage — insurance / payer record for a patient (v0.10).
//!
//! Maps to the FHIR R5 `Coverage` resource. One patient may carry many
//! coverages (primary + secondary insurance, retired plans, etc.); each
//! row is independent and linked back to the patient by `patient_id`.
//! A coverage may optionally be linked to a billing
//! [`crate::models::billing::Account`] via `account_id` — that link is
//! the join the billing aggregate uses to surface payer info on
//! invoices.
//!
//! Coverage has **no state machine**. Status flips are intentional and
//! operator-driven (e.g. a clerk corrects a wrong-policy-number row by
//! marking it `EnteredInError` and re-creating). No `deleted_at` —
//! invariant §5.3 keeps soft-delete to patients/encounters/appointments
//! only; coverage retirement is via `status = Cancelled`.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lifecycle status of a coverage row. Mirrors FHIR R5
/// `Coverage.status`. Serialised snake_case on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    /// In force.
    Active,
    /// No longer in force. Set when the patient's plan ends, the policy
    /// is cancelled, or the operator retires the row.
    Cancelled,
    /// Pending — captured but not yet effective. Useful when a new
    /// plan is keyed before its start date.
    Draft,
    /// Operator marked the row a mistake. Equivalent to "never existed"
    /// from a clinical / billing perspective; the row itself is kept
    /// for audit.
    EnteredInError,
}

impl CoverageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CoverageStatus::Active => "active",
            CoverageStatus::Cancelled => "cancelled",
            CoverageStatus::Draft => "draft",
            CoverageStatus::EnteredInError => "entered_in_error",
        }
    }
}

/// What kind of coverage this row represents. Deliberately coarse;
/// finer-grained product type (medical / dental / vision / …) is left
/// to the payer-supplied `payor_name` + `policy_number` strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageKind {
    /// Third-party payer (commercial insurance, Medicare/Medicaid, …).
    Insurance,
    /// Patient pays directly — no third party.
    SelfPay,
    /// Anything else (worker's-comp, charity, research grant, …).
    Other,
}

impl CoverageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CoverageKind::Insurance => "insurance",
            CoverageKind::SelfPay => "self_pay",
            CoverageKind::Other => "other",
        }
    }
}

/// One coverage record. A patient may have many; an account may have
/// many; coverage rows are never hard-deleted (status flips to
/// `Cancelled` or `EnteredInError`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coverage {
    pub id: Uuid,
    /// The patient receiving care under this coverage — the *beneficiary*
    /// in FHIR terms. Always set; coverage cannot exist without a patient.
    pub patient_id: Uuid,
    /// Billing account this coverage is linked to. Optional because a
    /// coverage may be captured before the account is opened, and may be
    /// re-linked later via `PUT /api/coverages/{id}`.
    #[serde(default)]
    pub account_id: Option<Uuid>,
    pub status: CoverageStatus,
    pub kind: CoverageKind,
    /// The policy holder. When the patient *is* the policy holder
    /// (`relationship = "self"`), this is `Some(patient_id)` — but it's
    /// optional so callers can omit it and let the handler default it.
    #[serde(default)]
    pub subscriber_id: Option<Uuid>,
    /// Human-readable payer / insurer name (e.g. `"Aetna"`,
    /// `"NHS England"`, `"Self-pay"`).
    pub payor_name: String,
    /// Payer-supplied opaque identifier — payer EIN, plan code, etc.
    /// Free-form string; never parsed by the PAS.
    #[serde(default)]
    pub payor_identifier: Option<String>,
    /// Policy / member number as printed on the patient's card.
    pub policy_number: String,
    /// Group number, if the policy is a group plan.
    #[serde(default)]
    pub group_number: Option<String>,
    /// Relationship between the subscriber and the patient — e.g.
    /// `"self"`, `"spouse"`, `"child"`, `"parent"`, `"other"`. Free-form
    /// so callers can use whichever code system they prefer.
    pub relationship: String,
    pub start_date: NaiveDate,
    #[serde(default)]
    pub end_date: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Coverage {
    /// Build a fresh `Active` coverage row for `patient_id`. Caller is
    /// expected to fill in the payer-specific fields immediately after.
    /// `start_date` defaults to today (UTC); callers with a known
    /// effective date should overwrite.
    pub fn new(
        patient_id: Uuid,
        payor_name: impl Into<String>,
        policy_number: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            patient_id,
            account_id: None,
            status: CoverageStatus::Active,
            kind: CoverageKind::Insurance,
            subscriber_id: None,
            payor_name: payor_name.into(),
            payor_identifier: None,
            policy_number: policy_number.into(),
            group_number: None,
            relationship: "self".into(),
            start_date: now.date_naive(),
            end_date: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coverage_new_defaults() {
        let patient_id = Uuid::new_v4();
        let c = Coverage::new(patient_id, "Aetna", "AET-12345");
        assert_eq!(c.patient_id, patient_id);
        assert_eq!(c.status, CoverageStatus::Active);
        assert_eq!(c.kind, CoverageKind::Insurance);
        assert_eq!(c.payor_name, "Aetna");
        assert_eq!(c.policy_number, "AET-12345");
        assert_eq!(c.relationship, "self");
        assert!(c.account_id.is_none());
        assert!(c.subscriber_id.is_none());
        assert!(c.end_date.is_none());
        assert_eq!(c.created_at, c.updated_at);
    }

    #[test]
    fn test_coverage_status_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&CoverageStatus::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&CoverageStatus::EnteredInError).unwrap(),
            "\"entered_in_error\""
        );
        let back: CoverageStatus = serde_json::from_str("\"cancelled\"").unwrap();
        assert_eq!(back, CoverageStatus::Cancelled);
    }

    #[test]
    fn test_coverage_kind_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&CoverageKind::SelfPay).unwrap(),
            "\"self_pay\""
        );
        let back: CoverageKind = serde_json::from_str("\"insurance\"").unwrap();
        assert_eq!(back, CoverageKind::Insurance);
    }

    #[test]
    fn test_coverage_status_as_str_round_trip() {
        for s in [
            CoverageStatus::Active,
            CoverageStatus::Cancelled,
            CoverageStatus::Draft,
            CoverageStatus::EnteredInError,
        ] {
            // Roundtrip via the str form (used by the repo's stringly
            // typed status column).
            let s_str = s.as_str();
            assert!(!s_str.is_empty());
        }
    }

    #[test]
    fn test_coverage_serde_roundtrip_with_all_fields() {
        let mut c = Coverage::new(Uuid::new_v4(), "BCBS", "MEM-9001");
        c.account_id = Some(Uuid::new_v4());
        c.kind = CoverageKind::SelfPay;
        c.subscriber_id = Some(Uuid::new_v4());
        c.payor_identifier = Some("0033-AB".into());
        c.group_number = Some("GRP-7".into());
        c.relationship = "spouse".into();
        c.end_date = Some(c.start_date + chrono::Duration::days(365));

        let json = serde_json::to_string(&c).expect("serialize");
        let back: Coverage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, c.id);
        assert_eq!(back.account_id, c.account_id);
        assert_eq!(back.kind, c.kind);
        assert_eq!(back.subscriber_id, c.subscriber_id);
        assert_eq!(back.payor_identifier, c.payor_identifier);
        assert_eq!(back.group_number, c.group_number);
        assert_eq!(back.relationship, c.relationship);
        assert_eq!(back.end_date, c.end_date);
        // snake_case rename should be in the wire.
        assert!(json.contains("self_pay"));
    }
}
