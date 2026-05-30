//! Bidirectional mapping between HL7 v2 segments and PAS domain types.
//!
//! v0.3 first cut: `PID` ↔ [`Patient`]. The encoder for ADT^A28 (add person
//! info) is exposed via [`encode_adt_a28`] for outbound use; the parser only
//! needs to peek at the message type via [`message_type`].

use chrono::{NaiveDate, Utc};
use uuid::Uuid;

use super::escape::{escape_value, unescape_value};
use super::{Message, Segment, encode_message};
use crate::models::identifier::{Identifier, IdentifierType, IdentifierUse};
use crate::models::patient::{HumanName, Patient};
use crate::models::{
    Address, AddressUse, ContactPoint, ContactPointSystem, ContactPointUse, Gender, NameUse,
};
use crate::{Error, Result};

/// Read MSH-9 as `(message_code, trigger_event)`. For `ADT^A01` returns
/// `("ADT", "A01")`. If the field is missing or malformed, the
/// trigger-event component is the empty string.
pub fn message_type(m: &Message) -> (String, String) {
    let msh = match m.segment("MSH") {
        Some(s) => s,
        None => return (String::new(), String::new()),
    };
    let code = msh.component(9, 1).to_string();
    let event = msh.component(9, 2).to_string();
    (code, event)
}

/// Build a [`Patient`] from a `PID` segment.
///
/// Maps:
/// - PID-3 (first repetition) → MRN [`Identifier`].
/// - PID-5 (first repetition) → primary [`HumanName`] (`family^given^middle^suffix^prefix`).
/// - PID-7 → birth date (`YYYYMMDD`).
/// - PID-8 → [`Gender`] (`M`/`F`/`O`/`U`).
/// - PID-11 → primary [`Address`] (`line1^line2^city^state^postal^country`).
/// - PID-13 → primary phone [`ContactPoint`].
/// - PID-14 → primary email [`ContactPoint`] (if it contains `@`).
///
/// Lossy: extra repetitions, additional name fields, marital status are
/// not surfaced. Use the FHIR R5 path when you need full fidelity.
pub fn patient_from_pid(pid: &Segment) -> Result<Patient> {
    if pid.name != "PID" {
        return Err(Error::validation(format!(
            "expected PID segment, got {:?}",
            pid.name
        )));
    }
    let family = unescape_value(pid.component(5, 1));
    if family.is_empty() {
        return Err(Error::validation("PID-5.1 (family name) must not be empty"));
    }
    let given_primary = unescape_value(pid.component(5, 2));
    let middle = unescape_value(pid.component(5, 3));
    let mut given = Vec::new();
    if !given_primary.is_empty() {
        given.push(given_primary);
    }
    if !middle.is_empty() {
        given.push(middle);
    }
    let prefix = unescape_value(pid.component(5, 5));
    let suffix = unescape_value(pid.component(5, 4));
    let name = HumanName {
        use_type: Some(NameUse::Official),
        family,
        given,
        prefix: if prefix.is_empty() {
            vec![]
        } else {
            vec![prefix]
        },
        suffix: if suffix.is_empty() {
            vec![]
        } else {
            vec![suffix]
        },
    };

    let mrn_value = unescape_value(pid.component(3, 1));
    let mrn_facility = unescape_value(pid.component(3, 4));
    let mut identifiers = Vec::new();
    if !mrn_value.is_empty() {
        let system = if mrn_facility.is_empty() {
            "urn:oid:facility:1".to_string()
        } else {
            format!("urn:oid:facility:{mrn_facility}")
        };
        identifiers.push(Identifier {
            use_type: Some(IdentifierUse::Official),
            identifier_type: IdentifierType::MRN,
            system,
            value: mrn_value,
            assigner: None,
        });
    }

    let birth_date = parse_v2_date(pid.field(7));
    let gender = match pid.field(8) {
        "M" => Gender::Male,
        "F" => Gender::Female,
        "O" => Gender::Other,
        _ => Gender::Unknown,
    };

    let mut addresses = Vec::new();
    if !pid.field(11).is_empty() {
        addresses.push(Address {
            use_type: Some(AddressUse::Home),
            line1: nonempty_unescape(pid.component(11, 1)),
            line2: nonempty_unescape(pid.component(11, 2)),
            city: nonempty_unescape(pid.component(11, 3)),
            state: nonempty_unescape(pid.component(11, 4)),
            postal_code: nonempty_unescape(pid.component(11, 5)),
            country: nonempty_unescape(pid.component(11, 6)),
        });
    }

    let mut telecom = Vec::new();
    let phone = unescape_value(pid.field(13));
    if !phone.is_empty() {
        telecom.push(ContactPoint {
            system: ContactPointSystem::Phone,
            value: phone,
            use_type: Some(ContactPointUse::Home),
        });
    }
    let email = unescape_value(pid.field(14));
    if !email.is_empty() && email.contains('@') {
        telecom.push(ContactPoint {
            system: ContactPointSystem::Email,
            value: email,
            use_type: Some(ContactPointUse::Home),
        });
    }

    // PID-29 (Patient Death Date and Time) and PID-30 (Patient Death
    // Indicator, `Y` or `N`). v0.22 added round-trip for both. The
    // indicator wins when present; otherwise the presence of PID-29
    // implies `deceased = true`.
    let deceased_datetime = {
        let s = pid.field(29);
        if s.is_empty() {
            None
        } else {
            parse_v2_datetime(s)
        }
    };
    let deceased = match pid.field(30) {
        "Y" | "y" => true,
        "N" | "n" => false,
        _ => deceased_datetime.is_some(),
    };

    let now = Utc::now();
    Ok(Patient {
        id: Uuid::new_v4(),
        mpi_id: None,
        identifiers,
        active: true,
        name,
        additional_names: Vec::new(),
        telecom,
        gender,
        birth_date,
        addresses,
        deceased,
        deceased_datetime,
        emergency_contacts: Vec::new(),
        marital_status: None,
        replaced_by: None,
        created_at: now,
        updated_at: now,
    })
}

/// Build a `PID` [`Segment`] from a [`Patient`]. The inverse of
/// [`patient_from_pid`] — see that function for the field mapping.
pub fn pid_from_patient(p: &Patient) -> Segment {
    let mrn = p
        .identifiers
        .iter()
        .find(|i| i.identifier_type == IdentifierType::MRN);
    let pid3 = match mrn {
        Some(i) => {
            let facility = i.system.strip_prefix("urn:oid:facility:").unwrap_or("FAC");
            format!("{}^^^{}^MR", escape_value(&i.value), escape_value(facility))
        }
        None => String::new(),
    };
    let middle = p.name.given.get(1).cloned().unwrap_or_default();
    let first = p.name.given.first().cloned().unwrap_or_default();
    let family = escape_value(&p.name.family);
    let first_esc = escape_value(&first);
    let middle_esc = escape_value(&middle);
    let pid5 = if first.is_empty() && middle.is_empty() {
        family
    } else if middle.is_empty() {
        format!("{}^{}", family, first_esc)
    } else {
        format!("{}^{}^{}", family, first_esc, middle_esc)
    };
    let pid7 = p.birth_date.map(format_v2_date).unwrap_or_default();
    let pid8 = match p.gender {
        Gender::Male => "M",
        Gender::Female => "F",
        Gender::Other => "O",
        Gender::Unknown => "U",
    };
    let pid11 = p
        .addresses
        .first()
        .map(|a| {
            format!(
                "{}^{}^{}^{}^{}^{}",
                escape_value(&a.line1.clone().unwrap_or_default()),
                escape_value(&a.line2.clone().unwrap_or_default()),
                escape_value(&a.city.clone().unwrap_or_default()),
                escape_value(&a.state.clone().unwrap_or_default()),
                escape_value(&a.postal_code.clone().unwrap_or_default()),
                escape_value(&a.country.clone().unwrap_or_default()),
            )
        })
        .unwrap_or_default();
    let phone = p
        .telecom
        .iter()
        .find(|c| c.system == ContactPointSystem::Phone)
        .map(|c| escape_value(&c.value))
        .unwrap_or_default();
    let email = p
        .telecom
        .iter()
        .find(|c| c.system == ContactPointSystem::Email)
        .map(|c| escape_value(&c.value))
        .unwrap_or_default();

    // PID has 30 fields in v2.5; we populate up through PID-14 always
    // and PID-29 / PID-30 (deceased datetime + indicator) when the
    // patient is marked deceased. Field index 1 = PID-1 (set ID).
    let needs_deceased = p.deceased || p.deceased_datetime.is_some();
    let len = if needs_deceased { 30 } else { 14 };
    let mut fields = vec![String::new(); len];
    fields[0] = "1".into(); // PID-1
    fields[2] = pid3; // PID-3
    fields[4] = pid5; // PID-5
    fields[6] = pid7; // PID-7
    fields[7] = pid8.into(); // PID-8
    fields[10] = pid11; // PID-11
    fields[12] = phone; // PID-13
    fields[13] = email; // PID-14
    if needs_deceased {
        // PID-29 (Death Date and Time) — `YYYYMMDDHHMMSS` when set.
        if let Some(d) = p.deceased_datetime {
            fields[28] = format_v2_datetime(d);
        }
        // PID-30 (Death Indicator) — `Y` when deceased, `N` otherwise.
        // We always emit the indicator when the patient is flagged
        // deceased so the receiver doesn't have to infer from PID-29.
        fields[29] = if p.deceased { "Y".into() } else { "N".into() };
    }

    Segment {
        name: "PID".into(),
        fields,
    }
}

