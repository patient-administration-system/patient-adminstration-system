//! Interchange formats.
//!
//! Bulk import/export of PAS patient data in three wire formats:
//!
//! - [`json`] — pretty or compact JSON arrays of [`PatientRow`].
//! - [`xml`]  — XML documents (`<patients><patient>…</patient></patients>`).
//! - [`tsv`]  — tab-separated values with a fixed header row.
//!
//! All three formats share the [`PatientRow`] projection: a flat view over a
//! [`Patient`] that picks the most commonly exchanged demographic fields. The
//! projection is intentionally lossy — additional names, emergency contacts,
//! deceased flags, and the full identifier list are not round-tripped through
//! `PatientRow`. The richer surface lives in the FHIR R5 endpoints.

use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::identifier::{Identifier, IdentifierType};
use crate::models::patient::{HumanName, Patient};
use crate::models::{
    Address, AddressUse, ContactPoint, ContactPointSystem, ContactPointUse, Gender, NameUse,
};

pub mod csv;
pub mod json;
pub mod tsv;
pub mod xml;

/// Flat projection of a [`Patient`] suitable for bulk interchange.
///
/// See module docs for the field-by-field source mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PatientRow {
    pub id: String,
    pub mrn: String,
    pub family_name: String,
    pub given_names: String,
    pub gender: String,
    pub birth_date: String,
    pub phone: String,
    pub email: String,
    pub line1: String,
    pub city: String,
    pub postal_code: String,
    pub country: String,
    pub active: bool,
}

impl PatientRow {
    /// Build an empty row with sentinel defaults. Useful as a base for tests.
    pub fn empty() -> Self {
        Self {
            id: String::new(),
            mrn: String::new(),
            family_name: String::new(),
            given_names: String::new(),
            gender: "unknown".to_string(),
            birth_date: String::new(),
            phone: String::new(),
            email: String::new(),
            line1: String::new(),
            city: String::new(),
            postal_code: String::new(),
            country: String::new(),
            active: true,
        }
    }

    /// Materialize a [`Patient`] from this row. Fields not present on a
    /// `PatientRow` are filled with sensible defaults — see the module-level
    /// docs for the (lossy) mapping.
    ///
    /// If `id` is empty or malformed, a fresh UUID is allocated. The caller
    /// is responsible for validating the result (via [`crate::validation`]).
    pub fn to_patient(&self) -> Patient {
        let id = Uuid::parse_str(&self.id).unwrap_or_else(|_| Uuid::new_v4());
        let now = Utc::now();
        let given: Vec<String> = self
            .given_names
            .split_whitespace()
            .map(String::from)
            .collect();
        let name = HumanName {
            use_type: Some(NameUse::Official),
            family: self.family_name.clone(),
            given,
            prefix: Vec::new(),
            suffix: Vec::new(),
        };
        let gender = match self.gender.to_lowercase().as_str() {
            "male" => Gender::Male,
            "female" => Gender::Female,
            "other" => Gender::Other,
            _ => Gender::Unknown,
        };
        let birth_date = if self.birth_date.is_empty() {
            None
        } else {
            NaiveDate::parse_from_str(&self.birth_date, "%Y-%m-%d").ok()
        };
        let mut identifiers = Vec::new();
        if !self.mrn.is_empty() {
            identifiers.push(Identifier::mrn("urn:oid:facility:1", &self.mrn));
        }
        let mut telecom = Vec::new();
        if !self.phone.is_empty() {
            telecom.push(ContactPoint {
                system: ContactPointSystem::Phone,
                value: self.phone.clone(),
                use_type: Some(ContactPointUse::Home),
            });
        }
        if !self.email.is_empty() {
            telecom.push(ContactPoint {
                system: ContactPointSystem::Email,
                value: self.email.clone(),
                use_type: Some(ContactPointUse::Home),
            });
        }
        let mut addresses = Vec::new();
        let has_addr = !self.line1.is_empty()
            || !self.city.is_empty()
            || !self.postal_code.is_empty()
            || !self.country.is_empty();
        if has_addr {
            addresses.push(Address {
                use_type: Some(AddressUse::Home),
                line1: opt(&self.line1),
                line2: None,
                city: opt(&self.city),
                state: None,
                postal_code: opt(&self.postal_code),
                country: opt(&self.country),
            });
        }
        Patient {
            id,
            mpi_id: None,
            identifiers,
            active: self.active,
            name,
            additional_names: Vec::new(),
            telecom,
            gender,
            birth_date,
            addresses,
            deceased: false,
            deceased_datetime: None,
            emergency_contacts: Vec::new(),
            marital_status: None,
            replaced_by: None,
            created_at: now,
            updated_at: now,
        }
    }
}

