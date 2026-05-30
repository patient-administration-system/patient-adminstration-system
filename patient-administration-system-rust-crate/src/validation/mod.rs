//! validation
//!
//! Cross-cutting validators for domain types. These complement the
//! constructor-level invariants in `src/models/` with rules that depend on
//! the outside world (current time) or on relationships between fields.
//!
//! Each validator returns `Err(Error::Validation(_))` for the first rule it
//! finds violated; callers should treat them as fail-fast.

use crate::models::appointment::Appointment;
use crate::models::billing::Charge;
use crate::models::identifier::{Identifier, IdentifierType};
use crate::models::patient::Patient;
use crate::models::{Address, ContactPoint, ContactPointSystem};
use crate::{Error, Result};

/// Validate a [`Patient`] aggregate.
///
/// Rules:
/// - `name.family` must be non-empty after trimming.
/// - At least one `name.given` must be non-empty after trimming.
/// - `birth_date`, when set, must not be in the future.
/// - Every contact point and address must individually validate.
pub fn validate_patient(p: &Patient) -> Result<()> {
    if p.name.family.trim().is_empty() {
        return Err(Error::validation("patient family name is required"));
    }
    if p.name.given.iter().all(|g| g.trim().is_empty()) {
        return Err(Error::validation("at least one given name is required"));
    }
    if let Some(bd) = p.birth_date
        && bd > chrono::Utc::now().date_naive()
    {
        return Err(Error::validation("birth_date cannot be in the future"));
    }
    for c in &p.telecom {
        validate_contact_point(c)?;
    }
    for a in &p.addresses {
        validate_address(a)?;
    }
    for id in &p.identifiers {
        validate_identifier(id)?;
    }
    Ok(())
}

/// Validate an [`Address`].
///
/// At least one of `city`, `postal_code`, or `country` must be populated;
/// an address with only `line1`/`line2`/`state` cannot be routed.
pub fn validate_address(a: &Address) -> Result<()> {
    if a.city.is_none() && a.postal_code.is_none() && a.country.is_none() {
        return Err(Error::validation(
            "address requires at least city, postal_code, or country",
        ));
    }
    Ok(())
}