/// Build a complete ADT^A28 (add person info) message for the given patient.
pub fn encode_adt_a28(
    patient: &Patient,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A28",
        patient,
        None,
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A01 (admit) message. `bed` is the bed the patient was
/// admitted to — its `code` is rendered as `PV1-3.3`.
pub fn encode_adt_a01(
    patient: &Patient,
    bed: &crate::models::facility::Bed,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A01",
        patient,
        Some(bed),
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A06 (change outpatient to inpatient) message.
/// PV1-3.3 carries the destination bed code so the receiver knows
/// where the patient was admitted — same shape as A01 admit, but
/// the trigger code signals a class change rather than a fresh
/// admit. (*v0.42*)
pub fn encode_adt_a06(
    patient: &Patient,
    bed: &crate::models::facility::Bed,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A06",
        patient,
        Some(bed),
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A23 (delete a patient record) message. PID
/// identifies the patient whose record is being deleted; no PV1.
/// The receiver is expected to soft-delete or mark the matching
/// patient by MRN (PID-3.1). (*v0.40*)
pub fn encode_adt_a23(
    patient: &Patient,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A23",
        patient,
        None,
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A21 (patient goes on leave of absence) message. PID
/// identifies the patient; no PV1 segment is emitted because the
/// receiver is expected to locate the open admission by patient
/// identity. (*v0.37*)
pub fn encode_adt_a21(
    patient: &Patient,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A21",
        patient,
        None,
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A22 (patient returns from leave of absence) message.
/// Shape mirrors A21 — PID identifies the patient, no PV1. (*v0.37*)
pub fn encode_adt_a22(
    patient: &Patient,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A22",
        patient,
        None,
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A38 (cancel pre-admit) message. PV1-3 carries the
/// bed code whose reservation is being released so the receiver can
/// locate the prior A05 it sent (or the matching record in its own
/// reservation table). (*v0.35*)
pub fn encode_adt_a38(
    patient: &Patient,
    released_bed: &crate::models::facility::Bed,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A38",
        patient,
        Some(released_bed),
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A05 (pre-admit patient) message. PV1-3 carries the
/// reserved bed code in the third PL sub-component so the receiver
/// knows which bed PAS has set aside for the future admission.
/// PV1-2 is the inpatient class (`I`) — A05 is by definition a
/// planned inpatient flow. (*v0.33*)
pub fn encode_adt_a05(
    patient: &Patient,
    reserved_bed: &crate::models::facility::Bed,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A05",
        patient,
        Some(reserved_bed),
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A04 (register patient) message. Used for outpatient and
/// emergency-department registrations — no bed allocation. PV1-2 carries
/// the patient class code: typically `"E"` for emergency, `"O"` for
/// outpatient. The caller passes the literal HL7 v2 code so callers from
/// other classes (`D` day-case, `H` home-care, etc.) can drive the same
/// helper if needed later. (*v0.29*)
pub fn encode_adt_a04(
    patient: &Patient,
    class_code: &str,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    let now = format_v2_datetime(Utc::now());
    let msh = Segment {
        name: "MSH".into(),
        fields: vec![
            "|".into(),
            "^~\\&".into(),
            sending_app.into(),
            "FAC".into(),
            receiving_app.into(),
            "FAC".into(),
            now.clone(),
            "".into(),
            "ADT^A04".into(),
            message_control_id.into(),
            "P".into(),
            "2.5".into(),
        ],
    };
    let evn = Segment {
        name: "EVN".into(),
        fields: vec!["A04".into(), now],
    };
    let pid = pid_from_patient(patient);
    let pv1 = Segment {
        name: "PV1".into(),
        fields: vec!["1".into(), class_code.into()],
    };
    encode_message(&Message {
        segments: vec![msh, evn, pid, pv1],
    })
}

/// Build an ADT^A02 (transfer) message. `new_bed` is the destination bed.
pub fn encode_adt_a02(
    patient: &Patient,
    new_bed: &crate::models::facility::Bed,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A02",
        patient,
        Some(new_bed),
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A03 (discharge) message. No PV1-3 bed component is emitted;
/// the message just signals that the patient identified by PID has been
/// discharged.
pub fn encode_adt_a03(
    patient: &Patient,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A03",
        patient,
        None,
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A08 (update patient information) message. The PID is the
/// authoritative new snapshot; no PV1 segment is emitted.
pub fn encode_adt_a08(
    patient: &Patient,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A08",
        patient,
        None,
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A12 (cancel transfer) message. PID identifies the patient
/// whose transfer is being undone; PV1-3 carries the origin bed code in
/// the PL-typed third component so the receiver knows where to put the
/// patient back. (*v0.31*)
pub fn encode_adt_a12(
    patient: &Patient,
    origin_bed: &crate::models::facility::Bed,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A12",
        patient,
        Some(origin_bed),
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A11 (cancel admit) message. PID identifies the patient
/// whose admit is being cancelled; no PV1 segment is emitted because the
/// receiver is expected to look the admission up by patient identity.
pub fn encode_adt_a11(
    patient: &Patient,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A11",
        patient,
        None,
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build an ADT^A13 (cancel discharge) message. `bed` is the original bed
/// the patient is being reinstated to — its `code` is rendered as
/// `PV1-3.3` so the downstream receiver can mirror the bed assignment.
pub fn encode_adt_a13(
    patient: &Patient,
    bed: &crate::models::facility::Bed,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_adt(
        "A13",
        patient,
        Some(bed),
        sending_app,
        receiving_app,
        message_control_id,
    )
}

// ----- DFT (Detail Financial Transaction) ---------------------------------
//
// v0.19: PAS understands DFT^P03 (post detail financial transaction) on
// both the inbound and outbound paths. One DFT^P03 message carries one
// charge against the patient identified by PID.
//
// Segment subset:
// - MSH: standard header (`DFT^P03` in MSH-9).
// - EVN: event type `P03`.
// - PID: same shape used by ADT — see `patient_from_pid` /
//   `pid_from_patient`. Dedup-on-MRN like A01 / A28 on the inbound side.
// - FT1: financial transaction. Honored fields:
//   - FT1-4 (Transaction Date): YYYYMMDDHHMMSS → `Charge.posted_at`.
//     Defaults to "now" when missing.
//   - FT1-6 (Transaction Type): only `CG` (charge) is accepted as of
//     v0.19. `PY` (payment) and `AJ` (adjustment) AE-ACK as
//     unsupported — they need different domain logic and storage.
//     Defaults to `CG` when missing.
//   - FT1-7 (Transaction Code, first component): → `Charge.code`.
//     Required.
//   - FT1-8 (Transaction Description): → `Charge.description`.
//     Required.
//   - FT1-11 (Transaction Amount Extended): a `CP` composite —
//     `<amount>^<currency>`. FT1-11.1 → `Charge.amount.amount`,
//     FT1-11.2 → `Charge.amount.currency`. Both required.
//
// On the inbound path, the PAS resolves an open billing account for
// the patient (creating one in the same currency when none exists)
// and posts the charge with the standard audit + outbox trail. The
// outbox event is `ChargePosted` with `source: "hl7v2_p03"` so the
// outbound publisher can avoid boomeranging the message back.

/// One parsed FT1 segment from a DFT^P03 message — i.e. one charge
/// against the patient identified by PID. v0.20 made `DftP03Message`
/// hold a vec of these so a single DFT can post multiple charges in
/// one transaction, which is how real-world EMR billing batches behave.
#[derive(Debug, Clone)]
pub struct DftP03Item {
    /// Transaction type from FT1-6. Always `"CG"` (charge); the parser
    /// rejects any other value with `Error::Validation`. Empty defaults
    /// to `"CG"`.
    pub transaction_type: String,
    /// Charge code from FT1-7.1.
    pub code: String,
    /// Free-text description from FT1-8.
    pub description: String,
    /// Decimal amount parsed from FT1-11.1.
    pub amount: rust_decimal::Decimal,
    /// ISO 4217 currency parsed from FT1-11.2 (uppercased).
    pub currency: String,
    /// Posted timestamp from FT1-4. `None` when missing; the handler
    /// defaults to `Utc::now()` per item.
    pub posted_at: Option<chrono::DateTime<Utc>>,
}

/// Structured view of an inbound DFT^P03 message. Constructed via
/// [`parse_dft_p03`]. Patient comes from PID; one entry in `items` per
/// FT1 segment. At least one item is guaranteed — the parser rejects
/// a DFT with no FT1 segments.
#[derive(Debug, Clone)]
pub struct DftP03Message {
    pub patient: Patient,
    pub items: Vec<DftP03Item>,
}

/// Parse a `DFT^P03` message into a [`DftP03Message`]. Walks every
/// `FT1` segment and produces one [`DftP03Item`] per segment. Returns
/// `Error::Validation` when required fields are missing or malformed
/// on any FT1, or when the transaction type is one we don't support
/// yet.
pub fn parse_dft_p03(m: &Message) -> Result<DftP03Message> {
    let (code, trigger) = message_type(m);
    if code != "DFT" || trigger != "P03" {
        return Err(Error::validation(format!(
            "expected DFT^P03, got {code}^{trigger}"
        )));
    }
    let pid = m
        .segment("PID")
        .ok_or_else(|| Error::validation("missing PID segment"))?;
    let patient = patient_from_pid(pid)?;

    let mut items: Vec<DftP03Item> = Vec::new();
    for (idx, ft1) in m.all_segments("FT1").enumerate() {
        items.push(parse_ft1(ft1, idx + 1)?);
    }
    if items.is_empty() {
        return Err(Error::validation("missing FT1 segment"));
    }
    Ok(DftP03Message { patient, items })
}

fn parse_ft1(ft1: &Segment, idx_1based: usize) -> Result<DftP03Item> {
    use std::str::FromStr;
    // FT1-6 transaction type. Accept "CG" only. Default to "CG" when
    // the field is empty so a sparse FT1 still works for the common
    // "post a charge" case.
    let raw_type = ft1.field(6);
    let transaction_type = if raw_type.is_empty() { "CG" } else { raw_type };
    if transaction_type != "CG" {
        return Err(Error::validation(format!(
            "FT1[{idx_1based}]-6 transaction type {transaction_type:?} not supported (only CG = charge as of v0.19)"
        )));
    }
    let txn_code = unescape_value(ft1.component(7, 1));
    if txn_code.is_empty() {
        return Err(Error::validation(format!(
            "FT1[{idx_1based}]-7 (transaction code) must not be empty"
        )));
    }
    let description = unescape_value(ft1.field(8));
    if description.is_empty() {
        return Err(Error::validation(format!(
            "FT1[{idx_1based}]-8 (transaction description) must not be empty"
        )));
    }
    let amt_str = unescape_value(ft1.component(11, 1));
    if amt_str.is_empty() {
        return Err(Error::validation(format!(
            "FT1[{idx_1based}]-11.1 (transaction amount) must not be empty"
        )));
    }
    let amount = rust_decimal::Decimal::from_str(&amt_str).map_err(|e| {
        Error::validation(format!(
            "FT1[{idx_1based}]-11.1 not a decimal amount: {amt_str:?} ({e})"
        ))
    })?;
    let currency_raw = unescape_value(ft1.component(11, 2));
    if currency_raw.is_empty() {
        return Err(Error::validation(format!(
            "FT1[{idx_1based}]-11.2 (currency code) must not be empty"
        )));
    }
    let currency = currency_raw.to_ascii_uppercase();
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(Error::validation(format!(
            "FT1[{idx_1based}]-11.2 not a 3-letter ISO 4217 code: {currency_raw:?}"
        )));
    }
    let posted_at = {
        let s = ft1.field(4);
        if s.is_empty() {
            None
        } else {
            parse_v2_datetime(s)
        }
    };
    Ok(DftP03Item {
        transaction_type: transaction_type.to_string(),
        code: txn_code,
        description,
        amount,
        currency,
        posted_at,
    })
}

/// Build a DFT^P03 (post detail financial transaction) message for the
/// given patient + charge. The amount and currency come from the
/// charge's `Money`; the code, description, and posted_at come from
/// the charge fields. FT1-6 is always emitted as `CG` (charge) since
/// `Charge` represents only that transaction type.
pub fn encode_dft_p03(
    patient: &Patient,
    charge: &crate::models::billing::Charge,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    let now = format_v2_datetime(Utc::now());
    let msh = Segment {
        name: "MSH".into(),
        fields: vec![
            "|".into(),
            "^~\\&".into(),
            sending_app.into(),
            "FAC".into(),
            receiving_app.into(),
            "FAC".into(),
            now.clone(),
            "".into(),
            "DFT^P03".into(),
            message_control_id.into(),
            "P".into(),
            "2.5".into(),
        ],
    };
    let evn = Segment {
        name: "EVN".into(),
        fields: vec!["P03".into(), now],
    };
    let pid = pid_from_patient(patient);
    // FT1 fields: 1=set id, 2=transaction id (placer-side), 3=batch id,
    // 4=date, 5=posting date, 6=type, 7=code, 8=description, 9=alt desc,
    // 10=quantity, 11=amount-extended (CP).
    let amount_str = charge.amount.amount.to_string();
    let currency_str = charge.amount.currency.0.clone();
    let amount_field = format!(
        "{}^{}",
        escape_value(&amount_str),
        escape_value(&currency_str)
    );
    let ft1 = Segment {
        name: "FT1".into(),
        fields: vec![
            "1".into(),                           // FT1-1 set id
            escape_value(&charge.id.to_string()), // FT1-2 transaction id (PAS uuid)
            String::new(),                        // FT1-3 batch id
            format_v2_datetime(charge.posted_at), // FT1-4 transaction date
            String::new(),                        // FT1-5 posting date
            "CG".into(),                          // FT1-6 transaction type (charge)
            escape_value(&charge.code),           // FT1-7 transaction code
            escape_value(&charge.description),    // FT1-8 transaction description
            String::new(),                        // FT1-9 alt desc
            String::new(),                        // FT1-10 quantity
            amount_field,                         // FT1-11 amount extended (CP)
        ],
    };
    encode_message(&Message {
        segments: vec![msh, evn, pid, ft1],
    })
}

/// Build an ADT^A40 (merge patient — patient ID) message.
///
/// `survivor` is the row that lives (PAS-side: the merge target); its
/// `PID` describes the kept identity. `source` is the row that became
/// the tombstone (PAS-side: the row whose `replaced_by` now points at
/// the survivor); its MRN populates the `MRG-1` field so the receiver
/// can locate the same patient locally.
///
/// PAS only emits the **first MRN identifier** in MRG-1, mirroring the
/// PID encoder. MRG-7 (Prior Patient Name) is intentionally left empty
/// because PAS doesn't track the source row's pre-merge name as a
/// distinct attribute — the row itself preserves it in the database.
pub fn encode_adt_a40(
    survivor: &Patient,
    source: &Patient,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    let now = format_v2_datetime(Utc::now());
    let msh = Segment {
        name: "MSH".into(),
        fields: vec![
            "|".into(),
            "^~\\&".into(),
            sending_app.into(),
            "FAC".into(),
            receiving_app.into(),
            "FAC".into(),
            now.clone(),
            "".into(),
            "ADT^A40".into(),
            message_control_id.into(),
            "P".into(),
            "2.5".into(),
        ],
    };
    let evn = Segment {
        name: "EVN".into(),
        fields: vec!["A40".into(), now],
    };
    let pid = pid_from_patient(survivor);
    let mrg = mrg_from_patient(source);
    encode_message(&Message {
        segments: vec![msh, evn, pid, mrg],
    })
}

/// Build an `MRG` segment from the source patient's MRN. Mirrors the
/// shape of PID-3 emitted by [`pid_from_patient`]: `<value>^^^<facility>^MR`.
/// When the patient has no MRN identifier, MRG-1 is left empty — the
/// receiver should then AE the message (parse_merge_source_mrn rejects).
fn mrg_from_patient(source: &Patient) -> Segment {
    let mrn = source
        .identifiers
        .iter()
        .find(|i| i.identifier_type == IdentifierType::MRN);
    let mrg1 = match mrn {
        Some(i) => {
            let facility = i.system.strip_prefix("urn:oid:facility:").unwrap_or("FAC");
            format!("{}^^^{}^MR", escape_value(&i.value), escape_value(facility))
        }
        None => String::new(),
    };
    Segment {
        name: "MRG".into(),
        fields: vec![mrg1],
    }
}

/// Extract the source patient's MRN from the `MRG` segment of an
/// inbound `ADT^A40`. Returns the bare MRN string (MRG-1.1) or
/// `Error::Validation` if the segment / component is missing.
pub fn parse_merge_source_mrn(m: &Message) -> Result<String> {
    let mrg = m
        .segment("MRG")
        .ok_or_else(|| Error::validation("missing MRG segment"))?;
    let mrn = unescape_value(mrg.component(1, 1));
    if mrn.is_empty() {
        return Err(Error::validation(
            "MRG-1.1 (prior patient MRN) must not be empty",
        ));
    }
    Ok(mrn)
}

// ----- SIU (Scheduling Information Unsolicited) ---------------------------
//
// v0.15: PAS understands SIU^S12 (notification of new appointment) and
// SIU^S15 (notification of appointment cancellation) on the inbound side,
// and emits both on the outbound side via the `Hl7v2MllpPublisher`.
//
// Segment subset:
// - MSH: standard header (`SIU^S12` or `SIU^S15` in MSH-9).
// - SCH: scheduling activity info — the minimal field set we honor is:
//   - SCH-1: Placer Appointment ID (caller's opaque id; preserved
//     verbatim, never parsed).
//   - SCH-2: Filler Appointment ID — for outbound this is the PAS
//     appointment UUID; for inbound S15 it MUST match a PAS UUID,
//     otherwise the receiver returns AE.
//   - SCH-7: Appointment Reason text → `Appointment.reason`.
//   - SCH-9 / SCH-10: appointment duration value + units (we always
//     emit `min`; on inbound we accept any units and compute end
//     from start + duration if the value is numeric).
//   - SCH-11: appointment start datetime, `YYYYMMDDHHMMSS`.
// - PID: same shape used by ADT — see `patient_from_pid` /
//   `pid_from_patient`.
//
// We deliberately do not emit AIG / AIL / AIP / AIS — the receiver doesn't
// need them to dedup, and most EMR-side schedulers expect the minimal
// subset above. Extending to richer SIU is straightforward but out of
// v0.15 scope.

/// Structured view of an inbound SIU message. Constructed via
/// [`parse_siu`]. Patient is built off the PID segment exactly like the
/// ADT path; the appointment fields come off SCH.
#[derive(Debug, Clone)]
pub struct SiuMessage {
    /// `S12` or `S15` (the trigger event from MSH-9.2).
    pub trigger: String,
    pub patient: Patient,
    /// Placer Appointment ID — opaque to PAS, preserved for ACK / audit.
    pub placer_appointment_id: Option<String>,
    /// Filler Appointment ID — for S15 this must be a parseable PAS
    /// UUID; for S12 it may be empty (PAS assigns it on accept).
    pub filler_appointment_id: Option<String>,
    /// SCH-11 parsed start datetime. Required for S12; ignored on S15.
    pub start_datetime: Option<chrono::DateTime<Utc>>,
    /// Computed from SCH-9 minutes (or defaults to 30 min when missing
    /// — see `parse_siu` for the fallback rule). Ignored on S15.
    pub end_datetime: Option<chrono::DateTime<Utc>>,
    /// SCH-7 reason text. Optional.
    pub reason: Option<String>,
}

/// Parse a SIU^S12 / SIU^S15 message into a [`SiuMessage`]. Returns
/// `Error::Validation` when required fields are missing or malformed.
pub fn parse_siu(m: &Message) -> Result<SiuMessage> {
    let (code, trigger) = message_type(m);
    if code != "SIU" {
        return Err(Error::validation(format!(
            "expected SIU message type, got {code}^{trigger}"
        )));
    }
    if !matches!(trigger.as_str(), "S12" | "S13" | "S14" | "S15") {
        return Err(Error::validation(format!(
            "unsupported SIU trigger event SIU^{trigger} (only S12, S13, S14, S15 understood as of v0.17)"
        )));
    }
    let pid = m
        .segment("PID")
        .ok_or_else(|| Error::validation("missing PID segment"))?;
    let patient = patient_from_pid(pid)?;
    let sch = m
        .segment("SCH")
        .ok_or_else(|| Error::validation("missing SCH segment"))?;
    let placer = nonempty_unescape(sch.component(1, 1));
    let filler = nonempty_unescape(sch.component(2, 1));

    // Time fields:
    // - S12 (book) requires SCH-11; we compute end from SCH-9 minutes
    //   (default 30) just like before.
    // - S13 (reschedule) also requires SCH-11 — it's the *new* start.
    // - S14 (modify) and S15 (cancel) ignore SCH-11.
    let (start, end) = if matches!(trigger.as_str(), "S12" | "S13") {
        let start_str = sch.field(11);
        if start_str.is_empty() {
            return Err(Error::validation(format!(
                "SCH-11 (appointment start datetime) is required for SIU^{trigger}"
            )));
        }
        let start = parse_v2_datetime(start_str).ok_or_else(|| {
            Error::validation(format!(
                "SCH-11 not a YYYYMMDDHHMMSS datetime: {start_str:?}"
            ))
        })?;
        let dur_min: i64 = sch
            .field(9)
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0 && *n <= 24 * 60)
            .unwrap_or(30);
        let end = start + chrono::Duration::minutes(dur_min);
        (Some(start), Some(end))
    } else {
        (None, None)
    };

    // S13 / S14 / S15 all identify the target appointment by SCH-2
    // (filler id). It's optional on S12 (PAS assigns it on accept) but
    // required for the modify / reschedule / cancel triggers.
    if matches!(trigger.as_str(), "S13" | "S14" | "S15") && filler.is_none() {
        return Err(Error::validation(format!(
            "SIU^{trigger} requires SCH-2 (filler appointment id)"
        )));
    }

    Ok(SiuMessage {
        trigger,
        patient,
        placer_appointment_id: placer,
        filler_appointment_id: filler,
        start_datetime: start,
        end_datetime: end,
        reason: nonempty_unescape(sch.field(7)),
    })
}

/// Build a SIU^S12 (notification of new appointment) message. The PAS
/// appointment UUID is rendered as the filler appointment id (SCH-2).
#[allow(clippy::too_many_arguments)]
pub fn encode_siu_s12(
    patient: &Patient,
    appointment_id: uuid::Uuid,
    placer_id: Option<&str>,
    start: chrono::DateTime<Utc>,
    end: chrono::DateTime<Utc>,
    reason: Option<&str>,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_siu(
        "S12",
        patient,
        appointment_id,
        placer_id,
        Some(start),
        Some(end),
        reason,
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build a SIU^S13 (notification of appointment rescheduling) message.
/// Carries the *new* start / end in SCH-11 + SCH-9 + SCH-10. The filler
/// id (PAS UUID in SCH-2) tells the receiver *which* appointment moved.
#[allow(clippy::too_many_arguments)]
pub fn encode_siu_s13(
    patient: &Patient,
    appointment_id: uuid::Uuid,
    placer_id: Option<&str>,
    start: chrono::DateTime<Utc>,
    end: chrono::DateTime<Utc>,
    reason: Option<&str>,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_siu(
        "S13",
        patient,
        appointment_id,
        placer_id,
        Some(start),
        Some(end),
        reason,
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build a SIU^S14 (notification of appointment modification) message.
/// Used to notify the receiver that non-time fields changed (typically
/// the appointment reason). Start / end are intentionally omitted — use
/// SIU^S13 for time changes.
pub fn encode_siu_s14(
    patient: &Patient,
    appointment_id: uuid::Uuid,
    placer_id: Option<&str>,
    reason: Option<&str>,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_siu(
        "S14",
        patient,
        appointment_id,
        placer_id,
        None,
        None,
        reason,
        sending_app,
        receiving_app,
        message_control_id,
    )
}

/// Build a SIU^S15 (notification of appointment cancellation) message.
/// Start / end / reason are optional in the cancellation; only the
/// filler appointment id (the PAS UUID) is strictly required for the
/// receiver to identify the appointment.
pub fn encode_siu_s15(
    patient: &Patient,
    appointment_id: uuid::Uuid,
    placer_id: Option<&str>,
    reason: Option<&str>,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    encode_siu(
        "S15",
        patient,
        appointment_id,
        placer_id,
        None,
        None,
        reason,
        sending_app,
        receiving_app,
        message_control_id,
    )
}

#[allow(clippy::too_many_arguments)]
fn encode_siu(
    event: &str,
    patient: &Patient,
    appointment_id: uuid::Uuid,
    placer_id: Option<&str>,
    start: Option<chrono::DateTime<Utc>>,
    end: Option<chrono::DateTime<Utc>>,
    reason: Option<&str>,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    let now = format_v2_datetime(Utc::now());
    let msh = Segment {
        name: "MSH".into(),
        fields: vec![
            "|".into(),
            "^~\\&".into(),
            sending_app.into(),
            "FAC".into(),
            receiving_app.into(),
            "FAC".into(),
            now,
            "".into(),
            format!("SIU^{event}"),
            message_control_id.into(),
            "P".into(),
            "2.5".into(),
        ],
    };
    let duration_minutes = match (start, end) {
        (Some(s), Some(e)) => {
            let mins = (e - s).num_minutes();
            if mins > 0 {
                mins.to_string()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };
    let sch_start = start.map(format_v2_datetime).unwrap_or_default();
    let sch = Segment {
        name: "SCH".into(),
        // Build a 12-element field vector so SCH-11 is addressable.
        // Field indexing matches HL7 1-based: fields[i-1] is SCH-i.
        fields: vec![
            placer_id.map(escape_value).unwrap_or_default(), // SCH-1
            appointment_id.to_string(),                      // SCH-2 (filler = PAS uuid)
            String::new(),                                   // SCH-3 occurrence
            String::new(),                                   // SCH-4 placer group
            String::new(),                                   // SCH-5 schedule id
            String::new(),                                   // SCH-6 event reason
            reason.map(escape_value).unwrap_or_default(),    // SCH-7 appointment reason
            String::new(),                                   // SCH-8 appointment type
            duration_minutes,                                // SCH-9 duration
            if start.is_some() && end.is_some() {
                // SCH-10 duration units
                "min".to_string()
            } else {
                String::new()
            },
            sch_start, // SCH-11 start datetime
        ],
    };
    let pid = pid_from_patient(patient);
    encode_message(&Message {
        segments: vec![msh, sch, pid],
    })
}

fn encode_adt(
    event: &str,
    patient: &Patient,
    bed: Option<&crate::models::facility::Bed>,
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    let now = format_v2_datetime(Utc::now());
    let msh = Segment {
        name: "MSH".into(),
        fields: vec![
            "|".into(),
            "^~\\&".into(),
            sending_app.into(),
            "FAC".into(),
            receiving_app.into(),
            "FAC".into(),
            now.clone(),
            "".into(),
            format!("ADT^{event}"),
            message_control_id.into(),
            "P".into(),
            "2.5".into(),
        ],
    };
    let evn = Segment {
        name: "EVN".into(),
        fields: vec![event.into(), now],
    };
    let pid = pid_from_patient(patient);
    let mut segments = vec![msh, evn, pid];
    if let Some(b) = bed {
        // PV1-1 set id, PV1-2 class (Inpatient), PV1-3 = "^^<bed-code>".
        let mut pv1_fields = vec![String::new(); 3];
        pv1_fields[0] = "1".into();
        pv1_fields[1] = "I".into();
        pv1_fields[2] = format!("^^{}", b.code);
        segments.push(Segment {
            name: "PV1".into(),
            fields: pv1_fields,
        });
    }
    encode_message(&Message { segments })
}

fn parse_v2_date(s: &str) -> Option<NaiveDate> {
    if s.len() < 8 {
        return None;
    }
    NaiveDate::parse_from_str(&s[..8], "%Y%m%d").ok()
}

/// Parse an HL7 v2 datetime field (`YYYYMMDDHHMMSS` or `YYYYMMDDHHMM`)
/// to a UTC `DateTime`. Returns `None` on any shape we don't recognise.
/// HL7 v2 datetimes carry an optional offset suffix; v0.15 ignores it
/// and treats the timestamp as UTC, matching the rest of the module.
fn parse_v2_datetime(s: &str) -> Option<chrono::DateTime<Utc>> {
    // Strip any trailing timezone offset (`+0500`, `-0530`) — v0.15 is
    // UTC-only on the wire.
    let core = s.split_once(['+', '-']).map(|(c, _)| c).unwrap_or(s);
    let parsed = if core.len() >= 14 {
        chrono::NaiveDateTime::parse_from_str(&core[..14], "%Y%m%d%H%M%S").ok()
    } else if core.len() >= 12 {
        chrono::NaiveDateTime::parse_from_str(&core[..12], "%Y%m%d%H%M").ok()
    } else if core.len() >= 8 {
        chrono::NaiveDate::parse_from_str(&core[..8], "%Y%m%d")
            .ok()
            .map(|d| d.and_hms_opt(0, 0, 0).unwrap())
    } else {
        None
    }?;
    Some(parsed.and_utc())
}

fn format_v2_date(d: NaiveDate) -> String {
    d.format("%Y%m%d").to_string()
}

fn format_v2_datetime(t: chrono::DateTime<Utc>) -> String {
    t.format("%Y%m%d%H%M%S").to_string()
}

fn nonempty_unescape(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(unescape_value(s))
    }
}

// ----- MFN (Master File Notification — Staff) -----------------------------
//
// v0.24: PAS understands MFN^M02 (master file - staff practitioner)
// on the inbound side. A single message can carry many MFE segments
// (one per practitioner) with the matching STF (Staff Identification)
// segment carrying the practitioner's master data.
//
// Segment subset honored:
// - MSH: standard header (`MFN^M02` in MSH-9).
// - MFI: master file identification.
//   - MFI-1 (Master File Identifier): expected `PRA` (practitioner
//     master). Other identifiers AE-ACK as unsupported.
//   - MFI-3 (File-level Event Code): not interpreted (v0.24 trusts
//     the per-record codes in MFE-1).
// - MFE (one per practitioner record):
//   - MFE-1 (Record-Level Event Code): `MAD` (add), `MUP` (update),
//     `MDL` (delete — soft delete via `active = false`).
//   - MFE-4 (Primary Key Value): the EMR's staff id. PAS stores
//     this as a `Identifier { type: Other, system: "urn:hl7v2:staff:id" }`
//     on the practitioner row and uses it to locate the row for
//     `MUP` / `MDL`.
// - STF (Staff Identification):
//   - STF-1 (Primary Key Value): same EMR staff id (MFE-4
//     duplicates it for record-level routing); the parser prefers
//     MFE-4 when present and falls back to STF-1 otherwise.
//   - STF-3 (Staff Name, XPN): `family^given^middle` → `HumanName`.
//   - STF-5 (Administrative Sex): `M`/`F`/`O`/`U` → `Gender`.
//   - STF-6 (Date/Time of Birth): `YYYYMMDD` → `NaiveDate`.
//   - STF-7 (Active/Inactive Flag): `A` → active = true, `I` →
//     active = false. Defaults to `true` when empty.
//
// PRA (Practitioner Detail) is intentionally not parsed in v0.24 —
// EMRs that need practitioner specialty / privilege detail can
// piggy-back on the REST `PUT /api/practitioners/{id}` surface.

/// One MFE+STF row from an inbound MFN^M02 message.
#[derive(Debug, Clone)]
pub struct MfnM02Item {
    /// MFE-1 record-level event code: `MAD` (add), `MUP` (update),
    /// `MDL` (delete). Unknown codes are rejected by the parser.
    pub event_code: String,
    /// EMR's staff id (MFE-4 with STF-1 fallback). The handler stores
    /// this as a practitioner identifier so subsequent MUP / MDL can
    /// locate the same row.
    pub primary_key: String,
    /// Practitioner name from STF-3.
    pub name: HumanName,
    /// From STF-5; defaults to `Gender::Unknown` when empty.
    pub gender: Gender,
    /// From STF-6.
    pub birth_date: Option<chrono::NaiveDate>,
    /// From STF-7 (`A`/`I`); defaults to `true` for `MAD` and
    /// `MUP` when STF-7 is empty.
    pub active: bool,
}

/// Structured view of an inbound MFN^M02 message. The parser
/// guarantees `items` is non-empty.
#[derive(Debug, Clone)]
pub struct MfnM02Message {
    /// MFI-1; always `"PRA"` for v0.24 (other values rejected).
    pub master_file_id: String,
    pub items: Vec<MfnM02Item>,
}

/// Parse an `MFN^M02` message into a [`MfnM02Message`]. Walks every
/// `MFE` segment and pairs it with the immediately-following `STF`
/// segment.
pub fn parse_mfn_m02(m: &Message) -> Result<MfnM02Message> {
    let (code, trigger) = message_type(m);
    if code != "MFN" || trigger != "M02" {
        return Err(Error::validation(format!(
            "expected MFN^M02, got {code}^{trigger}"
        )));
    }
    let mfi = m
        .segment("MFI")
        .ok_or_else(|| Error::validation("missing MFI segment"))?;
    let master_file_id = unescape_value(mfi.component(1, 1));
    if master_file_id != "PRA" {
        return Err(Error::validation(format!(
            "MFI-1 master file id {master_file_id:?} not supported (only PRA = practitioner master as of v0.24)"
        )));
    }

    // Walk segments in document order so we can pair each MFE with the
    // STF that immediately follows it. HL7 v2 MFN semantics put the STF
    // (and any PRA, OBX, etc.) directly after its MFE.
    let mut items: Vec<MfnM02Item> = Vec::new();
    let mut pending_mfe: Option<(usize, &Segment)> = None;
    for (idx, seg) in m.segments.iter().enumerate() {
        match seg.name.as_str() {
            "MFE" => {
                if let Some((mfe_idx, _)) = pending_mfe.take() {
                    return Err(Error::validation(format!(
                        "MFE at segment {mfe_idx} has no STF before next MFE"
                    )));
                }
                pending_mfe = Some((idx, seg));
            }
            "STF" => {
                let (_, mfe) = pending_mfe.take().ok_or_else(|| {
                    Error::validation(format!("STF at segment {idx} not preceded by an MFE"))
                })?;
                items.push(parse_mfn_item(mfe, seg, items.len() + 1)?);
            }
            _ => {}
        }
    }
    if pending_mfe.is_some() {
        return Err(Error::validation(
            "trailing MFE with no STF — every MFE must be paired with an STF",
        ));
    }
    if items.is_empty() {
        return Err(Error::validation(
            "MFN^M02 must carry at least one MFE+STF pair",
        ));
    }
    Ok(MfnM02Message {
        master_file_id,
        items,
    })
}

fn parse_mfn_item(mfe: &Segment, stf: &Segment, idx_1based: usize) -> Result<MfnM02Item> {
    let event_code = unescape_value(mfe.field(1));
    if !matches!(event_code.as_str(), "MAD" | "MUP" | "MDL") {
        return Err(Error::validation(format!(
            "MFE[{idx_1based}]-1 event code {event_code:?} not supported (only MAD / MUP / MDL as of v0.24)"
        )));
    }
    // Primary key: prefer MFE-4 (the record-level routing key), fall
    // back to STF-1.
    let primary_key = nonempty_unescape(mfe.component(4, 1))
        .or_else(|| nonempty_unescape(stf.component(1, 1)))
        .ok_or_else(|| {
            Error::validation(format!(
                "MFE[{idx_1based}] requires either MFE-4 (primary key) or STF-1"
            ))
        })?;
    let family = unescape_value(stf.component(3, 1));
    if family.is_empty() {
        return Err(Error::validation(format!(
            "STF[{idx_1based}]-3.1 (family name) must not be empty"
        )));
    }
    let given_primary = unescape_value(stf.component(3, 2));
    let middle = unescape_value(stf.component(3, 3));
    let mut given = Vec::new();
    if !given_primary.is_empty() {
        given.push(given_primary);
    }
    if !middle.is_empty() {
        given.push(middle);
    }
    let name = HumanName {
        use_type: Some(NameUse::Official),
        family,
        given,
        prefix: vec![],
        suffix: vec![],
    };
    let gender = match stf.field(5) {
        "M" => Gender::Male,
        "F" => Gender::Female,
        "O" => Gender::Other,
        _ => Gender::Unknown,
    };
    let birth_date = parse_v2_date(stf.field(6));
    let active = match stf.field(7) {
        "A" | "a" => true,
        "I" | "i" => false,
        _ => true,
    };
    Ok(MfnM02Item {
        event_code,
        primary_key,
        name,
        gender,
        birth_date,
        active,
    })
}

/// Build an MFN^M02 message from a list of practitioner items. Used
/// for unit tests today and reserved for an outbound publisher
/// mapping if/when one is added.
pub fn encode_mfn_m02(
    items: &[MfnM02Item],
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    let now = format_v2_datetime(Utc::now());
    let msh = Segment {
        name: "MSH".into(),
        fields: vec![
            "|".into(),
            "^~\\&".into(),
            sending_app.into(),
            "FAC".into(),
            receiving_app.into(),
            "FAC".into(),
            now.clone(),
            "".into(),
            "MFN^M02".into(),
            message_control_id.into(),
            "P".into(),
            "2.5".into(),
        ],
    };
    let mfi = Segment {
        name: "MFI".into(),
        // MFI-1 = PRA, MFI-3 = UPD (update — file-level event).
        fields: vec!["PRA".into(), String::new(), "UPD".into()],
    };
    let mut segments = vec![msh, mfi];
    for item in items {
        // MFE-1=event_code, MFE-2=event-id (unused), MFE-3=event datetime, MFE-4=primary key
        segments.push(Segment {
            name: "MFE".into(),
            fields: vec![
                item.event_code.clone(),
                String::new(),
                now.clone(),
                escape_value(&item.primary_key),
            ],
        });
        let middle = item.name.given.get(1).cloned().unwrap_or_default();
        let first = item.name.given.first().cloned().unwrap_or_default();
        let family = escape_value(&item.name.family);
        let stf3 = if first.is_empty() && middle.is_empty() {
            family
        } else if middle.is_empty() {
            format!("{family}^{}", escape_value(&first))
        } else {
            format!(
                "{family}^{}^{}",
                escape_value(&first),
                escape_value(&middle)
            )
        };
        let gender = match item.gender {
            Gender::Male => "M",
            Gender::Female => "F",
            Gender::Other => "O",
            Gender::Unknown => "U",
        };
        let birth = item.birth_date.map(format_v2_date).unwrap_or_default();
        let active = if item.active { "A" } else { "I" };
        // STF has many fields; populate STF-1..STF-7 and leave the
        // rest empty. Index 1 = STF-1.
        segments.push(Segment {
            name: "STF".into(),
            fields: vec![
                escape_value(&item.primary_key), // STF-1
                String::new(),                   // STF-2 identifier list (deferred)
                stf3,                            // STF-3 name
                String::new(),                   // STF-4 staff type
                gender.into(),                   // STF-5 sex
                birth,                           // STF-6 birth date
                active.into(),                   // STF-7 active/inactive
            ],
        });
    }
    encode_message(&Message { segments })
}

// ----- MFN^M05 (Master File Notification — Location) ----------------------
//
// v0.26: PAS understands MFN^M05 (master file notification - patient
// location). The PAS bed roster (the leaf of the
// facility → ward → room → bed hierarchy) is the operational
// location surface, so this handler manages Bed rows.
//
// Segment subset honored:
// - MSH: `MFN^M05` in MSH-9.
// - MFI: MFI-1 must equal `LOC`. Other master file ids AE-ACK.
// - MFE (one per bed):
//   - MFE-1: `MAD` / `MUP` / `MDL`.
//   - MFE-4: optional; primary key fallback when LOC-1 is missing.
// - LOC (Location Identification):
//   - LOC-1 (Patient Location, PL type):
//     - LOC-1.1 (PL.1, Point of Care): the parent **room code**
//       in PAS. Required for `MAD` (so the handler can resolve
//       `room_id`). Ignored on `MUP` / `MDL` if not changing.
//     - LOC-1.3 (PL.3, Bed): the **bed code**. This is the
//       primary lookup key for `MUP` / `MDL` and the value
//       stored on the new row for `MAD`.
//   - LOC-2 (Location Description): free-form text → `Bed.name`.
//
// `MDL` is a **soft delete via the bed status** (`OutOfService`)
// rather than a row delete; bed rows are referenced by admissions
// / bed assignments and a hard delete would orphan them. The flip
// goes through `BedRepository::set_status_unchecked` because the
// state machine doesn't model "any status → OutOfService" for
// occupied beds (you'd normally discharge first); MFN-driven
// retirement is treated as an operator-authorised bypass, same
// pattern as v0.4 ADT^A13.

/// One MFE+LOC row from an inbound MFN^M05 message.
#[derive(Debug, Clone)]
pub struct MfnM05Item {
    /// MFE-1 record-level event code: `MAD`, `MUP`, or `MDL`.
    pub event_code: String,
    /// LOC-1.3 — the bed code. Required.
    pub bed_code: String,
    /// LOC-1.1 — the parent room code. Required for `MAD`
    /// (where PAS must resolve a `room_id`); optional for `MUP`
    /// (allows moving a bed to a different room) and ignored
    /// for `MDL`.
    pub room_code: Option<String>,
    /// LOC-2 — free-form name / description. Required for `MAD`,
    /// optional for `MUP`, ignored for `MDL`.
    pub name: Option<String>,
}

/// Structured view of an inbound MFN^M05 message. `items` is
/// guaranteed non-empty by the parser.
#[derive(Debug, Clone)]
pub struct MfnM05Message {
    /// MFI-1; always `"LOC"` for v0.26.
    pub master_file_id: String,
    pub items: Vec<MfnM05Item>,
}

/// Parse an `MFN^M05` message into [`MfnM05Message`].
pub fn parse_mfn_m05(m: &Message) -> Result<MfnM05Message> {
    let (code, trigger) = message_type(m);
    if code != "MFN" || trigger != "M05" {
        return Err(Error::validation(format!(
            "expected MFN^M05, got {code}^{trigger}"
        )));
    }
    let mfi = m
        .segment("MFI")
        .ok_or_else(|| Error::validation("missing MFI segment"))?;
    let master_file_id = unescape_value(mfi.component(1, 1));
    if master_file_id != "LOC" {
        return Err(Error::validation(format!(
            "MFI-1 master file id {master_file_id:?} not supported (only LOC = location master as of v0.26)"
        )));
    }

    // Walk segments in document order; pair each MFE with the LOC
    // that immediately follows it. Same shape as the MFN^M02 walker.
    let mut items: Vec<MfnM05Item> = Vec::new();
    let mut pending_mfe: Option<(usize, &Segment)> = None;
    for (idx, seg) in m.segments.iter().enumerate() {
        match seg.name.as_str() {
            "MFE" => {
                if let Some((mfe_idx, _)) = pending_mfe.take() {
                    return Err(Error::validation(format!(
                        "MFE at segment {mfe_idx} has no LOC before next MFE"
                    )));
                }
                pending_mfe = Some((idx, seg));
            }
            "LOC" => {
                let (_, mfe) = pending_mfe.take().ok_or_else(|| {
                    Error::validation(format!("LOC at segment {idx} not preceded by an MFE"))
                })?;
                items.push(parse_m05_item(mfe, seg, items.len() + 1)?);
            }
            _ => {}
        }
    }
    if pending_mfe.is_some() {
        return Err(Error::validation(
            "trailing MFE with no LOC — every MFE must be paired with a LOC",
        ));
    }
    if items.is_empty() {
        return Err(Error::validation(
            "MFN^M05 must carry at least one MFE+LOC pair",
        ));
    }
    Ok(MfnM05Message {
        master_file_id,
        items,
    })
}

fn parse_m05_item(mfe: &Segment, loc: &Segment, idx_1based: usize) -> Result<MfnM05Item> {
    let event_code = unescape_value(mfe.field(1));
    if !matches!(event_code.as_str(), "MAD" | "MUP" | "MDL") {
        return Err(Error::validation(format!(
            "MFE[{idx_1based}]-1 event code {event_code:?} not supported (only MAD / MUP / MDL as of v0.26)"
        )));
    }
    // LOC-1 is PL-typed: PL.1 = room, PL.3 = bed. The MFE-4 primary
    // key is a sender-side opaque id that we accept as a fallback
    // bed code when LOC-1.3 is empty (some EMRs put the bed id
    // there instead of in PL.3).
    let bed_code = nonempty_unescape(loc.component(1, 3))
        .or_else(|| nonempty_unescape(mfe.component(4, 1)))
        .ok_or_else(|| {
            Error::validation(format!(
                "MFE[{idx_1based}] requires LOC-1.3 (bed code) or MFE-4 fallback"
            ))
        })?;
    let room_code = nonempty_unescape(loc.component(1, 1));
    let name = nonempty_unescape(loc.field(2));
    if event_code == "MAD" {
        if room_code.is_none() {
            return Err(Error::validation(format!(
                "MFE[{idx_1based}] MAD requires LOC-1.1 (parent room code)"
            )));
        }
        if name.is_none() {
            return Err(Error::validation(format!(
                "MFE[{idx_1based}] MAD requires LOC-2 (bed name / description)"
            )));
        }
    }
    Ok(MfnM05Item {
        event_code,
        bed_code,
        room_code,
        name,
    })
}

/// Build an `MFN^M05` message from a list of bed items. Exposed for
/// unit tests; not used by any current outbound publisher.
pub fn encode_mfn_m05(
    items: &[MfnM05Item],
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
) -> String {
    let now = format_v2_datetime(Utc::now());
    let msh = Segment {
        name: "MSH".into(),
        fields: vec![
            "|".into(),
            "^~\\&".into(),
            sending_app.into(),
            "FAC".into(),
            receiving_app.into(),
            "FAC".into(),
            now.clone(),
            "".into(),
            "MFN^M05".into(),
            message_control_id.into(),
            "P".into(),
            "2.5".into(),
        ],
    };
    let mfi = Segment {
        name: "MFI".into(),
        fields: vec!["LOC".into(), String::new(), "UPD".into()],
    };
    let mut segments = vec![msh, mfi];
    for item in items {
        segments.push(Segment {
            name: "MFE".into(),
            fields: vec![
                item.event_code.clone(),
                String::new(),
                now.clone(),
                escape_value(&item.bed_code),
            ],
        });
        // LOC-1 is PL-typed: `<room>^^<bed>` (PL.1=room, PL.2=empty,
        // PL.3=bed). Skip PL components beyond .3.
        let loc1 = format!(
            "{}^^{}",
            item.room_code
                .as_deref()
                .map(escape_value)
                .unwrap_or_default(),
            escape_value(&item.bed_code),
        );
        segments.push(Segment {
            name: "LOC".into(),
            fields: vec![
                loc1,
                item.name.as_deref().map(escape_value).unwrap_or_default(),
            ],
        });
    }
    encode_message(&Message { segments })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hl7v2::parse_message;

    const A01: &str = "MSH|^~\\&|SENDAPP|FAC|RECVAPP|FAC|20260523120000||ADT^A01|MSG001|P|2.5\r\
EVN|A01|20260523120000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F|||123 Elm^^Springfield^IL^62701^US||(555)555-0100\r\
PV1|1|I|WARD1^ROOM1^BED1|||||||||||||||VISIT001\r";

    #[test]
    fn test_message_type_a01() {
        let m = parse_message(A01).expect("parse");
        assert_eq!(message_type(&m), ("ADT".to_string(), "A01".to_string()));
    }

    #[test]
    fn test_patient_from_pid_populates_core_fields() {
        let m = parse_message(A01).expect("parse");
        let pid = m.segment("PID").expect("PID");
        let p = patient_from_pid(pid).expect("map");
        assert_eq!(p.name.family, "Doe");
        assert_eq!(p.name.given, vec!["Jane".to_string(), "Marie".into()]);
        assert_eq!(p.gender, Gender::Female);
        assert_eq!(
            p.birth_date,
            Some(NaiveDate::from_ymd_opt(1990, 1, 15).unwrap())
        );
        assert_eq!(p.identifiers.len(), 1);
        assert_eq!(p.identifiers[0].value, "MRN-001");
        assert_eq!(p.identifiers[0].identifier_type, IdentifierType::MRN);
        assert_eq!(p.telecom.len(), 1);
        assert_eq!(p.telecom[0].value, "(555)555-0100");
        assert_eq!(p.addresses.len(), 1);
        assert_eq!(p.addresses[0].city.as_deref(), Some("Springfield"));
        assert_eq!(p.addresses[0].postal_code.as_deref(), Some("62701"));
    }

    #[test]
    fn test_patient_from_pid_rejects_empty_family_name() {
        let bad = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A28|X|P|2.5\r\
PID|1||MRN-001^^^FAC^MR||||19900115|F\r";
        let m = parse_message(bad).expect("parse");
        let pid = m.segment("PID").expect("PID");
        assert!(matches!(patient_from_pid(pid), Err(Error::Validation(_))));
    }

    #[test]
    fn test_pid_from_patient_round_trip_preserves_name_and_gender() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let pid_back = pid_from_patient(&p);
        // PID-5 should still encode "Doe^Jane^Marie".
        assert_eq!(pid_back.field(5), "Doe^Jane^Marie");
        // PID-8 should be F.
        assert_eq!(pid_back.field(8), "F");
        // PID-7 should be 19900115.
        assert_eq!(pid_back.field(7), "19900115");
        // PID-3 should round-trip MRN with facility.
        assert_eq!(pid_back.field(3), "MRN-001^^^FAC^MR");
    }

    // ----- PID-29 / PID-30 deceased round-trip (v0.22) ------------------

    #[test]
    fn test_patient_from_pid_reads_pid29_pid30() {
        // PID with PID-29 = 20260601120000 and PID-30 = Y.
        // PID structure: PID|1|external|MRN^^^FAC^MR||Doe^Jane||birth|F|
        // … 11 more fields … |||||||||||death-dt|Y
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601120000||ADT^A08|MSG-D|P|2.5\r\
PID|1||MRN-DEC^^^FAC^MR||Smith^John||19500115|M|||||||||||||||||||||20260601120000|Y\r";
        let m = parse_message(wire).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        assert!(p.deceased, "PID-30=Y must set deceased=true");
        let dt = p.deceased_datetime.expect("PID-29 must populate dt");
        assert_eq!(dt.format("%Y%m%d%H%M%S").to_string(), "20260601120000");
    }

    #[test]
    fn test_patient_from_pid_infers_deceased_from_pid29_when_pid30_missing() {
        // PID-29 set, PID-30 empty → deceased = true (inferred).
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601120000||ADT^A08|MSG-D|P|2.5\r\
PID|1||MRN-DEC^^^FAC^MR||Smith^John||19500115|M|||||||||||||||||||||20260601120000\r";
        let m = parse_message(wire).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        assert!(p.deceased, "PID-29 alone must imply deceased=true");
        assert!(p.deceased_datetime.is_some());
    }

    #[test]
    fn test_patient_from_pid_honors_pid30_n_when_pid29_absent() {
        // PID-30 = N, no PID-29 → deceased = false.
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601120000||ADT^A08|MSG-D|P|2.5\r\
PID|1||MRN-LIVE^^^FAC^MR||Smith^John||19500115|M||||||||||||||||||||||N\r";
        let m = parse_message(wire).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        assert!(!p.deceased);
        assert!(p.deceased_datetime.is_none());
    }

    #[test]
    fn test_pid_from_patient_emits_pid29_pid30_when_deceased() {
        let m = parse_message(A01).expect("parse");
        let mut p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        p.deceased = true;
        p.deceased_datetime = Some(
            chrono::NaiveDate::from_ymd_opt(2027, 3, 15)
                .unwrap()
                .and_hms_opt(8, 45, 0)
                .unwrap()
                .and_utc(),
        );
        let pid = pid_from_patient(&p);
        assert_eq!(pid.field(29), "20270315084500");
        assert_eq!(pid.field(30), "Y");
    }

    #[test]
    fn test_pid_from_patient_omits_pid29_pid30_when_alive() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        // Default A01 fixture has no death — encoder should stay
        // compact (14 fields) and PID-29/30 should both be empty.
        let pid = pid_from_patient(&p);
        assert_eq!(pid.field(29), "");
        assert_eq!(pid.field(30), "");
    }

    #[test]
    fn test_pid_round_trip_preserves_deceased_with_datetime() {
        let m = parse_message(A01).expect("parse");
        let mut p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        p.deceased = true;
        let dt = chrono::NaiveDate::from_ymd_opt(2027, 7, 4)
            .unwrap()
            .and_hms_opt(23, 59, 59)
            .unwrap()
            .and_utc();
        p.deceased_datetime = Some(dt);
        let pid = pid_from_patient(&p);
        // Wrap the encoded PID in a parseable MSH-prefixed message so
        // we can round-trip via the standard parser.
        let wire = format!(
            "MSH|^~\\&|PAS|FAC|EMR|FAC|20270704235959||ADT^A08|MSG-RT|P|2.5\r\
{}\r",
            crate::hl7v2::encode_message(&Message {
                segments: vec![pid],
            })
            .lines()
            .find(|l| l.starts_with("PID|"))
            .expect("PID line")
        );
        let m2 = parse_message(&wire).expect("re-parse");
        let p2 = patient_from_pid(m2.segment("PID").unwrap()).expect("map back");
        assert!(p2.deceased);
        assert_eq!(p2.deceased_datetime, Some(dt));
    }

    #[test]
    fn test_encode_adt_a01_includes_pv1_bed_code() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let bed = crate::models::facility::Bed::new(
            uuid::Uuid::new_v4(),
            "Bed X".into(),
            "BED-CODE-XYZ".into(),
        );
        let out = encode_adt_a01(&p, &bed, "PAS", "EMR", "MSG-OUT-A01");
        assert!(out.contains("|ADT^A01|MSG-OUT-A01|"));
        assert!(out.contains("\rEVN|A01|"));
        assert!(out.contains("\rPV1|1|I|^^BED-CODE-XYZ"));
    }

    #[test]
    fn test_encode_adt_a02_uses_destination_bed_code() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let bed = crate::models::facility::Bed::new(
            uuid::Uuid::new_v4(),
            "Bed New".into(),
            "BED-NEW".into(),
        );
        let out = encode_adt_a02(&p, &bed, "PAS", "EMR", "MSG-OUT-A02");
        assert!(out.contains("|ADT^A02|MSG-OUT-A02|"));
        assert!(out.contains("\rPV1|1|I|^^BED-NEW"));
    }

    #[test]
    fn test_encode_adt_a04_outpatient_pv1_class_o() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let out = encode_adt_a04(&p, "O", "PAS", "EMR", "MSG-OUT-A04-O");
        assert!(out.contains("|ADT^A04|MSG-OUT-A04-O|"));
        assert!(out.contains("\rEVN|A04|"));
        assert!(out.contains("\rPV1|1|O"));
        assert!(!out.contains("\rPV1|1|I"));
    }

    #[test]
    fn test_encode_adt_a04_emergency_pv1_class_e() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let out = encode_adt_a04(&p, "E", "PAS", "EMR", "MSG-OUT-A04-E");
        assert!(out.contains("|ADT^A04|MSG-OUT-A04-E|"));
        assert!(out.contains("\rPV1|1|E"));
    }

    #[test]
    fn test_encode_adt_a06_includes_bed_code() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let bed = crate::models::facility::Bed::new(
            uuid::Uuid::new_v4(),
            "Bed Promote".into(),
            "BED-A06".into(),
        );
        let out = encode_adt_a06(&p, &bed, "PAS", "EMR", "MSG-OUT-A06");
        assert!(out.contains("|ADT^A06|MSG-OUT-A06|"));
        assert!(out.contains("\rEVN|A06|"));
        assert!(out.contains("\rPV1|1|I|^^BED-A06"));
    }

    #[test]
    fn test_encode_adt_a23_omits_pv1() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let out = encode_adt_a23(&p, "PAS", "EMR", "MSG-OUT-A23");
        assert!(out.contains("|ADT^A23|MSG-OUT-A23|"));
        assert!(out.contains("\rEVN|A23|"));
        assert!(!out.contains("\rPV1|"));
    }

    #[test]
    fn test_encode_adt_a21_omits_pv1() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let out = encode_adt_a21(&p, "PAS", "EMR", "MSG-OUT-A21");
        assert!(out.contains("|ADT^A21|MSG-OUT-A21|"));
        assert!(out.contains("\rEVN|A21|"));
        assert!(!out.contains("\rPV1|"));
    }

    #[test]
    fn test_encode_adt_a22_omits_pv1() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let out = encode_adt_a22(&p, "PAS", "EMR", "MSG-OUT-A22");
        assert!(out.contains("|ADT^A22|MSG-OUT-A22|"));
        assert!(out.contains("\rEVN|A22|"));
        assert!(!out.contains("\rPV1|"));
    }

    #[test]
    fn test_encode_adt_a38_includes_released_bed_code() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let bed = crate::models::facility::Bed::new(
            uuid::Uuid::new_v4(),
            "Bed Released".into(),
            "BED-REL-A38".into(),
        );
        let out = encode_adt_a38(&p, &bed, "PAS", "EMR", "MSG-OUT-A38");
        assert!(out.contains("|ADT^A38|MSG-OUT-A38|"));
        assert!(out.contains("\rEVN|A38|"));
        assert!(out.contains("\rPV1|1|I|^^BED-REL-A38"));
    }

    #[test]
    fn test_encode_adt_a05_includes_reserved_bed_code() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let bed = crate::models::facility::Bed::new(
            uuid::Uuid::new_v4(),
            "Bed Reserved".into(),
            "BED-RES-A05".into(),
        );
        let out = encode_adt_a05(&p, &bed, "PAS", "EMR", "MSG-OUT-A05");
        assert!(out.contains("|ADT^A05|MSG-OUT-A05|"));
        assert!(out.contains("\rEVN|A05|"));
        assert!(out.contains("\rPV1|1|I|^^BED-RES-A05"));
    }

    #[test]
    fn test_encode_adt_a12_includes_origin_bed_code() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let bed = crate::models::facility::Bed::new(
            uuid::Uuid::new_v4(),
            "Bed Origin".into(),
            "BED-ORIGIN-A12".into(),
        );
        let out = encode_adt_a12(&p, &bed, "PAS", "EMR", "MSG-OUT-A12");
        assert!(out.contains("|ADT^A12|MSG-OUT-A12|"));
        assert!(out.contains("\rEVN|A12|"));
        assert!(out.contains("\rPV1|1|I|^^BED-ORIGIN-A12"));
    }

    #[test]
    fn test_encode_adt_a03_omits_pv1() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let out = encode_adt_a03(&p, "PAS", "EMR", "MSG-OUT-A03");
        assert!(out.contains("|ADT^A03|MSG-OUT-A03|"));
        assert!(out.contains("\rEVN|A03|"));
        assert!(!out.contains("\rPV1|"));
    }

    #[test]
    fn test_round_trip_preserves_caret_in_family_name() {
        // Patient family name contains a literal '^', which is the v2
        // component separator. Without escaping this would silently split
        // into a phantom component on the wire.
        let name = HumanName {
            use_type: Some(NameUse::Official),
            family: "O^Brien-Jones".into(),
            given: vec!["Anne".into()],
            prefix: vec![],
            suffix: vec![],
        };
        let mut p = Patient::new(name, Gender::Female);
        p.identifiers = vec![crate::models::identifier::Identifier::mrn(
            "urn:oid:facility:1",
            "MRN-001",
        )];
        let pid = pid_from_patient(&p);
        // Wire-level: PID-5.1 carries the escape sequence \S\ in place of '^'.
        assert!(
            pid.field(5).starts_with("O\\S\\Brien-Jones"),
            "PID-5 escape not present: {:?}",
            pid.field(5)
        );
        // PID-5.2 is still "Anne" — the unescaped '^' has not been promoted
        // to a real component boundary.
        assert_eq!(pid.component(5, 2), "Anne");
        // Round-trip through patient_from_pid restores the literal name.
        let back = patient_from_pid(&pid).expect("decode");
        assert_eq!(back.name.family, "O^Brien-Jones");
        assert_eq!(back.name.given, vec!["Anne".to_string()]);
    }

    #[test]
    fn test_round_trip_preserves_pipe_and_ampersand_in_address() {
        let name = HumanName {
            use_type: Some(NameUse::Official),
            family: "Smith".into(),
            given: vec!["John".into()],
            prefix: vec![],
            suffix: vec![],
        };
        let mut p = Patient::new(name, Gender::Male);
        p.addresses = vec![Address {
            use_type: Some(AddressUse::Home),
            line1: Some("Apt B&B | 101".into()),
            line2: None,
            city: Some("Boston".into()),
            state: None,
            postal_code: Some("02108".into()),
            country: Some("US".into()),
        }];
        let pid = pid_from_patient(&p);
        // Re-wrap as a message so we can run the round-trip via patient_from_pid.
        let back = patient_from_pid(&pid).expect("decode");
        assert_eq!(back.addresses[0].line1.as_deref(), Some("Apt B&B | 101"));
    }

    #[test]
    fn test_encode_adt_a08_includes_pid_and_omits_pv1() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let out = encode_adt_a08(&p, "PAS", "EMR", "MSG-OUT-A08");
        assert!(out.contains("|ADT^A08|MSG-OUT-A08|"));
        assert!(out.contains("\rEVN|A08|"));
        assert!(out.contains("\rPID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F"));
        assert!(!out.contains("\rPV1|"));
    }

    #[test]
    fn test_encode_adt_a11_omits_pv1() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let out = encode_adt_a11(&p, "PAS", "EMR", "MSG-OUT-A11");
        assert!(out.contains("|ADT^A11|MSG-OUT-A11|"));
        assert!(out.contains("\rEVN|A11|"));
        assert!(!out.contains("\rPV1|"));
    }

    #[test]
    fn test_encode_adt_a13_includes_pv1_bed_code() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let bed = crate::models::facility::Bed::new(
            uuid::Uuid::new_v4(),
            "Bed Restore".into(),
            "BED-RESTORE".into(),
        );
        let out = encode_adt_a13(&p, &bed, "PAS", "EMR", "MSG-OUT-A13");
        assert!(out.contains("|ADT^A13|MSG-OUT-A13|"));
        assert!(out.contains("\rEVN|A13|"));
        assert!(out.contains("\rPV1|1|I|^^BED-RESTORE"));
    }

    // ----- DFT^P03 (v0.19) ----------------------------------------------

    const DFT_WIRE: &str = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-DFT-1|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r\
