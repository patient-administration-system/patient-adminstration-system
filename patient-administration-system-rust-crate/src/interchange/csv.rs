//! CSV (comma-separated values) import/export for [`PatientRow`].
//!
//! Wire shape: identical to [`super::tsv`] but with `,` as the delimiter and
//! standard CSV quoting rules (RFC 4180-style). Use this when the consumer
//! cannot accept TSV. Prefer TSV when free-text fields may contain commas —
//! see the module docs on [`super::tsv`] for why.

use super::PatientRow;
use crate::{Error, Result};

/// Serialize a slice of patient rows to a CSV string (header row included).
pub fn patients_to_csv(rows: &[PatientRow]) -> Result<String> {
    let mut wtr = csv::WriterBuilder::new()
        .delimiter(b',')
        .has_headers(true)
        .from_writer(Vec::new());
    for r in rows {
        wtr.serialize(r)
            .map_err(|e| Error::internal(format!("CSV serialize: {e}")))?;
    }
    let bytes = wtr
        .into_inner()
        .map_err(|e| Error::internal(format!("CSV flush: {e}")))?;
    String::from_utf8(bytes).map_err(|e| Error::internal(format!("CSV utf8: {e}")))
}

/// Parse a CSV string. The first line must be the header row.
pub fn patients_from_csv(s: &str) -> Result<Vec<PatientRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b',')
        .has_headers(true)
        .from_reader(s.as_bytes());
    let mut out = Vec::new();
    for rec in rdr.deserialize() {
        let row: PatientRow = rec.map_err(|e| Error::validation(format!("CSV parse: {e}")))?;
        out.push(row);
    }
    Ok(out)
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
    fn test_csv_roundtrip() {
        let original = rows();
        let s = patients_to_csv(&original).expect("serialize");
        let first_line = s.lines().next().expect("header");
        assert!(first_line.contains(",family_name,"));
        let back = patients_from_csv(&s).expect("parse");
        assert_eq!(back, original);
    }

    #[test]
    fn test_csv_quotes_commas_in_address_lines() {
        let mut row = PatientRow::empty();
        row.family_name = "OBrien".into();
        row.line1 = "Apt 4, 123 Elm".into();
        row.city = "Springfield".into();
        let s = patients_to_csv(&[row.clone()]).expect("serialize");
        // CSV must quote the field because it contains a comma.
        assert!(s.contains("\"Apt 4, 123 Elm\""));
        let back = patients_from_csv(&s).expect("parse");
        assert_eq!(back[0], row);
    }

    #[test]
    fn test_csv_empty_input_parses_to_empty_vec() {
        let back = patients_from_csv("").expect("parse empty");
        assert!(back.is_empty());
    }

    #[test]
    fn test_csv_rejects_unknown_columns() {
        let bad = "id,mrn,family_name,given_name\nx,M,Doe,Jane\n";
        let err = patients_from_csv(bad).expect_err("must reject");
        assert!(matches!(err, Error::Validation(_)));
    }
}