/// Validate a [`ContactPoint`].
///
/// - Emails must contain both `@` and `.`.
/// - Phone/SMS/Fax values must have between 7 and 15 ASCII digits.
/// - All other systems pass without further checks.
pub fn validate_contact_point(c: &ContactPoint) -> Result<()> {
    match c.system {
        ContactPointSystem::Email =>
        {
            #[allow(clippy::collapsible_match)]
            if !c.value.contains('@') || !c.value.contains('.') {
                return Err(Error::validation(format!("invalid email: {}", c.value)));
            }
        }
        ContactPointSystem::Phone | ContactPointSystem::Sms | ContactPointSystem::Fax => {
            let digits = c.value.chars().filter(|x| x.is_ascii_digit()).count();
            if !(7..=15).contains(&digits) {
                return Err(Error::validation(format!(
                    "phone digits out of range: {}",
                    c.value
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Validate an [`Appointment`]'s time range: `start_datetime` must be strictly
/// before `end_datetime`. Zero-length and inverted ranges are rejected.
pub fn validate_appointment(a: &Appointment) -> Result<()> {
    if a.start_datetime >= a.end_datetime {
        return Err(Error::validation("appointment start must be before end"));
    }
    Ok(())
}

/// Validate a [`Charge`]: the monetary amount must be non-negative.
pub fn validate_charge(c: &Charge) -> Result<()> {
    if c.amount.amount.is_sign_negative() {
        return Err(Error::validation("charge amount must be non-negative"));
    }
    Ok(())
}

/// Strip ASCII whitespace and hyphens — both are conventional in
/// pretty-printed forms of NHS / IHI / H&C numbers (`"943 476 5919"`,
/// `"943-476-5919"`, etc.). Used by the national-identifier validators.
fn strip_id_format_chars(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect()
}

/// Validate a UK NHS Number.
///
/// 10 digits, last digit a Modulus 11 check on the first 9. Whitespace
/// and hyphens are stripped before validation so `"943 476 5919"` and
/// `"943-476-5919"` are accepted identically to `"9434765919"`.
///
/// The Mod 11 algorithm: multiply the first 9 digits by weights
/// `10, 9, 8, 7, 6, 5, 4, 3, 2`, sum, take `sum mod 11`. The expected
/// check digit is `11 - remainder` (with `remainder == 0` → check `0`).
/// A computed check of `10` means the candidate NHS number is invalid —
/// such numbers were never issued.
pub fn validate_nhs_number(value: &str) -> Result<()> {
    let digits = strip_id_format_chars(value);
    if digits.len() != 10 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::validation(format!(
            "NHS number must be 10 digits, got {value:?}"
        )));
    }
    let weighted_sum: u32 = digits
        .chars()
        .take(9)
        .enumerate()
        .map(|(i, c)| c.to_digit(10).unwrap() * (10 - i as u32))
        .sum();
    let remainder = weighted_sum % 11;
    let expected = if remainder == 0 { 0 } else { 11 - remainder };
    if expected == 10 {
        return Err(Error::validation(format!(
            "NHS number with check digit 10 is never issued: {value:?}"
        )));
    }
    let actual = digits.chars().nth(9).unwrap().to_digit(10).unwrap();
    if actual != expected {
        return Err(Error::validation(format!(
            "NHS number check digit mismatch: expected {expected}, got {actual} in {value:?}"
        )));
    }
    Ok(())
}

/// Validate a France Numéro d'Identification au Répertoire (NIR).
///
/// 15 characters: `sex(1) + year(2) + month(2) + department(2) +
/// commune(3) + order(3) + control(2)`. The control is
/// `97 - (body mod 97)` where `body` is the 13-character prefix
/// interpreted as a decimal integer; for Corsica the `2A`/`2B`
/// department codes are substituted with `19`/`18` respectively before
/// computing the modulus.
///
/// Whitespace is stripped before validation.
pub fn validate_nir(value: &str) -> Result<()> {
    let stripped = strip_id_format_chars(value);
    if stripped.len() != 15 {
        return Err(Error::validation(format!(
            "NIR must be 15 characters after stripping formatting, got {} in {value:?}",
            stripped.len()
        )));
    }
    let upper = stripped.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if !matches!(bytes[0], b'1' | b'2' | b'7' | b'8') {
        return Err(Error::validation(format!(
            "NIR sex digit must be 1, 2, 7, or 8: got {:?} in {value:?}",
            bytes[0] as char
        )));
    }
    if !(bytes[1].is_ascii_digit() && bytes[2].is_ascii_digit()) {
        return Err(Error::validation(format!(
            "NIR year-of-birth must be 2 digits, got {:?}",
            &upper[1..3]
        )));
    }
    let month_str = &upper[3..5];
    let month: u32 = month_str
        .parse()
        .map_err(|_| Error::validation(format!("NIR month must be numeric, got {month_str:?}")))?;
    if !(1..=12).contains(&month) && month != 20 && !(30..=99).contains(&month) {
        return Err(Error::validation(format!(
            "NIR month invalid (expected 01-12, 20, or 30-99), got {month:02}"
        )));
    }
    let dept = &upper[5..7];
    let is_corsica = dept == "2A" || dept == "2B";
    if !is_corsica && !dept.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::validation(format!(
            "NIR department must be 2 digits or 2A/2B, got {dept:?}"
        )));
    }
    if !upper[7..13].chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::validation(format!(
            "NIR commune+order must be 6 digits, got {:?}",
            &upper[7..13]
        )));
    }
    let control: u64 = upper[13..15].parse().map_err(|_| {
        Error::validation(format!(
            "NIR control key must be 2 digits, got {:?}",
            &upper[13..15]
        ))
    })?;
    let body_str = if is_corsica {
        let substituted = if dept == "2A" { "19" } else { "18" };
        format!("{}{}{}", &upper[0..5], substituted, &upper[7..13])
    } else {
        upper[0..13].to_string()
    };
    let body: u64 = body_str.parse().map_err(|_| {
        Error::validation(format!("NIR body not numeric after fix-up: {body_str:?}"))
    })?;
    let expected = 97 - (body % 97);
    if expected != control {
        return Err(Error::validation(format!(
            "NIR control key mismatch: expected {expected}, got {control} in {value:?}"
        )));
    }
    Ok(())
}

/// Validate a Spain Tarjeta Sanitaria Individual (TSI / SNS CIP).
///
/// The Spanish autonomous-community TSI codes do not share a single
/// national format — Andalucía, Cataluña, Madrid, etc. each issue their
/// own CIP and the unified SNS CIP is alphanumeric. The PAS enforces an
/// envelope only: 1 to 20 ASCII alphanumeric characters after trimming.
pub fn validate_tsi(value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 20 {
        return Err(Error::validation(format!(
            "TSI must be 1-20 characters, got length {} in {value:?}",
            trimmed.len()
        )));
    }
    if !trimmed.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Error::validation(format!(
            "TSI must be ASCII alphanumeric, got {value:?}"
        )));
    }
    Ok(())
}