FT1|1|EXT-TXN-9||20260601100000||CG|97110|Therapeutic exercise|||125.50^USD\r";

    #[test]
    fn test_parse_dft_p03_happy_path() {
        let m = parse_message(DFT_WIRE).expect("parse");
        let dft = parse_dft_p03(&m).expect("parse_dft");
        assert_eq!(dft.patient.name.family, "Doe");
        assert_eq!(dft.items.len(), 1);
        let item = &dft.items[0];
        assert_eq!(item.transaction_type, "CG");
        assert_eq!(item.code, "97110");
        assert_eq!(item.description, "Therapeutic exercise");
        assert_eq!(
            item.amount,
            rust_decimal::Decimal::from_str_exact("125.50").unwrap()
        );
        assert_eq!(item.currency, "USD");
        assert!(item.posted_at.is_some());
    }

    #[test]
    fn test_parse_dft_p03_defaults_transaction_type_to_cg() {
        // FT1-6 left empty — handler defaults to CG.
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-X|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r\
FT1|1||||||97110|Visit|||50^USD\r";
        let m = parse_message(wire).expect("parse");
        let dft = parse_dft_p03(&m).expect("parse_dft");
        assert_eq!(dft.items.len(), 1);
        assert_eq!(dft.items[0].transaction_type, "CG");
        assert_eq!(dft.items[0].amount, rust_decimal::Decimal::from(50));
    }

    #[test]
    fn test_parse_dft_p03_walks_multiple_ft1_segments() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-MULTI|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r\