fn opt(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

impl From<&Patient> for PatientRow {
    fn from(p: &Patient) -> Self {
        let mrn = p
            .identifiers
            .iter()
            .find(|i| i.identifier_type == IdentifierType::MRN)
            .map(|i| i.value.clone())
            .unwrap_or_default();
        let given_names = p.name.given.join(" ");
        let phone = pick_telecom(&p.telecom, ContactPointSystem::Phone);
        let email = pick_telecom(&p.telecom, ContactPointSystem::Email);
        let primary_addr = p.addresses.first();
        let line1 = primary_addr
            .and_then(|a| a.line1.clone())
            .unwrap_or_default();
        let city = primary_addr
            .and_then(|a| a.city.clone())
            .unwrap_or_default();
        let postal_code = primary_addr
            .and_then(|a| a.postal_code.clone())
            .unwrap_or_default();
        let country = primary_addr
            .and_then(|a| a.country.clone())
            .unwrap_or_default();
        let gender = match p.gender {
            Gender::Male => "male",
            Gender::Female => "female",
            Gender::Other => "other",
            Gender::Unknown => "unknown",
        };
        let birth_date = p
            .birth_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        Self {
            id: p.id.to_string(),
            mrn,
            family_name: p.name.family.clone(),
            given_names,
            gender: gender.to_string(),
            birth_date,
            phone,
            email,
            line1,
            city,
            postal_code,
            country,
            active: p.active,
        }
    }
}

fn pick_telecom(tc: &[ContactPoint], system: ContactPointSystem) -> String {
    tc.iter()
        .find(|c| c.system == system)
        .map(|c| c.value.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::identifier::Identifier;

    fn sample_patient() -> Patient {
        let name = HumanName {
            use_type: Some(NameUse::Official),
            family: "Doe".into(),
            given: vec!["Jane".into(), "Marie".into()],
            prefix: vec![],
            suffix: vec![],
        };
        let mut p = Patient::new(name, Gender::Female);
        p.birth_date = Some(NaiveDate::from_ymd_opt(1990, 1, 15).unwrap());
        p.identifiers = vec![Identifier::mrn("urn:oid:facility:1", "MRN-001")];
        p.telecom = vec![
            ContactPoint {
                system: ContactPointSystem::Phone,
                value: "+1-555-0100".into(),
                use_type: Some(ContactPointUse::Home),
            },
            ContactPoint {
                system: ContactPointSystem::Email,
                value: "jane@example.com".into(),
                use_type: Some(ContactPointUse::Home),
            },
        ];
        p.addresses = vec![Address {
            use_type: Some(AddressUse::Home),
            line1: Some("123 Elm".into()),
            line2: None,
            city: Some("Springfield".into()),
            state: Some("IL".into()),
            postal_code: Some("62701".into()),
            country: Some("US".into()),
        }];
        p
    }

    #[test]
    fn test_patient_to_row_picks_primary_fields() {
        let p = sample_patient();
        let row = PatientRow::from(&p);
        assert_eq!(row.id, p.id.to_string());
        assert_eq!(row.mrn, "MRN-001");
        assert_eq!(row.family_name, "Doe");
        assert_eq!(row.given_names, "Jane Marie");
        assert_eq!(row.gender, "female");
        assert_eq!(row.birth_date, "1990-01-15");
        assert_eq!(row.phone, "+1-555-0100");
        assert_eq!(row.email, "jane@example.com");
        assert_eq!(row.line1, "123 Elm");
        assert_eq!(row.city, "Springfield");
        assert_eq!(row.postal_code, "62701");
        assert_eq!(row.country, "US");
        assert!(row.active);
    }

    #[test]
    fn test_row_to_patient_round_trip_preserves_core_fields() {
        let p = sample_patient();
        let row = PatientRow::from(&p);
        let back = row.to_patient();
        assert_eq!(back.id, p.id);
        assert_eq!(back.name.family, "Doe");
        assert_eq!(back.name.given, vec!["Jane".to_string(), "Marie".into()]);
        assert_eq!(back.gender, Gender::Female);
        assert_eq!(back.birth_date, p.birth_date);
        assert_eq!(back.telecom.len(), 2);
        assert_eq!(back.identifiers.len(), 1);
        assert_eq!(back.identifiers[0].value, "MRN-001");
        assert_eq!(back.addresses.len(), 1);
        assert_eq!(back.addresses[0].postal_code.as_deref(), Some("62701"));
    }

    #[test]
    fn test_row_to_patient_handles_empty_id() {
        let mut row = PatientRow::empty();
        row.family_name = "Smith".into();
        let back = row.to_patient();
        assert_eq!(back.name.family, "Smith");
        // A fresh UUID was allocated.
        assert_ne!(back.id, Uuid::nil());
    }

    #[test]
    fn test_row_to_patient_handles_missing_address() {
        let mut row = PatientRow::empty();
        row.family_name = "NoAddr".into();
        let back = row.to_patient();
        assert!(back.addresses.is_empty());
    }
}
