//! identifier
//!
//! Multinational national-government healthcare identifiers, plus the
//! local Medical Record Number (`MRN`) and a small set of general-purpose
//! types (`DL`, `Passport`, `SSN`, `Other`).
//!
//! | Type       | Country          | Typed factory          | System URI                                            |
//! |------------|------------------|------------------------|-------------------------------------------------------|
//! | `MRN`      | (facility-local) | `Identifier::mrn`      | per-facility (caller-supplied)                        |
//! | `NHS`      | United Kingdom   | `Identifier::nhs`      | `https://fhir.nhs.uk/Id/nhs-number`                   |
//! | `NIR`      | France           | `Identifier::nir`      | `urn:oid:1.2.250.1.213.1.4.8`                         |
//! | `TSI`      | España           | `Identifier::tsi`      | `urn:oid:2.16.724.4.40`                               |
//! | `IHI`      | Ireland          | `Identifier::ihi`      | `https://fhir.hl7.ie/Id/individual-health-identifier` |
//! | `HCN`      | Northern Ireland | `Identifier::hcn`      | `https://fhir.hscni.net/Id/hcn`                       |
//! | `SSN`      | United States    | `Identifier::ssn`      | `http://hl7.org/fhir/sid/us-ssn`                      |
//!
//! Format-level validation of the national health-identifier values
//! (NHS Mod 11, NIR Mod 97 with Corsica fix-up, IHI / H&C digit count,
//! TSI alphanumeric envelope) lives in
//! [`crate::validation`]: `validate_nhs_number`, `validate_nir`,
//! `validate_tsi`, `validate_ihi`, `validate_hcn`, and the
//! dispatch helper `validate_identifier`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IdentifierUse {
    Usual,
    Official,
    Temp,
    Secondary,
    Old,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum IdentifierType {
    /// Medical Record Number — assigned locally by the facility.
    MRN,
    /// United Kingdom NHS Number (England, Wales, Isle of Man).
    NHS,
    /// France Numéro d'Identification au Répertoire (a.k.a. INSEE / NIR).
    NIR,
    /// España Tarjeta Sanitaria Individual / SNS CIP.
    TSI,
    /// Ireland Individual Health Identifier.
    IHI,
    /// Northern Ireland Health & Care Number.
    HCN,
    /// United States Social Security Number.
    SSN,
    /// Driver's licence.
    DL,
    /// Passport number.
    Passport,
    /// Any other identifier whose type is documented out-of-band.
    Other,
}

/// FHIR system URI for the UK NHS Number.
pub const NHS_SYSTEM_URI: &str = "https://fhir.nhs.uk/Id/nhs-number";
/// OID-form system URI for the French NIR / INSEE registry.
pub const NIR_SYSTEM_URI: &str = "urn:oid:1.2.250.1.213.1.4.8";
/// OID-form system URI for the Spanish SNS CIP / TSI register.
pub const TSI_SYSTEM_URI: &str = "urn:oid:2.16.724.4.40";
/// FHIR system URI for the Irish Individual Health Identifier.
pub const IHI_SYSTEM_URI: &str = "https://fhir.hl7.ie/Id/individual-health-identifier";
/// FHIR system URI for the Northern Ireland Health & Care Number.
pub const HCN_SYSTEM_URI: &str = "https://fhir.hscni.net/Id/hcn";
/// FHIR system URI for the US Social Security Number.
pub const SSN_SYSTEM_URI: &str = "http://hl7.org/fhir/sid/us-ssn";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identifier {
    pub use_type: Option<IdentifierUse>,
    pub identifier_type: IdentifierType,
    pub system: String,
    pub value: String,
    pub assigner: Option<String>,
}