FT1|1|||20260601100000||CG|97110|Therapeutic exercise|||125.50^USD\r\
FT1|2|||20260601100500||CG|99213|Office visit|||80.00^USD\r\
FT1|3|||20260601100800||CG|J0696|Antibiotic injection|||15.25^USD\r";
        let m = parse_message(wire).expect("parse");
        let dft = parse_dft_p03(&m).expect("parse_dft");
        assert_eq!(dft.items.len(), 3, "all three FT1 segments must parse");
        assert_eq!(dft.items[0].code, "97110");
        assert_eq!(dft.items[1].code, "99213");
        assert_eq!(dft.items[2].code, "J0696");
        for item in &dft.items {
            assert_eq!(item.currency, "USD");
        }
    }

    #[test]
    fn test_parse_dft_p03_reports_per_item_field_index_on_error() {
        // Second FT1 has a bad amount — the error message should name
        // FT1[2]-11.1 so the sender can locate the bad row.
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-BAD-IDX|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r\
FT1|1|||||CG|97110|First|||50^USD\r\
FT1|2|||||CG|99213|Second|||not-a-number^USD\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_dft_p03(&m).expect_err("bad row 2");
        let msg = format!("{err}");
        assert!(
            msg.contains("FT1[2]-11.1"),
            "error should name the offending FT1 index, got: {msg}"
        );
    }

    #[test]
    fn test_parse_dft_p03_rejects_unsupported_transaction_type() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-X|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r\
