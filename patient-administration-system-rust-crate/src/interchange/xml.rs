//! XML import/export for [`PatientRow`].
//!
//! Wire shape:
//!
//! ```xml
//! <patients>
//!   <patient>
//!     <id>11111111-1111-4111-8111-111111111111</id>
//!     <mrn>MRN-001</mrn>
//!     <family_name>Doe</family_name>
//!     <given_names>Jane</given_names>
//!     <gender>female</gender>
//!     <birth_date>1990-01-15</birth_date>
//!     <phone>+1-555-0100</phone>
//!     <email>jane@example.com</email>
//!     <line1>123 Elm</line1>
//!     <city>Springfield</city>
//!     <postal_code>62701</postal_code>
//!     <country>US</country>
//!     <active>true</active>
//!   </patient>
//!   …
//! </patients>
//! ```

use serde::{Deserialize, Serialize};

use super::PatientRow;
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Patients {
    #[serde(rename = "patient", default)]
    patient: Vec<PatientRow>,
}

/// Serialize a slice of patient rows to a `<patients>…</patients>` XML
/// document.
pub fn patients_to_xml(rows: &[PatientRow]) -> Result<String> {
    let wrap = Patients {
        patient: rows.to_vec(),
    };
    quick_xml::se::to_string_with_root("patients", &wrap)
        .map(|body| format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{body}"))
        .map_err(|e| Error::internal(format!("XML serialize: {e}")))
}

/// Parse a `<patients>…</patients>` XML document.
pub fn patients_from_xml(s: &str) -> Result<Vec<PatientRow>> {
    let wrap: Patients =
        quick_xml::de::from_str(s).map_err(|e| Error::validation(format!("XML parse: {e}")))?;
    Ok(wrap.patient)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<PatientRow> {
        let mut a = PatientRow::empty();
        a.id = "11111111-1111-4111-8111-111111111111".into();
        a.mrn = "MRN-001".into();
        a.family_name = "Doe".into();
        a.given_names = "Jane".into();
        a.gender = "female".into();
        a.birth_date = "1990-01-15".into();
        let mut b = PatientRow::empty();
        b.id = "22222222-2222-4222-8222-222222222222".into();
        b.mrn = "MRN-002".into();
        b.family_name = "Smith".into();
        b.given_names = "John".into();
        b.gender = "male".into();
        vec![a, b]
    }

    #[test]
    fn test_xml_roundtrip() {
        let original = rows();
        let xml = patients_to_xml(&original).expect("serialize");
        assert!(xml.contains("<patients>"));
        assert!(xml.contains("<patient>"));
        assert!(xml.contains("<family_name>Doe</family_name>"));
        let back = patients_from_xml(&xml).expect("parse");
        assert_eq!(back, original);
    }

    #[test]
    fn test_xml_empty_list_roundtrip() {
        let xml = patients_to_xml(&[]).expect("serialize");
        let back = patients_from_xml(&xml).expect("parse");
        assert!(back.is_empty());
    }

    #[test]
    fn test_xml_parses_minimal_document() {
        let doc = r#"<patients>
            <patient>
                <id></id>
                <mrn>X</mrn>
                <family_name>Solo</family_name>
                <given_names></given_names>
                <gender>unknown</gender>
                <birth_date></birth_date>
                <phone></phone>
                <email></email>
                <line1></line1>
                <city></city>
                <postal_code></postal_code>
                <country></country>
                <active>true</active>
            </patient>
        </patients>"#;
        let back = patients_from_xml(doc).expect("parse");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].family_name, "Solo");
        assert!(back[0].active);
    }

    #[test]
    fn test_xml_rejects_garbage() {
        let err = patients_from_xml("not-xml-at-all").expect_err("must reject");
        assert!(matches!(err, Error::Validation(_)));
    }
}