/// Validate an Ireland Individual Health Identifier (IHI).
///
/// Per the HSE Individual Health Identifier Act 2014, an IHI is "a
/// randomly generated unique 7 digit number". There is no documented
/// check-digit algorithm — the PAS enforces shape only. Whitespace and
/// hyphens are stripped before validation.
pub fn validate_ihi(value: &str) -> Result<()> {
    let digits = strip_id_format_chars(value);
    if digits.len() != 7 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::validation(format!(
            "IHI must be exactly 7 digits, got {value:?}"
        )));
    }
    Ok(())
}

/// Validate a Northern Ireland Health & Care Number (HCN).
///
/// Modern HCNs are 10 digits, allocated by HSC; there is no publicly
/// documented checksum, so the PAS enforces length + digit-only.
/// Whitespace and hyphens are stripped before validation.
pub fn validate_hcn(value: &str) -> Result<()> {
    let digits = strip_id_format_chars(value);
    if digits.len() != 10 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::validation(format!(
            "Northern Ireland H&C number must be 10 digits, got {value:?}"
        )));
    }
    Ok(())
}

/// Dispatch an [`Identifier`] to the matching per-type format
/// validator. Types without a documented national format
/// (`MRN`, `SSN`, `DL`, `Passport`, `Other`) pass unconditionally.
pub fn validate_identifier(id: &Identifier) -> Result<()> {
    match id.identifier_type {
        IdentifierType::NHS => validate_nhs_number(&id.value),
        IdentifierType::NIR => validate_nir(&id.value),
        IdentifierType::TSI => validate_tsi(&id.value),
        IdentifierType::IHI => validate_ihi(&id.value),
        IdentifierType::HCN => validate_hcn(&id.value),
        IdentifierType::MRN
        | IdentifierType::SSN
        | IdentifierType::DL
        | IdentifierType::Passport
        | IdentifierType::Other => Ok(()),
    }
}