FT1|1|||||PY|97110|Visit|||50^USD\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_dft_p03(&m).expect_err("PY must AE");
        assert!(format!("{err}").contains("PY"));
    }

    #[test]
    fn test_parse_dft_p03_rejects_missing_code() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-X|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r\
FT1|1|||||CG||Visit|||50^USD\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_dft_p03(&m).expect_err("missing code");
        assert!(format!("{err}").contains("FT1[1]-7"));
    }

    #[test]
    fn test_parse_dft_p03_rejects_missing_description() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-X|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r\
FT1|1|||||CG|97110||||50^USD\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_dft_p03(&m).expect_err("missing description");
        assert!(format!("{err}").contains("FT1[1]-8"));
    }

    #[test]
    fn test_parse_dft_p03_rejects_bad_amount() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-X|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r\
FT1|1|||||CG|97110|Visit|||not-a-number^USD\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_dft_p03(&m).expect_err("bad amount");
        assert!(format!("{err}").contains("FT1[1]-11.1"));
    }

    #[test]
    fn test_parse_dft_p03_rejects_bad_currency() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-X|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r\
FT1|1|||||CG|97110|Visit|||50^usdollars\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_dft_p03(&m).expect_err("bad currency");
        assert!(format!("{err}").contains("FT1[1]-11.2"));
    }

    #[test]
    fn test_parse_dft_p03_rejects_missing_ft1() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||DFT^P03|MSG-X|P|2.5\r\