impl Identifier {
    pub fn mrn(system: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            use_type: None,
            identifier_type: IdentifierType::MRN,
            system: system.into(),
            value: value.into(),
            assigner: None,
        }
    }

    /// United Kingdom NHS Number.
    pub fn nhs(value: impl Into<String>) -> Self {
        Self {
            use_type: None,
            identifier_type: IdentifierType::NHS,
            system: NHS_SYSTEM_URI.to_string(),
            value: value.into(),
            assigner: None,
        }
    }

    /// France Numéro d'Identification au Répertoire (NIR).
    pub fn nir(value: impl Into<String>) -> Self {
        Self {
            use_type: None,
            identifier_type: IdentifierType::NIR,
            system: NIR_SYSTEM_URI.to_string(),
            value: value.into(),
            assigner: None,
        }
    }

    /// España Tarjeta Sanitaria Individual (TSI / SNS CIP).
    pub fn tsi(value: impl Into<String>) -> Self {
        Self {
            use_type: None,
            identifier_type: IdentifierType::TSI,
            system: TSI_SYSTEM_URI.to_string(),
            value: value.into(),
            assigner: None,
        }
    }

    /// Ireland Individual Health Identifier (IHI).
    pub fn ihi(value: impl Into<String>) -> Self {
        Self {
            use_type: None,
            identifier_type: IdentifierType::IHI,
            system: IHI_SYSTEM_URI.to_string(),
            value: value.into(),
            assigner: None,
        }
    }

    /// Northern Ireland Health & Care Number (HCN).
    pub fn hcn(value: impl Into<String>) -> Self {
        Self {
            use_type: None,
            identifier_type: IdentifierType::HCN,
            system: HCN_SYSTEM_URI.to_string(),
            value: value.into(),
            assigner: None,
        }
    }

    pub fn ssn(value: impl Into<String>) -> Self {
        Self {
            use_type: None,
            identifier_type: IdentifierType::SSN,
            system: SSN_SYSTEM_URI.to_string(),
            value: value.into(),
            assigner: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mrn_factory() {
        let id = Identifier::mrn("urn:oid:facility:1", "MRN-123");
        assert_eq!(id.identifier_type, IdentifierType::MRN);
        assert_eq!(id.system, "urn:oid:facility:1");
        assert_eq!(id.value, "MRN-123");
        assert!(id.use_type.is_none());
        assert!(id.assigner.is_none());
    }

    #[test]
    fn test_nhs_factory_system_uri() {
        let id = Identifier::nhs("9434765919");
        assert_eq!(id.identifier_type, IdentifierType::NHS);
        assert_eq!(id.system, NHS_SYSTEM_URI);
        assert_eq!(id.value, "9434765919");
    }

    #[test]
    fn test_nir_factory_system_uri() {
        let id = Identifier::nir("184077501925877");
        assert_eq!(id.identifier_type, IdentifierType::NIR);
        assert_eq!(id.system, NIR_SYSTEM_URI);
        assert_eq!(id.value, "184077501925877");
    }

    #[test]
    fn test_tsi_factory_system_uri() {
        let id = Identifier::tsi("BOSEAR750515");
        assert_eq!(id.identifier_type, IdentifierType::TSI);
        assert_eq!(id.system, TSI_SYSTEM_URI);
        assert_eq!(id.value, "BOSEAR750515");
    }

    #[test]
    fn test_ihi_factory_system_uri() {
        let id = Identifier::ihi("1234567");
        assert_eq!(id.identifier_type, IdentifierType::IHI);
        assert_eq!(id.system, IHI_SYSTEM_URI);
        assert_eq!(id.value, "1234567");
    }

    #[test]
    fn test_hcn_factory_system_uri() {
        let id = Identifier::hcn("1234567890");
        assert_eq!(id.identifier_type, IdentifierType::HCN);
        assert_eq!(id.system, HCN_SYSTEM_URI);
        assert_eq!(id.value, "1234567890");
    }

    #[test]
    fn test_ssn_factory_system_uri() {
        let id = Identifier::ssn("123-45-6789");
        assert_eq!(id.identifier_type, IdentifierType::SSN);
        assert_eq!(id.system, SSN_SYSTEM_URI);
        assert_eq!(id.value, "123-45-6789");
    }

    #[test]
    fn test_identifier_serde_roundtrip() {
        let mut id = Identifier::ssn("123-45-6789");
        id.use_type = Some(IdentifierUse::Official);
        id.assigner = Some("SSA".to_string());
        let json = serde_json::to_string(&id).expect("serialize");
        let back: Identifier = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.identifier_type, IdentifierType::SSN);
        assert_eq!(back.use_type, Some(IdentifierUse::Official));
        assert_eq!(back.system, SSN_SYSTEM_URI);
        assert_eq!(back.value, "123-45-6789");
        assert_eq!(back.assigner.as_deref(), Some("SSA"));
    }

    #[test]
    fn test_identifier_type_serde_uppercase() {
        assert_eq!(
            serde_json::to_string(&IdentifierType::MRN).unwrap(),
            "\"MRN\""
        );
        assert_eq!(
            serde_json::to_string(&IdentifierType::NHS).unwrap(),
            "\"NHS\""
        );
        assert_eq!(
            serde_json::to_string(&IdentifierType::NIR).unwrap(),
            "\"NIR\""
        );
        assert_eq!(
            serde_json::to_string(&IdentifierType::TSI).unwrap(),
            "\"TSI\""
        );
        assert_eq!(
            serde_json::to_string(&IdentifierType::IHI).unwrap(),
            "\"IHI\""
        );
        assert_eq!(
            serde_json::to_string(&IdentifierType::HCN).unwrap(),
            "\"HCN\""
        );
    }
}
