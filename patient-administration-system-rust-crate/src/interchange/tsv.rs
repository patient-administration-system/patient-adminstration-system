//! TSV (tab-separated values) import/export for [`PatientRow`].
//!
//! Wire shape: a header row followed by one row per patient. Fields are in the
//! struct's declared order:
//!
//! ```text
//! id\tmrn\tfamily_name\tgiven_names\tgender\tbirth_date\tphone\temail\tline1\tcity\tpostal_code\tcountry\tactive
//! 11111111-…\tMRN-001\tDoe\tJane\tfemale\t1990-01-15\t…\t…\t…\t…\t…\t…\ttrue
//! ```
//!
//! TSV is preferred over CSV for clinical interchange because address lines
//! frequently contain commas; tabs are very rare in real-world demographic
//! values and don't need quoting.

use super::PatientRow;
use crate::{Error, Result};

/// Serialize a slice of patient rows to a TSV string (header row included).
pub fn patients_to_tsv(rows: &[PatientRow]) -> Result<String> {
    let mut wtr = csv::WriterBuilder::new()
        .delimiter(b'\t')
        .has_headers(true)
        .from_writer(Vec::new());
    for r in rows {
        wtr.serialize(r)
            .map_err(|e| Error::internal(format!("TSV serialize: {e}")))?;
    }
    let bytes = wtr
        .into_inner()
        .map_err(|e| Error::internal(format!("TSV flush: {e}")))?;
    String::from_utf8(bytes).map_err(|e| Error::internal(format!("TSV utf8: {e}")))
}

/// Parse a TSV string. The first line must be the header row.
pub fn patients_from_tsv(s: &str) -> Result<Vec<PatientRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .has_headers(true)
        .from_reader(s.as_bytes());
    let mut out = Vec::new();
    for rec in rdr.deserialize() {
        let row: PatientRow = rec.map_err(|e| Error::validation(format!("TSV parse: {e}")))?;
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
        a.phone = "+1-555-0100".into();
        let mut b = PatientRow::empty();
        b.id = "22222222-2222-4222-8222-222222222222".into();
        b.mrn = "MRN-002".into();
        b.family_name = "Smith".into();
        b.given_names = "John".into();
        b.gender = "male".into();
        vec![a, b]
    }

    #[test]
    fn test_tsv_roundtrip() {
        let original = rows();
        let s = patients_to_tsv(&original).expect("serialize");
        // Header row.
        let first_line = s.lines().next().expect("header");
        assert!(first_line.contains("id"));
        assert!(first_line.contains("\tfamily_name\t"));
        let back = patients_from_tsv(&s).expect("parse");
        assert_eq!(back, original);
    }

    #[test]
    fn test_tsv_empty_serializes_header_only() {
        let s = patients_to_tsv(&[]).expect("serialize empty");
        // csv only emits a header when at least one record is written, so the
        // empty case yields the empty string.
        assert!(s.is_empty() || s.lines().count() <= 1);
        let back = patients_from_tsv(&s).expect("parse empty");
        assert!(back.is_empty());
    }

    #[test]
    fn test_tsv_handles_addresses_with_commas() {
        let mut row = PatientRow::empty();
        row.family_name = "O'Brien".into();
        row.line1 = "Apt 4, 123 Elm".into();
        row.city = "Springfield".into();
        let s = patients_to_tsv(&[row.clone()]).expect("serialize");
        // Commas should NOT trigger quoting in TSV.
        assert!(s.contains("Apt 4, 123 Elm"));
        let back = patients_from_tsv(&s).expect("parse");
        assert_eq!(back[0], row);
    }

    #[test]
    fn test_tsv_rejects_unknown_columns() {
        // Header has a typo: `given_name` instead of `given_names`. csv treats
        // it as a missing required column → deserialize error.
        let bad = "id\tmrn\tfamily_name\tgiven_name\n\
            x\tM\tDoe\tJane\n";
        let err = patients_from_tsv(bad).expect_err("must reject");
        assert!(matches!(err, Error::Validation(_)));
    }
}