EVN|P03|20260601090000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_dft_p03(&m).expect_err("missing FT1");
        assert!(format!("{err}").contains("FT1"));
    }

    #[test]
    fn test_encode_dft_p03_includes_pid_and_ft1() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let account_id = uuid::Uuid::new_v4();
        let usd = crate::models::Iso4217::new("USD").unwrap();
        let amt = crate::models::Money::new(
            rust_decimal::Decimal::from_str_exact("125.50").unwrap(),
            usd,
        );
        let charge = crate::models::billing::Charge::new(
            account_id,
            "97110".into(),
            "Therapeutic exercise".into(),
            amt,
        );
        let out = encode_dft_p03(&p, &charge, "PAS", "EMR", "MSG-OUT-DFT");
        assert!(out.contains("|DFT^P03|MSG-OUT-DFT|"));
        assert!(out.contains("\rEVN|P03|"));
        assert!(out.contains("\rPID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F"));
        // FT1 spot-check: code in FT1-7, description in FT1-8, amount + currency in FT1-11.
        assert!(out.contains("|CG|97110|Therapeutic exercise|||125.50^USD"));
        // FT1-2 carries the PAS charge id.
        assert!(out.contains(&format!("\rFT1|1|{}", charge.id)));
    }

    #[test]
    fn test_encode_dft_p03_round_trip_through_parse_dft_p03() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let account_id = uuid::Uuid::new_v4();
        let eur = crate::models::Iso4217::new("EUR").unwrap();
        let amt =
            crate::models::Money::new(rust_decimal::Decimal::from_str_exact("87.25").unwrap(), eur);
        let charge = crate::models::billing::Charge::new(
            account_id,
            "CONS-15".into(),
            "Consultation 15 min".into(),
            amt,
        );
        let wire = encode_dft_p03(&p, &charge, "PAS", "EMR", "MSG-RT-DFT");
        let parsed = parse_message(&wire).expect("parse wire");
        let dft = parse_dft_p03(&parsed).expect("parse_dft_p03");
        assert_eq!(dft.items.len(), 1);
        let item = &dft.items[0];
        assert_eq!(item.transaction_type, "CG");
        assert_eq!(item.code, "CONS-15");
        assert_eq!(item.description, "Consultation 15 min");
        assert_eq!(
            item.amount,
            rust_decimal::Decimal::from_str_exact("87.25").unwrap()
        );
        assert_eq!(item.currency, "EUR");
        assert_eq!(dft.patient.name.family, p.name.family);
    }

    // ----- MFN^M02 (v0.24) ----------------------------------------------

    const MFN_WIRE: &str = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-1|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MAD||20260601090000|STF-001\r\
