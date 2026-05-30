//! patient

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::identifier::Identifier;
use super::{Address, ContactPoint, Gender, NameUse};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanName {
    pub use_type: Option<NameUse>,
    pub family: String,
    pub given: Vec<String>,
    pub prefix: Vec<String>,
    pub suffix: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergencyContact {
    pub name: String,
    pub relationship: String,
    pub telecom: Vec<ContactPoint>,
    pub address: Option<Address>,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patient {
    pub id: Uuid,
    pub mpi_id: Option<Uuid>,
    pub identifiers: Vec<Identifier>,
    pub active: bool,
    pub name: HumanName,
    pub additional_names: Vec<HumanName>,
    pub telecom: Vec<ContactPoint>,
    pub gender: Gender,
    pub birth_date: Option<NaiveDate>,
    pub addresses: Vec<Address>,
    pub deceased: bool,
    pub deceased_datetime: Option<DateTime<Utc>>,
    pub emergency_contacts: Vec<EmergencyContact>,
    pub marital_status: Option<String>,
    /// When `Some(survivor_id)`, this row is a merge tombstone — the
    /// patient's identity has been merged into the row whose `id` is
    /// `survivor_id`. Tombstones never auto-redirect; clients see the
    /// link and choose whether to follow. `active` is set to `false`
    /// at the time of merge so default lists exclude them.
    #[serde(default)]
    pub replaced_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Patient {
    pub fn new(name: HumanName, gender: Gender) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            mpi_id: None,
            identifiers: Vec::new(),
            active: true,
            name,
            additional_names: Vec::new(),
            telecom: Vec::new(),
            gender,
            birth_date: None,
            addresses: Vec::new(),
            deceased: false,
            deceased_datetime: None,
            emergency_contacts: Vec::new(),
            marital_status: None,
            replaced_by: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn full_name(&self) -> String {
        let given = self.name.given.join(" ");
        format!("{} {}", given, self.name.family)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_name() -> HumanName {
        HumanName {
            use_type: Some(NameUse::Official),
            family: "Doe".into(),
            given: vec!["Jane".into()],
            prefix: vec![],
            suffix: vec![],
        }
    }

    #[test]
    fn test_patient_new_defaults() {
        let p = Patient::new(sample_name(), Gender::Female);
        assert!(p.active);
        assert!(!p.deceased);
        assert!(p.deceased_datetime.is_none());
        assert!(p.mpi_id.is_none());
        assert_eq!(p.gender, Gender::Female);
        assert!(p.identifiers.is_empty());
        assert!(p.additional_names.is_empty());
        assert!(p.telecom.is_empty());
        assert!(p.addresses.is_empty());
        assert!(p.emergency_contacts.is_empty());
        assert!(p.birth_date.is_none());
        assert!(p.marital_status.is_none());
        assert_eq!(p.created_at, p.updated_at);
    }

    #[test]
    fn test_full_name_format() {
        let name = HumanName {
            use_type: None,
            family: "Garcia".into(),
            given: vec!["Maria".into(), "Elena".into()],
            prefix: vec![],
            suffix: vec![],
        };
        let p = Patient::new(name, Gender::Female);
        assert_eq!(p.full_name(), "Maria Elena Garcia");
    }

    #[test]
    fn test_patient_serde_roundtrip() {
        let mut p = Patient::new(sample_name(), Gender::Female);
        p.birth_date = Some(NaiveDate::from_ymd_opt(1990, 1, 15).unwrap());
        p.marital_status = Some("single".into());
        let json = serde_json::to_string(&p).expect("serialize");
        let back: Patient = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, p.id);
        assert_eq!(back.name.family, "Doe");
        assert_eq!(back.gender, Gender::Female);
        assert_eq!(back.birth_date, p.birth_date);
        assert_eq!(back.marital_status.as_deref(), Some("single"));
    }
}
