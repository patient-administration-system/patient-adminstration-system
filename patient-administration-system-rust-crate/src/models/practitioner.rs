//! practitioner
//!
//! Workforce domain models: a `Practitioner` (clinician/admin staff), a
//! `Department` they may work within, and a `PractitionerRole` linking the
//! two together with role/specialty information.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::identifier::Identifier;
use super::patient::HumanName;
use super::{Address, ContactPoint, Gender};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Practitioner {
    pub id: Uuid,
    pub identifiers: Vec<Identifier>,
    pub active: bool,
    pub name: HumanName,
    pub telecom: Vec<ContactPoint>,
    pub addresses: Vec<Address>,
    pub gender: Gender,
    pub birth_date: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Practitioner {
    pub fn new(name: HumanName, gender: Gender) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            identifiers: Vec::new(),
            active: true,
            name,
            telecom: Vec::new(),
            addresses: Vec::new(),
            gender,
            birth_date: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PractitionerRole {
    pub id: Uuid,
    pub practitioner_id: Uuid,
    pub department_id: Uuid,
    pub role: String,
    pub specialty: Option<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PractitionerRole {
    pub fn new(practitioner_id: Uuid, department_id: Uuid, role: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            practitioner_id,
            department_id,
            role,
            specialty: None,
            active: true,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Department {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub name: String,
    pub code: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Department {
    pub fn new(facility_id: Uuid, name: String, code: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            facility_id,
            name,
            code,
            active: true,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NameUse;

    fn sample_name() -> HumanName {
        HumanName {
            use_type: Some(NameUse::Official),
            family: "Smith".into(),
            given: vec!["Alice".into()],
            prefix: vec!["Dr.".into()],
            suffix: vec!["MD".into()],
        }
    }

    #[test]
    fn test_practitioner_new_defaults() {
        let p = Practitioner::new(sample_name(), Gender::Female);
        assert!(p.active);
        assert_eq!(p.gender, Gender::Female);
        assert!(p.identifiers.is_empty());
        assert!(p.telecom.is_empty());
        assert!(p.addresses.is_empty());
        assert!(p.birth_date.is_none());
        assert_ne!(p.id, Uuid::nil());
        assert_eq!(p.created_at, p.updated_at);
        assert_eq!(p.name.family, "Smith");
    }

    #[test]
    fn test_department_new_defaults() {
        let facility_id = Uuid::new_v4();
        let d = Department::new(facility_id, "Cardiology".into(), "CARD".into());
        assert!(d.active);
        assert_ne!(d.id, Uuid::nil());
        assert_eq!(d.facility_id, facility_id);
        assert_eq!(d.name, "Cardiology");
        assert_eq!(d.code, "CARD");
        assert_eq!(d.created_at, d.updated_at);
    }

    #[test]
    fn test_practitioner_role_new_defaults() {
        let practitioner_id = Uuid::new_v4();
        let department_id = Uuid::new_v4();
        let r = PractitionerRole::new(practitioner_id, department_id, "Attending Physician".into());
        assert!(r.active);
        assert!(r.specialty.is_none());
        assert_ne!(r.id, Uuid::nil());
        assert_eq!(r.practitioner_id, practitioner_id);
        assert_eq!(r.department_id, department_id);
        assert_eq!(r.role, "Attending Physician");
        assert_eq!(r.created_at, r.updated_at);
    }

    #[test]
    fn test_practitioner_serde_roundtrip() {
        let mut p = Practitioner::new(sample_name(), Gender::Female);
        p.birth_date = Some(NaiveDate::from_ymd_opt(1980, 3, 14).unwrap());
        let json = serde_json::to_string(&p).expect("serialize");
        let back: Practitioner = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, p.id);
        assert_eq!(back.name.family, "Smith");
        assert_eq!(back.gender, Gender::Female);
        assert_eq!(back.birth_date, p.birth_date);
        assert!(back.active);
    }

    #[test]
    fn test_department_serde_roundtrip() {
        let facility_id = Uuid::new_v4();
        let d = Department::new(facility_id, "Radiology".into(), "RAD".into());
        let json = serde_json::to_string(&d).expect("serialize");
        let back: Department = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, d.id);
        assert_eq!(back.facility_id, facility_id);
        assert_eq!(back.name, "Radiology");
        assert_eq!(back.code, "RAD");
        assert!(back.active);
    }

    #[test]
    fn test_practitioner_role_serde_roundtrip() {
        let practitioner_id = Uuid::new_v4();
        let department_id = Uuid::new_v4();
        let mut r = PractitionerRole::new(practitioner_id, department_id, "Nurse".into());
        r.specialty = Some("Pediatrics".into());
        let json = serde_json::to_string(&r).expect("serialize");
        let back: PractitionerRole = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, r.id);
        assert_eq!(back.practitioner_id, practitioner_id);
        assert_eq!(back.department_id, department_id);
        assert_eq!(back.role, "Nurse");
        assert_eq!(back.specialty.as_deref(), Some("Pediatrics"));
        assert!(back.active);
    }
}