STF|STF-001||Smith^John^Q||M|19700115|A\r";

    #[test]
    fn test_parse_mfn_m02_happy_path_mad() {
        let m = parse_message(MFN_WIRE).expect("parse");
        let mfn = parse_mfn_m02(&m).expect("parse_mfn_m02");
        assert_eq!(mfn.master_file_id, "PRA");
        assert_eq!(mfn.items.len(), 1);
        let item = &mfn.items[0];
        assert_eq!(item.event_code, "MAD");
        assert_eq!(item.primary_key, "STF-001");
        assert_eq!(item.name.family, "Smith");
        assert_eq!(item.name.given, vec!["John".to_string(), "Q".to_string()]);
        assert_eq!(item.gender, Gender::Male);
        assert_eq!(
            item.birth_date,
            Some(chrono::NaiveDate::from_ymd_opt(1970, 1, 15).unwrap())
        );
        assert!(item.active);
    }

    #[test]
    fn test_parse_mfn_m02_walks_multiple_mfe_stf_pairs() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-MULTI|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MAD||20260601090000|STF-001\r\
STF|STF-001||Smith^John||M|19700115|A\r\
MFE|MUP||20260601090000|STF-002\r\
STF|STF-002||Doe^Jane||F|19850301|A\r\
MFE|MDL||20260601090000|STF-003\r\
STF|STF-003||Ghost^Frank||M|19500101|I\r";
        let m = parse_message(wire).expect("parse");
        let mfn = parse_mfn_m02(&m).expect("parse_mfn_m02");
        assert_eq!(mfn.items.len(), 3);
        assert_eq!(mfn.items[0].event_code, "MAD");
        assert_eq!(mfn.items[1].event_code, "MUP");
        assert_eq!(mfn.items[2].event_code, "MDL");
        assert!(!mfn.items[2].active);
    }

    #[test]
    fn test_parse_mfn_m02_rejects_non_pra_master_file_id() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-X|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MAD||20260601090000|LOC-001\r\
STF|LOC-001||X||M||A\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_mfn_m02(&m).expect_err("LOC not supported");
        assert!(format!("{err}").contains("LOC"));
    }

    #[test]
    fn test_parse_mfn_m02_rejects_unknown_event_code() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-X|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MRP||20260601090000|STF-001\r\
STF|STF-001||Smith^John||M||A\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_mfn_m02(&m).expect_err("MRP not supported");
        assert!(format!("{err}").contains("MRP"));
    }

    #[test]
    fn test_parse_mfn_m02_rejects_mfe_without_stf() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-X|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MAD||20260601090000|STF-001\r\
MFE|MAD||20260601090000|STF-002\r\
STF|STF-002||Doe^Jane||F||A\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_mfn_m02(&m).expect_err("dangling MFE");
        assert!(format!("{err}").contains("STF"));
    }

    #[test]
    fn test_parse_mfn_m02_rejects_missing_family_name() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-X|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MAD||20260601090000|STF-001\r\
STF|STF-001||^John||M||A\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_mfn_m02(&m).expect_err("missing family");
        assert!(format!("{err}").contains("family"));
    }

    #[test]
    fn test_parse_mfn_m02_falls_back_to_stf1_when_mfe4_empty() {
        // No MFE-4; parser must use STF-1 as the primary key.
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M02|MSG-MFN-FB|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MAD||20260601090000\r\
STF|STF-FALLBACK||Smith^John||M||A\r";
        let m = parse_message(wire).expect("parse");
        let mfn = parse_mfn_m02(&m).expect("parse_mfn_m02");
        assert_eq!(mfn.items[0].primary_key, "STF-FALLBACK");
    }

    #[test]
    fn test_encode_mfn_m02_round_trip() {
        let item = MfnM02Item {
            event_code: "MAD".into(),
            primary_key: "STF-RT-01".into(),
            name: HumanName {
                use_type: Some(NameUse::Official),
                family: "Curie".into(),
                given: vec!["Marie".into(), "Salomea".into()],
                prefix: vec![],
                suffix: vec![],
            },
            gender: Gender::Female,
            birth_date: Some(chrono::NaiveDate::from_ymd_opt(1867, 11, 7).unwrap()),
            active: true,
        };
        let wire = encode_mfn_m02(std::slice::from_ref(&item), "PAS", "EMR", "MSG-RT-MFN");
        assert!(wire.contains("|MFN^M02|MSG-RT-MFN|"));
        assert!(wire.contains("\rMFI|PRA||UPD\r"));
        assert!(wire.contains("\rMFE|MAD||"));
        assert!(wire.contains("|STF-RT-01\r"));
        assert!(wire.contains("\rSTF|STF-RT-01||Curie^Marie^Salomea|"));

        let m = parse_message(&wire).expect("parse round-trip");
        let mfn = parse_mfn_m02(&m).expect("parse_mfn_m02");
        assert_eq!(mfn.items.len(), 1);
        assert_eq!(mfn.items[0].event_code, item.event_code);
        assert_eq!(mfn.items[0].primary_key, item.primary_key);
        assert_eq!(mfn.items[0].name.family, item.name.family);
        assert_eq!(mfn.items[0].name.given, item.name.given);
        assert_eq!(mfn.items[0].gender, item.gender);
        assert_eq!(mfn.items[0].birth_date, item.birth_date);
        assert_eq!(mfn.items[0].active, item.active);
    }

    // ----- MFN^M05 (v0.26) ----------------------------------------------

    const MFN_M05_WIRE: &str = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-M05|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MAD||20260601090000|BED-OUT-01\r\
LOC|ROOM-101^^BED-OUT-01|Bed 1\r";

    #[test]
    fn test_parse_mfn_m05_happy_path_mad() {
        let m = parse_message(MFN_M05_WIRE).expect("parse");
        let mfn = parse_mfn_m05(&m).expect("parse_mfn_m05");
        assert_eq!(mfn.master_file_id, "LOC");
        assert_eq!(mfn.items.len(), 1);
        let item = &mfn.items[0];
        assert_eq!(item.event_code, "MAD");
        assert_eq!(item.bed_code, "BED-OUT-01");
        assert_eq!(item.room_code.as_deref(), Some("ROOM-101"));
        assert_eq!(item.name.as_deref(), Some("Bed 1"));
    }

    #[test]
    fn test_parse_mfn_m05_mad_requires_room_code() {
        // LOC-1.1 empty → MAD must fail.
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-X|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MAD||20260601090000|BED-001\r\
LOC|^^BED-001|Bed 1\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_mfn_m05(&m).expect_err("MAD must require room code");
        assert!(format!("{err}").contains("LOC-1.1"));
    }

    #[test]
    fn test_parse_mfn_m05_mad_requires_name() {
        // LOC-2 empty → MAD must fail.
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-X|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MAD||20260601090000|BED-001\r\
LOC|ROOM-101^^BED-001\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_mfn_m05(&m).expect_err("MAD must require name");
        assert!(format!("{err}").contains("LOC-2"));
    }

    #[test]
    fn test_parse_mfn_m05_mup_and_mdl_dont_require_name_or_room() {
        // MUP and MDL only need the bed code (LOC-1.3). Room and name
        // are optional / ignored.
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-X|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MUP||20260601090000|BED-001\r\
LOC|^^BED-001\r\
MFE|MDL||20260601090000|BED-002\r\
LOC|^^BED-002\r";
        let m = parse_message(wire).expect("parse");
        let mfn = parse_mfn_m05(&m).expect("parse_mfn_m05");
        assert_eq!(mfn.items.len(), 2);
        assert_eq!(mfn.items[0].event_code, "MUP");
        assert_eq!(mfn.items[1].event_code, "MDL");
    }

    #[test]
    fn test_parse_mfn_m05_rejects_non_loc_master_file() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-X|P|2.5\r\
MFI|PRA||UPD\r\
MFE|MAD||20260601090000|BED-001\r\
LOC|ROOM-101^^BED-001|Bed 1\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_mfn_m05(&m).expect_err("PRA wrong master file id for M05");
        assert!(format!("{err}").contains("PRA"));
    }

    #[test]
    fn test_parse_mfn_m05_falls_back_to_mfe4_for_bed_code() {
        // LOC-1 entirely empty → parser must fall back to MFE-4 for
        // the bed code.
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||MFN^M05|MSG-X|P|2.5\r\
MFI|LOC||UPD\r\
MFE|MUP||20260601090000|BED-FB\r\
LOC||New name\r";
        let m = parse_message(wire).expect("parse");
        let mfn = parse_mfn_m05(&m).expect("parse_mfn_m05");
        assert_eq!(mfn.items[0].bed_code, "BED-FB");
        assert_eq!(mfn.items[0].name.as_deref(), Some("New name"));
    }

    #[test]
    fn test_encode_mfn_m05_round_trip() {
        let item = MfnM05Item {
            event_code: "MAD".into(),
            bed_code: "BED-RT-1".into(),
            room_code: Some("ROOM-RT-1".into()),
            name: Some("Bed One".into()),
        };
        let wire = encode_mfn_m05(std::slice::from_ref(&item), "PAS", "EMR", "MSG-RT-M05");
        assert!(wire.contains("|MFN^M05|MSG-RT-M05|"));
        assert!(wire.contains("\rMFI|LOC||UPD\r"));
        assert!(wire.contains("\rMFE|MAD||"));
        assert!(wire.contains("|BED-RT-1\r"));
        assert!(wire.contains("\rLOC|ROOM-RT-1^^BED-RT-1|Bed One\r"));

        let m = parse_message(&wire).expect("parse round-trip");
        let mfn = parse_mfn_m05(&m).expect("parse_mfn_m05");
        assert_eq!(mfn.items.len(), 1);
        assert_eq!(mfn.items[0].event_code, item.event_code);
        assert_eq!(mfn.items[0].bed_code, item.bed_code);
        assert_eq!(mfn.items[0].room_code, item.room_code);
        assert_eq!(mfn.items[0].name, item.name);
    }

    // ----- A40 (merge patient) ------------------------------------------

    #[test]
    fn test_encode_adt_a40_includes_pid_and_mrg() {
        let m = parse_message(A01).expect("parse");
        let survivor = patient_from_pid(m.segment("PID").unwrap()).expect("survivor");
        // Build a second patient with a known MRN so we can assert the
        // MRG segment carries it.
        let mut source = survivor.clone();
        source.id = uuid::Uuid::new_v4();
        source.identifiers = vec![crate::models::identifier::Identifier {
            use_type: Some(crate::models::identifier::IdentifierUse::Official),
            identifier_type: IdentifierType::MRN,
            system: "urn:oid:facility:FAC".to_string(),
            value: "MRN-SOURCE-9".to_string(),
            assigner: None,
        }];
        let out = encode_adt_a40(&survivor, &source, "PAS", "EMR", "MSG-OUT-A40");
        assert!(out.contains("|ADT^A40|MSG-OUT-A40|"));
        assert!(out.contains("\rEVN|A40|"));
        // Survivor PID is the MRN-001 row from A01 fixture.
        assert!(out.contains("\rPID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F"));
        // MRG-1.1 carries the source MRN.
        assert!(out.contains("\rMRG|MRN-SOURCE-9^^^FAC^MR"));
    }

    #[test]
    fn test_parse_merge_source_mrn_extracts_mrg_1_1() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A40|MSG-A40|P|2.5\r\
EVN|A40|20260601090000\r\
PID|1||MRN-SURV^^^FAC^MR||Doe^Jane^Marie||19900115|F\r\
MRG|MRN-PRIOR^^^FAC^MR\r";
        let m = parse_message(wire).expect("parse");
        assert_eq!(parse_merge_source_mrn(&m).unwrap(), "MRN-PRIOR");
    }

    #[test]
    fn test_parse_merge_source_mrn_rejects_missing_mrg() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A40|MSG-A40|P|2.5\r\
EVN|A40|20260601090000\r\
PID|1||MRN-SURV^^^FAC^MR||Doe^Jane^Marie||19900115|F\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_merge_source_mrn(&m).expect_err("must fail without MRG");
        assert!(format!("{err}").contains("MRG"));
    }

    #[test]
    fn test_parse_merge_source_mrn_rejects_empty_mrg1() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||ADT^A40|MSG-A40|P|2.5\r\