/// Normalize a free-form phone string to a digits-only form, preserving a
/// leading `+` if one was present after trimming.
///
/// Examples:
/// - `"(555) 123-4567"` → `"5551234567"`
/// - `"+1 555 123 4567"` → `"+15551234567"`
pub fn normalize_phone(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 1);
    if s.trim_start().starts_with('+') {
        out.push('+');
    }
    for c in s.chars().filter(|c| c.is_ascii_digit()) {
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::patient::HumanName;
    use crate::models::{
        Address, AddressUse, ContactPoint, ContactPointSystem, ContactPointUse, Gender, Iso4217,
        Money,
    };
    use chrono::{Duration, NaiveDate, TimeZone, Utc};
    use rust_decimal::Decimal;
    use uuid::Uuid;

    fn sample_name() -> HumanName {
        HumanName {
            use_type: None,
            family: "Doe".into(),
            given: vec!["Jane".into()],
            prefix: vec![],
            suffix: vec![],
        }
    }

    fn sample_patient() -> Patient {
        Patient::new(sample_name(), Gender::Female)
    }

    #[test]
    fn test_validate_patient_accepts_minimal_valid() {
        let p = sample_patient();
        assert!(validate_patient(&p).is_ok());
    }

    #[test]
    fn test_validate_patient_rejects_missing_family() {
        let mut p = sample_patient();
        p.name.family = "  ".into();
        assert!(validate_patient(&p).is_err());
    }

    #[test]
    fn test_validate_patient_rejects_missing_given() {
        let mut p = sample_patient();
        p.name.given = vec!["   ".into()];
        assert!(validate_patient(&p).is_err());
    }

    #[test]
    fn test_validate_patient_rejects_future_birth_date() {
        let mut p = sample_patient();
        let tomorrow = (Utc::now() + Duration::days(1)).date_naive();
        p.birth_date = Some(tomorrow);
        assert!(validate_patient(&p).is_err());
    }

    #[test]
    fn test_validate_patient_accepts_past_birth_date() {
        let mut p = sample_patient();
        p.birth_date = Some(NaiveDate::from_ymd_opt(1980, 1, 1).unwrap());
        assert!(validate_patient(&p).is_ok());
    }

    #[test]
    fn test_validate_address_rejects_empty() {
        let a = Address {
            use_type: Some(AddressUse::Home),
            line1: Some("123 Main St".into()),
            line2: None,
            city: None,
            state: Some("CA".into()),
            postal_code: None,
            country: None,
        };
        assert!(validate_address(&a).is_err());
    }

    #[test]
    fn test_validate_address_accepts_with_city() {
        let a = Address {
            use_type: None,
            line1: None,
            line2: None,
            city: Some("Anywhere".into()),
            state: None,
            postal_code: None,
            country: None,
        };
        assert!(validate_address(&a).is_ok());
    }

    #[test]
    fn test_validate_address_accepts_with_postal_code() {
        let a = Address {
            use_type: None,
            line1: None,
            line2: None,
            city: None,
            state: None,
            postal_code: Some("90210".into()),
            country: None,
        };
        assert!(validate_address(&a).is_ok());
    }

    #[test]
    fn test_validate_address_accepts_with_country() {
        let a = Address {
            use_type: None,
            line1: None,
            line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: Some("US".into()),
        };
        assert!(validate_address(&a).is_ok());
    }

    fn cp(system: ContactPointSystem, value: &str) -> ContactPoint {
        ContactPoint {
            system,
            value: value.into(),
            use_type: Some(ContactPointUse::Home),
        }
    }

    #[test]
    fn test_validate_contact_point_email_good() {
        assert!(validate_contact_point(&cp(ContactPointSystem::Email, "j@x.io")).is_ok());
    }

    #[test]
    fn test_validate_contact_point_email_missing_at() {
        assert!(validate_contact_point(&cp(ContactPointSystem::Email, "jane.x.io")).is_err());
    }

    #[test]
    fn test_validate_contact_point_email_missing_dot() {
        assert!(validate_contact_point(&cp(ContactPointSystem::Email, "jane@x")).is_err());
    }

    #[test]
    fn test_validate_contact_point_phone_good() {
        assert!(validate_contact_point(&cp(ContactPointSystem::Phone, "(555) 123-4567")).is_ok());
    }

    #[test]
    fn test_validate_contact_point_phone_too_short() {
        assert!(validate_contact_point(&cp(ContactPointSystem::Phone, "123456")).is_err());
    }

    #[test]
    fn test_validate_contact_point_phone_too_long() {
        assert!(
            validate_contact_point(&cp(ContactPointSystem::Phone, "1234567890123456")).is_err()
        );
    }

    #[test]
    fn test_validate_contact_point_sms_validated_like_phone() {
        assert!(validate_contact_point(&cp(ContactPointSystem::Sms, "5551234567")).is_ok());
        assert!(validate_contact_point(&cp(ContactPointSystem::Sms, "12")).is_err());
    }

    #[test]
    fn test_validate_appointment_ok() {
        let start = Utc.with_ymd_and_hms(2030, 1, 1, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2030, 1, 1, 10, 0, 0).unwrap();
        let a = Appointment::new(Uuid::new_v4(), start, end);
        assert!(validate_appointment(&a).is_ok());
    }

    #[test]
    fn test_validate_appointment_rejects_equal_endpoints() {
        let t = Utc.with_ymd_and_hms(2030, 1, 1, 9, 0, 0).unwrap();
        let a = Appointment::new(Uuid::new_v4(), t, t);
        assert!(validate_appointment(&a).is_err());
    }

    #[test]
    fn test_validate_appointment_rejects_inverted() {
        let start = Utc.with_ymd_and_hms(2030, 1, 1, 10, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2030, 1, 1, 9, 0, 0).unwrap();
        let a = Appointment::new(Uuid::new_v4(), start, end);
        assert!(validate_appointment(&a).is_err());
    }

    fn usd() -> Iso4217 {
        Iso4217::new("USD").unwrap()
    }

    #[test]
    fn test_validate_charge_positive_ok() {
        let amount = Money::new(Decimal::new(1234, 2), usd());
        let c = Charge::new(Uuid::new_v4(), "CODE".into(), "desc".into(), amount);
        assert!(validate_charge(&c).is_ok());
    }

    #[test]
    fn test_validate_charge_zero_ok() {
        let amount = Money::new(Decimal::ZERO, usd());
        let c = Charge::new(Uuid::new_v4(), "CODE".into(), "desc".into(), amount);
        assert!(validate_charge(&c).is_ok());
    }

    #[test]
    fn test_validate_charge_negative_rejected() {
        let amount = Money::new(Decimal::new(-100, 2), usd());
        let c = Charge::new(Uuid::new_v4(), "CODE".into(), "desc".into(), amount);
        assert!(validate_charge(&c).is_err());
    }

    #[test]
    fn test_normalize_phone_strips_punctuation() {
        assert_eq!(normalize_phone("(555) 123-4567"), "5551234567");
    }

    #[test]
    fn test_normalize_phone_preserves_plus() {
        assert_eq!(normalize_phone("+1 555 123 4567"), "+15551234567");
    }

    #[test]
    fn test_normalize_phone_plus_after_whitespace() {
        assert_eq!(normalize_phone("   +44 20 7946 0958"), "+442079460958");
    }

    #[test]
    fn test_normalize_phone_no_plus() {
        assert_eq!(normalize_phone("020-7946-0958"), "02079460958");
    }

    // ---------------------------------------------------------------
    // National healthcare-identifier validators
    // ---------------------------------------------------------------

    #[test]
    fn test_validate_nhs_number_accepts_valid() {
        assert!(validate_nhs_number("9434765919").is_ok());
        // Same value, pretty-printed forms.
        assert!(validate_nhs_number("943 476 5919").is_ok());
        assert!(validate_nhs_number("943-476-5919").is_ok());
    }

    #[test]
    fn test_validate_nhs_number_accepts_check_digit_zero() {
        // Weighted sum 330 → mod 11 == 0 → expected check digit 0.
        assert!(validate_nhs_number("9876543210").is_ok());
    }

    #[test]
    fn test_validate_nhs_number_rejects_wrong_length() {
        assert!(validate_nhs_number("123").is_err());
        assert!(validate_nhs_number("12345678901").is_err());
    }

    #[test]
    fn test_validate_nhs_number_rejects_non_digits() {
        assert!(validate_nhs_number("123456789a").is_err());
    }

    #[test]
    fn test_validate_nhs_number_rejects_bad_check_digit() {
        // Mutate the trailing check digit of a known-good number.
        assert!(validate_nhs_number("9434765918").is_err());
    }

    #[test]
    fn test_validate_nir_accepts_metropolitan() {
        // Sex=1, year=84, month=07, dept=75, commune=019, order=258,
        // control = 97 - (1840775019258 mod 97) = 97 - 20 = 77.
        assert!(validate_nir("184077501925877").is_ok());
    }

    #[test]
    fn test_validate_nir_accepts_corsica_2a() {
        // After 2A → 19 substitution: 1840719075019 mod 97 = 46,
        // control = 97 - 46 = 51.
        assert!(validate_nir("184072A07501951").is_ok());
    }

    #[test]
    fn test_validate_nir_accepts_corsica_2b() {
        // After 2B → 18 substitution: 1840718075019 mod 97 = 19,
        // control = 97 - 19 = 78.
        assert!(validate_nir("184072B07501978").is_ok());
    }

    #[test]
    fn test_validate_nir_rejects_wrong_length() {
        assert!(validate_nir("18407750192587").is_err());
        assert!(validate_nir("1840775019258777").is_err());
    }

    #[test]
    fn test_validate_nir_rejects_bad_sex_digit() {
        // Leading '3' is not a valid NIR sex digit.
        assert!(validate_nir("384077501925877").is_err());
    }

    #[test]
    fn test_validate_nir_rejects_bad_control_key() {
        // Mutate the last 2 control digits.
        assert!(validate_nir("184077501925800").is_err());
    }

    #[test]
    fn test_validate_nir_rejects_bad_department() {
        // Dept '9X' is neither digits nor Corsica 2A/2B.
        assert!(validate_nir("184079X07501977").is_err());
    }

    #[test]
    fn test_validate_tsi_accepts_alphanumeric() {
        assert!(validate_tsi("BOSEAR750515").is_ok());
        assert!(validate_tsi("ANDA01234567").is_ok());
        assert!(validate_tsi("12345678").is_ok());
    }

    #[test]
    fn test_validate_tsi_rejects_empty() {
        assert!(validate_tsi("").is_err());
        assert!(validate_tsi("   ").is_err());
    }

    #[test]
    fn test_validate_tsi_rejects_oversize() {
        assert!(validate_tsi("A".repeat(21).as_str()).is_err());
    }

    #[test]
    fn test_validate_tsi_rejects_punctuation() {
        assert!(validate_tsi("BOSE-AR-750515").is_err());
    }

    #[test]
    fn test_validate_ihi_accepts_seven_digits() {
        assert!(validate_ihi("1234567").is_ok());
        assert!(validate_ihi("123-4567").is_ok());
    }

    #[test]
    fn test_validate_ihi_rejects_wrong_length() {
        assert!(validate_ihi("123456").is_err());
        assert!(validate_ihi("12345678").is_err());
    }

    #[test]
    fn test_validate_ihi_rejects_non_digits() {
        assert!(validate_ihi("12A4567").is_err());
    }

    #[test]
    fn test_validate_hcn_accepts_ten_digits() {
        assert!(validate_hcn("1234567890").is_ok());
        assert!(validate_hcn("123 456 7890").is_ok());
    }

    #[test]
    fn test_validate_hcn_rejects_wrong_length() {
        assert!(validate_hcn("123456789").is_err());
        assert!(validate_hcn("12345678901").is_err());
    }

    #[test]
    fn test_validate_hcn_rejects_non_digits() {
        assert!(validate_hcn("123456789X").is_err());
    }

    #[test]
    fn test_validate_identifier_dispatches_by_type() {
        // National types route to their own validators.
        assert!(validate_identifier(&Identifier::nhs("9434765919")).is_ok());
        assert!(validate_identifier(&Identifier::nhs("0000000001")).is_err());
        assert!(validate_identifier(&Identifier::nir("184077501925877")).is_ok());
        assert!(validate_identifier(&Identifier::nir("000000000000000")).is_err());
        assert!(validate_identifier(&Identifier::ihi("1234567")).is_ok());
        assert!(validate_identifier(&Identifier::ihi("X")).is_err());
        assert!(validate_identifier(&Identifier::hcn("1234567890")).is_ok());
        assert!(validate_identifier(&Identifier::hcn("123")).is_err());
        assert!(validate_identifier(&Identifier::tsi("BOSEAR750515")).is_ok());
        assert!(validate_identifier(&Identifier::tsi("")).is_err());
    }

    #[test]
    fn test_validate_identifier_passes_unrestricted_types() {
        // MRN / SSN / DL / Passport / Other carry no national format
        // check — only structural sanity, which the dispatcher does
        // not enforce.
        assert!(validate_identifier(&Identifier::mrn("urn:facility", "anything")).is_ok());
        assert!(validate_identifier(&Identifier::ssn("not really an ssn")).is_ok());
    }

    #[test]
    fn test_validate_patient_rejects_invalid_identifier() {
        let mut p = sample_patient();
        // Trailing check digit deliberately wrong.
        p.identifiers.push(Identifier::nhs("9434765910"));
        assert!(validate_patient(&p).is_err());
    }

    #[test]
    fn test_validate_patient_accepts_valid_multinational_identifiers() {
        let mut p = sample_patient();
        p.identifiers.push(Identifier::nhs("9434765919"));
        p.identifiers.push(Identifier::nir("184077501925877"));
        p.identifiers.push(Identifier::tsi("BOSEAR750515"));
        p.identifiers.push(Identifier::ihi("1234567"));
        p.identifiers.push(Identifier::hcn("1234567890"));
        assert!(validate_patient(&p).is_ok());
    }
}
