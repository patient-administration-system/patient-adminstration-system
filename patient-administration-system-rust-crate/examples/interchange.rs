//! Round-trip three patients through every v0.2 interchange format and
//! assert no rows are dropped along the way.
//!
//! Run with:
//!
//! ```bash
//! cargo run --example interchange
//! ```

use patient_administration_system::interchange::{
    PatientRow,
    csv::{patients_from_csv, patients_to_csv},
    json::{patients_from_json, patients_to_json_pretty},
    tsv::{patients_from_tsv, patients_to_tsv},
    xml::{patients_from_xml, patients_to_xml},
};

fn rows() -> Vec<PatientRow> {
    let mut a = PatientRow::empty();
    a.id = "11111111-1111-4111-8111-111111111111".into();
    a.mrn = "MRN-001".into();
    a.family_name = "Doe".into();
    a.given_names = "Jane Marie".into();
    a.gender = "female".into();
    a.birth_date = "1990-01-15".into();
    a.phone = "+1-555-0100".into();
    a.email = "jane.doe@example.com".into();
    a.line1 = "123 Elm Street".into();
    a.city = "Springfield".into();
    a.postal_code = "62701".into();
    a.country = "US".into();

    let mut b = PatientRow::empty();
    b.id = "22222222-2222-4222-8222-222222222222".into();
    b.mrn = "MRN-002".into();
    b.family_name = "Smith".into();
    b.given_names = "John".into();
    b.gender = "male".into();
    b.birth_date = "1985-07-22".into();
    b.phone = "+1-555-0200".into();
    b.email = "john.smith@example.com".into();
    b.line1 = "456 Oak Avenue".into();
    b.city = "Springfield".into();
    b.postal_code = "62702".into();
    b.country = "US".into();

    let mut c = PatientRow::empty();
    c.id = "33333333-3333-4333-8333-333333333333".into();
    c.mrn = "MRN-003".into();
    c.family_name = "O'Brien".into();
    c.given_names = "Mary Elizabeth".into();
    c.gender = "female".into();
    c.birth_date = "1972-11-03".into();
    c.phone = "+1-555-0300".into();
    c.email = "mary.obrien@example.com".into();
    c.line1 = "789 Pine Road, Apt 4".into();
    c.city = "Boston".into();
    c.postal_code = "02108".into();
    c.country = "US".into();

    vec![a, b, c]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let original = rows();
    println!("patients: {}", original.len());

    let json = patients_to_json_pretty(&original)?;
    let xml = patients_to_xml(&original)?;
    let tsv = patients_to_tsv(&original)?;
    let csv = patients_to_csv(&original)?;

    println!("  json bytes: {:>5}", json.len());
    println!("  xml bytes:  {:>5}", xml.len());
    println!("  tsv bytes:  {:>5}", tsv.len());
    println!("  csv bytes:  {:>5}", csv.len());

    let from_json = patients_from_json(&json)?;
    let from_xml = patients_from_xml(&xml)?;
    let from_tsv = patients_from_tsv(&tsv)?;
    let from_csv = patients_from_csv(&csv)?;

    assert_eq!(from_json, original, "JSON round-trip mismatch");
    assert_eq!(from_xml, original, "XML round-trip mismatch");
    assert_eq!(from_tsv, original, "TSV round-trip mismatch");
    assert_eq!(from_csv, original, "CSV round-trip mismatch");

    println!(
        "roundtrip ok: every format reparses to the same {} PatientRows",
        original.len()
    );
    Ok(())
}