EVN|A40|20260601090000\r\
PID|1||MRN-SURV^^^FAC^MR||Doe^Jane^Marie||19900115|F\r\
MRG|\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_merge_source_mrn(&m).expect_err("must fail with empty MRG-1");
        assert!(format!("{err}").contains("MRN"));
    }

    #[test]
    fn test_encode_adt_a40_round_trip_through_parse_helpers() {
        let m = parse_message(A01).expect("parse");
        let survivor = patient_from_pid(m.segment("PID").unwrap()).expect("survivor");
        let mut source = survivor.clone();
        source.id = uuid::Uuid::new_v4();
        source.identifiers = vec![crate::models::identifier::Identifier {
            use_type: Some(crate::models::identifier::IdentifierUse::Official),
            identifier_type: IdentifierType::MRN,
            system: "urn:oid:facility:FAC".to_string(),
            value: "MRN-RT-99".to_string(),
            assigner: None,
        }];
        let wire = encode_adt_a40(&survivor, &source, "PAS", "EMR", "MSG-RT-A40");
        let parsed = parse_message(&wire).expect("parse round-trip");
        assert_eq!(
            message_type(&parsed),
            ("ADT".to_string(), "A40".to_string())
        );
        assert_eq!(parse_merge_source_mrn(&parsed).unwrap(), "MRN-RT-99");
        // Survivor PID must still round-trip back to the same MRN as the
        // input survivor.
        let back = patient_from_pid(parsed.segment("PID").unwrap()).expect("survivor back");
        assert_eq!(back.name.family, survivor.name.family);
    }

    #[test]
    fn test_encode_adt_a28_includes_msh_and_pid() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let out = encode_adt_a28(&p, "PAS", "EMR", "MSG-OUT-001");
        assert!(out.contains("MSH|^~\\&|PAS|FAC|EMR|FAC|"));
        assert!(out.contains("|ADT^A28|MSG-OUT-001|"));
        assert!(out.contains("\rEVN|A28|"));
        assert!(out.contains("\rPID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F"));
    }

    // ----- SIU (v0.16) ----------------------------------------------------

    const S12_WIRE: &str = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S12|MSG-S12-001|P|2.5\r\
SCH|PLACER-1||||||routine follow-up||30|min|20260605143000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r";

    const S15_WIRE: &str = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S15|MSG-S15-001|P|2.5\r\
SCH|PLACER-1|550e8400-e29b-41d4-a716-446655440000|||||patient request\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r";

    #[test]
    fn test_message_type_siu_s12() {
        let m = parse_message(S12_WIRE).expect("parse");
        assert_eq!(message_type(&m), ("SIU".to_string(), "S12".to_string()));
    }

    #[test]
    fn test_parse_siu_s12_extracts_patient_and_appointment() {
        let m = parse_message(S12_WIRE).expect("parse");
        let siu = parse_siu(&m).expect("parse_siu");
        assert_eq!(siu.trigger, "S12");
        assert_eq!(siu.patient.name.family, "Doe");
        assert_eq!(siu.placer_appointment_id.as_deref(), Some("PLACER-1"));
        assert!(siu.filler_appointment_id.is_none());
        let start = siu.start_datetime.expect("start required for S12");
        let end = siu.end_datetime.expect("end required for S12");
        assert_eq!(
            start,
            chrono::NaiveDate::from_ymd_opt(2026, 6, 5)
                .unwrap()
                .and_hms_opt(14, 30, 0)
                .unwrap()
                .and_utc()
        );
        assert_eq!((end - start).num_minutes(), 30);
        assert_eq!(siu.reason.as_deref(), Some("routine follow-up"));
    }

    #[test]
    fn test_parse_siu_s12_defaults_duration_when_sch9_missing() {
        // SCH-9 empty → fall back to 30 min default.
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S12|MSG-X|P|2.5\r\
SCH|PLACER-1||||||consult||||20260605143000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r";
        let m = parse_message(wire).expect("parse");
        let siu = parse_siu(&m).expect("parse_siu");
        let s = siu.start_datetime.unwrap();
        let e = siu.end_datetime.unwrap();
        assert_eq!((e - s).num_minutes(), 30);
    }

    #[test]
    fn test_parse_siu_s12_rejects_missing_sch11() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S12|MSG-X|P|2.5\r\
SCH|PLACER-1||||||consult||30|min|\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_siu(&m).expect_err("must fail without SCH-11");
        assert!(format!("{err}").contains("SCH-11"));
    }

    #[test]
    fn test_parse_siu_s15_carries_filler_uuid() {
        let m = parse_message(S15_WIRE).expect("parse");
        let siu = parse_siu(&m).expect("parse_siu");
        assert_eq!(siu.trigger, "S15");
        assert_eq!(
            siu.filler_appointment_id.as_deref(),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
        // S15 ignores SCH-11; start/end are None.
        assert!(siu.start_datetime.is_none());
        assert!(siu.end_datetime.is_none());
        assert_eq!(siu.reason.as_deref(), Some("patient request"));
    }

    #[test]
    fn test_parse_siu_s13_requires_filler_and_sch11() {
        // S13 needs both SCH-2 (which appointment to reschedule) and
        // SCH-11 (the new start). Missing SCH-2 → AE.
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S13|MSG-X|P|2.5\r\
SCH|PLACER-1||||||consult||30|min|20260605143000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_siu(&m).expect_err("S13 must require filler");
        assert!(format!("{err}").contains("SCH-2"));
    }

    #[test]
    fn test_parse_siu_s13_extracts_new_time_window() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S13|MSG-X|P|2.5\r\
SCH|PLACER-1|550e8400-e29b-41d4-a716-446655440000|||||||60|min|20270605160000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r";
        let m = parse_message(wire).expect("parse");
        let siu = parse_siu(&m).expect("parse_siu");
        assert_eq!(siu.trigger, "S13");
        assert_eq!(
            siu.filler_appointment_id.as_deref(),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
        let start = siu.start_datetime.expect("S13 carries start");
        let end = siu.end_datetime.expect("S13 carries end");
        assert_eq!(start.format("%Y%m%d%H%M%S").to_string(), "20270605160000");
        assert_eq!((end - start).num_minutes(), 60);
    }

    #[test]
    fn test_parse_siu_s14_carries_reason_no_time() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S14|MSG-X|P|2.5\r\
SCH|PLACER-1|550e8400-e29b-41d4-a716-446655440000|||||updated reason text\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r";
        let m = parse_message(wire).expect("parse");
        let siu = parse_siu(&m).expect("parse_siu");
        assert_eq!(siu.trigger, "S14");
        assert_eq!(siu.reason.as_deref(), Some("updated reason text"));
        // S14 ignores SCH-11 — no start/end populated.
        assert!(siu.start_datetime.is_none());
        assert!(siu.end_datetime.is_none());
    }

    #[test]
    fn test_parse_siu_s14_requires_filler() {
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S14|MSG-X|P|2.5\r\
SCH|PLACER-1\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_siu(&m).expect_err("S14 must require filler");
        assert!(format!("{err}").contains("SCH-2"));
    }

    #[test]
    fn test_parse_siu_rejects_unknown_trigger() {
        // S17 (block schedule) is not in our supported set.
        let wire = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260601090000||SIU^S17|MSG-X|P|2.5\r\
SCH|PLACER-1\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F\r";
        let m = parse_message(wire).expect("parse");
        let err = parse_siu(&m).expect_err("S17 not supported");
        assert!(format!("{err}").contains("S17"));
    }

    #[test]
    fn test_encode_siu_s13_includes_new_time_and_filler() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let appt_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let start = chrono::NaiveDate::from_ymd_opt(2027, 6, 5)
            .unwrap()
            .and_hms_opt(16, 0, 0)
            .unwrap()
            .and_utc();
        let end = start + chrono::Duration::minutes(60);
        let out = encode_siu_s13(
            &p,
            appt_id,
            Some("PLACER-RT"),
            start,
            end,
            Some("rescheduled"),
            "PAS",
            "EMR",
            "MSG-OUT-S13",
        );
        assert!(out.contains("|SIU^S13|MSG-OUT-S13|"));
        assert!(out.contains(
            "\rSCH|PLACER-RT|550e8400-e29b-41d4-a716-446655440000|||||rescheduled||60|min|20270605160000"
        ));
    }

    #[test]
    fn test_encode_siu_s14_carries_reason_omits_time() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let appt_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let out = encode_siu_s14(
            &p,
            appt_id,
            Some("PLACER-RT"),
            Some("updated reason"),
            "PAS",
            "EMR",
            "MSG-OUT-S14",
        );
        assert!(out.contains("|SIU^S14|MSG-OUT-S14|"));
        // No duration, no start; reason present in SCH-7.
        assert!(out.contains(
            "\rSCH|PLACER-RT|550e8400-e29b-41d4-a716-446655440000|||||updated reason||||\r"
        ));
    }

    #[test]
    fn test_encode_siu_s13_round_trip() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let appt_id = uuid::Uuid::new_v4();
        let start = chrono::NaiveDate::from_ymd_opt(2027, 8, 1)
            .unwrap()
            .and_hms_opt(11, 0, 0)
            .unwrap()
            .and_utc();
        let end = start + chrono::Duration::minutes(45);
        let wire = encode_siu_s13(
            &p,
            appt_id,
            Some("PLACER-RT"),
            start,
            end,
            Some("rescheduled"),
            "PAS",
            "EMR",
            "MSG-RT-S13",
        );
        let parsed = parse_message(&wire).expect("parse wire");
        let siu = parse_siu(&parsed).expect("parse_siu");
        assert_eq!(siu.trigger, "S13");
        assert_eq!(
            siu.filler_appointment_id.as_deref(),
            Some(&appt_id.to_string()[..])
        );
        assert_eq!(siu.start_datetime, Some(start));
        assert_eq!(siu.end_datetime, Some(end));
        assert_eq!(siu.reason.as_deref(), Some("rescheduled"));
    }

    #[test]
    fn test_parse_siu_rejects_non_siu_message() {
        let m = parse_message(A01).expect("parse");
        let err = parse_siu(&m).expect_err("ADT must be rejected by parse_siu");
        assert!(format!("{err}").contains("expected SIU"));
    }

    #[test]
    fn test_encode_siu_s12_includes_sch_and_filler_uuid() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let appt_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let start = chrono::NaiveDate::from_ymd_opt(2026, 6, 5)
            .unwrap()
            .and_hms_opt(14, 30, 0)
            .unwrap()
            .and_utc();
        let end = start + chrono::Duration::minutes(45);
        let out = encode_siu_s12(
            &p,
            appt_id,
            Some("PLACER-9"),
            start,
            end,
            Some("follow-up"),
            "PAS",
            "EMR",
            "MSG-OUT-S12",
        );
        assert!(out.contains("|SIU^S12|MSG-OUT-S12|"));
        // SCH-1 placer, SCH-2 filler (PAS uuid), SCH-7 reason, SCH-9/10 duration, SCH-11 start.
        assert!(
            out.contains(
                "\rSCH|PLACER-9|550e8400-e29b-41d4-a716-446655440000|||||follow-up||45|min|20260605143000"
            )
        );
        // PID still present at the end.
        assert!(out.contains("\rPID|1||MRN-001"));
    }

    #[test]
    fn test_encode_siu_s15_omits_start_and_duration() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let appt_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let out = encode_siu_s15(
            &p,
            appt_id,
            Some("PLACER-9"),
            Some("patient request"),
            "PAS",
            "EMR",
            "MSG-OUT-S15",
        );
        assert!(out.contains("|SIU^S15|MSG-OUT-S15|"));
        // No duration, no start datetime; reason is still present in SCH-7.
        assert!(out.contains(
            "\rSCH|PLACER-9|550e8400-e29b-41d4-a716-446655440000|||||patient request||||\r"
        ));
    }

    #[test]
    fn test_encode_siu_s12_round_trip_through_parse_siu() {
        let m = parse_message(A01).expect("parse");
        let p = patient_from_pid(m.segment("PID").unwrap()).expect("map");
        let appt_id = uuid::Uuid::new_v4();
        let start = chrono::NaiveDate::from_ymd_opt(2026, 7, 1)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_utc();
        let end = start + chrono::Duration::minutes(20);
        let wire = encode_siu_s12(
            &p,
            appt_id,
            Some("PLACER-RT"),
            start,
            end,
            Some("intake"),
            "PAS",
            "EMR",
            "MSG-RT",
        );
        let parsed = parse_message(&wire).expect("parse wire");
        let siu = parse_siu(&parsed).expect("parse_siu");
        assert_eq!(siu.trigger, "S12");
        assert_eq!(siu.placer_appointment_id.as_deref(), Some("PLACER-RT"));
        assert_eq!(
            siu.filler_appointment_id.as_deref(),
            Some(&appt_id.to_string()[..])
        );
        assert_eq!(siu.start_datetime, Some(start));
        assert_eq!(siu.end_datetime, Some(end));
        assert_eq!(siu.reason.as_deref(), Some("intake"));
    }

    #[test]
    fn test_parse_v2_datetime_accepts_short_form() {
        // 12-char form (no seconds).
        let dt = parse_v2_datetime("202606051430").expect("parse 12-char");
        assert_eq!(
            dt,
            chrono::NaiveDate::from_ymd_opt(2026, 6, 5)
                .unwrap()
                .and_hms_opt(14, 30, 0)
                .unwrap()
                .and_utc()
        );
    }

    #[test]
    fn test_parse_v2_datetime_strips_timezone_offset() {
        // HL7 v2 allows `+0500` / `-0530` suffixes; v0.16 treats them as UTC.
        let dt = parse_v2_datetime("20260605143000+0500").expect("parse with offset");
        assert_eq!(
            dt,
            chrono::NaiveDate::from_ymd_opt(2026, 6, 5)
                .unwrap()
                .and_hms_opt(14, 30, 0)
                .unwrap()
                .and_utc()
        );
    }
}
