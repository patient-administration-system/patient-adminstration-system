//! JSON import/export for [`PatientRow`].
//!
//! Wire shape is a top-level JSON array, e.g.:
//!
//! ```json
//! [
//!   { "id": "…", "mrn": "MRN-001", "family_name": "Doe", … },
//!   { "id": "…", "mrn": "MRN-002", "family_name": "Smith", … }
//! ]
//! ```

use super::PatientRow;
use crate::{Error, Result};

/// Serialize a slice of patient rows to a pretty-printed JSON array.
pub fn patients_to_json_pretty(rows: &[PatientRow]) -> Result<String> {
    serde_json::to_string_pretty(rows).map_err(|e| Error::internal(format!("JSON serialize: {e}")))
}

/// Serialize a slice of patient rows to a compact JSON array.
pub fn patients_to_json_compact(rows: &[PatientRow]) -> Result<String> {
    serde_json::to_string(rows).map_err(|e| Error::internal(format!("JSON serialize: {e}")))
}

/// Parse a JSON array of patient rows.
pub fn patients_from_json(s: &str) -> Result<Vec<PatientRow>> {
    serde_json::from_str(s).map_err(|e| Error::validation(format!("JSON parse: {e}")))
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
        let mut b = PatientRow::empty();
        b.id = "22222222-2222-4222-8222-222222222222".into();
        b.mrn = "MRN-002".into();
        b.family_name = "Smith".into();
        b.given_names = "John".into();
        b.gender = "male".into();
        vec![a, b]
    }

    #[test]
    fn test_pretty_roundtrip() {
        let original = rows();
        let s = patients_to_json_pretty(&original).expect("serialize");
        assert!(s.contains("\n")); // pretty printed
        let back = patients_from_json(&s).expect("parse");
        assert_eq!(back, original);
    }

    #[test]
    fn test_compact_roundtrip() {
        let original = rows();
        let s = patients_to_json_compact(&original).expect("serialize");
        assert!(!s.contains('\n'));
        let back = patients_from_json(&s).expect("parse");
        assert_eq!(back, original);
    }

    #[test]
    fn test_parse_empty_array() {
        let back = patients_from_json("[]").expect("parse empty");
        assert!(back.is_empty());
    }

    #[test]
    fn test_parse_rejects_non_array() {
        let err = patients_from_json("{}").expect_err("must reject object");
        assert!(matches!(err, Error::Validation(_)));
    }
}
