//! Hand-rolled FHIR R5 resource types and bidirectional conversion to and
//! from the PAS domain models.
//!
//! v0.1 scope: enough fields to round-trip the most commonly exchanged
//! patient-administration data. Anything that does not appear on the PAS
//! domain models (FHIR `meta`, `text`, extensions, contained resources, …) is
//! intentionally omitted.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::appointment::{Appointment, AppointmentStatus};
use crate::models::encounter::{Encounter, EncounterClass, EncounterStatus};
use crate::models::facility::{Bed, BedStatus};
use crate::models::identifier::{Identifier, IdentifierType, IdentifierUse};
use crate::models::patient::{HumanName, Patient};
use crate::models::practitioner::Practitioner;
use crate::models::schedule::{Schedule, ScheduleOwner, Slot, SlotStatus};
use crate::models::{
    Address, AddressUse, ContactPoint, ContactPointSystem, ContactPointUse, Gender, NameUse,
};
use crate::{Error, Result};

// ---------------------------------------------------------------------------
// Shared building blocks
// ---------------------------------------------------------------------------

/// FHIR `HumanName`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirHumanName {
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    pub use_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub given: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefix: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suffix: Vec<String>,
}

/// FHIR `Address`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirAddress {
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    pub use_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub line: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

/// FHIR `ContactPoint`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirContactPoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    pub use_field: Option<String>,
}

/// FHIR `Identifier` (minimal: `system`, `value`, `use`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirIdentifier {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    pub use_field: Option<String>,
}

/// FHIR `Reference` — minimal subset (just the `reference` string).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FhirReference {
    pub reference: String,
}

/// FHIR `Coding`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirCoding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

/// FHIR `Period` (UTC ISO 8601 strings).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirPeriod {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
}

/// FHIR `CodeableConcept` — for v0.1 we only carry the free-text label.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirCodeableConcept {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

// ---------------------------------------------------------------------------
// String helpers for enum mapping
// ---------------------------------------------------------------------------

fn name_use_to_str(u: NameUse) -> &'static str {
    match u {
        NameUse::Usual => "usual",
        NameUse::Official => "official",
        NameUse::Temp => "temp",
        NameUse::Nickname => "nickname",
        NameUse::Anonymous => "anonymous",
        NameUse::Old => "old",
        NameUse::Maiden => "maiden",
    }
}

fn name_use_from_str(s: &str) -> Option<NameUse> {
    match s {
        "usual" => Some(NameUse::Usual),
        "official" => Some(NameUse::Official),
        "temp" => Some(NameUse::Temp),
        "nickname" => Some(NameUse::Nickname),
        "anonymous" => Some(NameUse::Anonymous),
        "old" => Some(NameUse::Old),
        "maiden" => Some(NameUse::Maiden),
        _ => None,
    }
}

fn address_use_to_str(u: AddressUse) -> &'static str {
    match u {
        AddressUse::Home => "home",
        AddressUse::Work => "work",
        AddressUse::Temp => "temp",
        AddressUse::Old => "old",
        AddressUse::Billing => "billing",
    }
}

fn address_use_from_str(s: &str) -> Option<AddressUse> {
    match s {
        "home" => Some(AddressUse::Home),
        "work" => Some(AddressUse::Work),
        "temp" => Some(AddressUse::Temp),
        "old" => Some(AddressUse::Old),
        "billing" => Some(AddressUse::Billing),
        _ => None,
    }
}

fn contact_system_to_str(s: ContactPointSystem) -> &'static str {
    match s {
        ContactPointSystem::Phone => "phone",
        ContactPointSystem::Fax => "fax",
        ContactPointSystem::Email => "email",
        ContactPointSystem::Pager => "pager",
        ContactPointSystem::Url => "url",
        ContactPointSystem::Sms => "sms",
        ContactPointSystem::Other => "other",
    }
}

fn contact_system_from_str(s: &str) -> Option<ContactPointSystem> {
    match s {
        "phone" => Some(ContactPointSystem::Phone),
        "fax" => Some(ContactPointSystem::Fax),
        "email" => Some(ContactPointSystem::Email),
        "pager" => Some(ContactPointSystem::Pager),
        "url" => Some(ContactPointSystem::Url),
        "sms" => Some(ContactPointSystem::Sms),
        "other" => Some(ContactPointSystem::Other),
        _ => None,
    }
}

fn contact_use_to_str(u: ContactPointUse) -> &'static str {
    match u {
        ContactPointUse::Home => "home",
        ContactPointUse::Work => "work",
        ContactPointUse::Temp => "temp",
        ContactPointUse::Old => "old",
        ContactPointUse::Mobile => "mobile",
    }
}

fn contact_use_from_str(s: &str) -> Option<ContactPointUse> {
    match s {
        "home" => Some(ContactPointUse::Home),
        "work" => Some(ContactPointUse::Work),
        "temp" => Some(ContactPointUse::Temp),
        "old" => Some(ContactPointUse::Old),
        "mobile" => Some(ContactPointUse::Mobile),
        _ => None,
    }
}

fn gender_to_str(g: Gender) -> &'static str {
    match g {
        Gender::Male => "male",
        Gender::Female => "female",
        Gender::Other => "other",
        Gender::Unknown => "unknown",
    }
}

fn gender_from_str(s: &str) -> Gender {
    match s {
        "male" => Gender::Male,
        "female" => Gender::Female,
        "other" => Gender::Other,
        _ => Gender::Unknown,
    }
}

fn identifier_use_to_str(u: IdentifierUse) -> &'static str {
    match u {
        IdentifierUse::Usual => "usual",
        IdentifierUse::Official => "official",
        IdentifierUse::Temp => "temp",
        IdentifierUse::Secondary => "secondary",
        IdentifierUse::Old => "old",
    }
}

fn identifier_use_from_str(s: &str) -> Option<IdentifierUse> {
    match s {
        "usual" => Some(IdentifierUse::Usual),
        "official" => Some(IdentifierUse::Official),
        "temp" => Some(IdentifierUse::Temp),
        "secondary" => Some(IdentifierUse::Secondary),
        "old" => Some(IdentifierUse::Old),
        _ => None,
    }
}

fn encounter_status_to_str(s: EncounterStatus) -> &'static str {
    match s {
        EncounterStatus::Planned => "planned",
        // FHIR has no "arrived" for Encounter v5 — v0.1 keeps the PAS label so
        // round-tripping is lossless.
        EncounterStatus::Arrived => "arrived",
        EncounterStatus::InProgress => "in-progress",
        EncounterStatus::OnLeave => "onleave",
        EncounterStatus::Finished => "finished",
        EncounterStatus::Cancelled => "cancelled",
    }
}

fn encounter_status_from_str(s: &str) -> Result<EncounterStatus> {
    match s {
        "planned" => Ok(EncounterStatus::Planned),
        "arrived" => Ok(EncounterStatus::Arrived),
        "in-progress" => Ok(EncounterStatus::InProgress),
        "onleave" => Ok(EncounterStatus::OnLeave),
        "finished" => Ok(EncounterStatus::Finished),
        "cancelled" => Ok(EncounterStatus::Cancelled),
        other => Err(Error::Fhir(format!("unknown encounter status: {other}"))),
    }
}

fn encounter_class_to_code(c: EncounterClass) -> (&'static str, &'static str) {
    // (code, display)
    match c {
        EncounterClass::Outpatient => ("AMB", "ambulatory"),
        EncounterClass::Inpatient => ("IMP", "inpatient encounter"),
        EncounterClass::Emergency => ("EMER", "emergency"),
        EncounterClass::DayCase => ("SS", "short stay"),
        EncounterClass::HomeCare => ("HH", "home health"),
        EncounterClass::Virtual => ("VR", "virtual"),
    }
}

fn encounter_class_from_code(code: &str) -> EncounterClass {
    match code {
        "AMB" => EncounterClass::Outpatient,
        "IMP" => EncounterClass::Inpatient,
        "EMER" => EncounterClass::Emergency,
        "SS" => EncounterClass::DayCase,
        "HH" => EncounterClass::HomeCare,
        "VR" => EncounterClass::Virtual,
        _ => EncounterClass::Outpatient,
    }
}

fn appointment_status_to_str(s: AppointmentStatus) -> &'static str {
    match s {
        AppointmentStatus::Proposed => "proposed",
        AppointmentStatus::Booked => "booked",
        AppointmentStatus::Arrived => "arrived",
        AppointmentStatus::Fulfilled => "fulfilled",
        AppointmentStatus::Cancelled => "cancelled",
        AppointmentStatus::NoShow => "noshow",
    }
}

fn appointment_status_from_str(s: &str) -> Result<AppointmentStatus> {
    match s {
        "proposed" => Ok(AppointmentStatus::Proposed),
        "booked" => Ok(AppointmentStatus::Booked),
        "arrived" => Ok(AppointmentStatus::Arrived),
        "fulfilled" => Ok(AppointmentStatus::Fulfilled),
        "cancelled" => Ok(AppointmentStatus::Cancelled),
        "noshow" => Ok(AppointmentStatus::NoShow),
        other => Err(Error::Fhir(format!("unknown appointment status: {other}"))),
    }
}

fn slot_status_to_str(s: SlotStatus) -> &'static str {
    match s {
        SlotStatus::Free => "free",
        SlotStatus::Busy => "busy",
        SlotStatus::BlockedOut => "blocked-out",
    }
}

fn slot_status_from_str(s: &str) -> Result<SlotStatus> {
    match s {
        "free" => Ok(SlotStatus::Free),
        "busy" => Ok(SlotStatus::Busy),
        "blocked-out" => Ok(SlotStatus::BlockedOut),
        other => Err(Error::Fhir(format!("unknown slot status: {other}"))),
    }
}

fn bed_status_to_location_status(b: BedStatus) -> &'static str {
    match b {
        BedStatus::Available | BedStatus::Reserved | BedStatus::Cleaning | BedStatus::Occupied => {
            "active"
        }
        BedStatus::OutOfService => "inactive",
    }
}

fn format_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn parse_datetime(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Fhir(format!("bad datetime '{s}': {e}")))
}

// ---------------------------------------------------------------------------
// HumanName / Address / ContactPoint / Identifier ↔ domain
// ---------------------------------------------------------------------------

impl From<&HumanName> for FhirHumanName {
    fn from(n: &HumanName) -> Self {
        Self {
            use_field: n.use_type.map(|u| name_use_to_str(u).to_string()),
            family: if n.family.is_empty() {
                None
            } else {
                Some(n.family.clone())
            },
            given: n.given.clone(),
            prefix: n.prefix.clone(),
            suffix: n.suffix.clone(),
        }
    }
}

impl From<&FhirHumanName> for HumanName {
    fn from(n: &FhirHumanName) -> Self {
        Self {
            use_type: n.use_field.as_deref().and_then(name_use_from_str),
            family: n.family.clone().unwrap_or_default(),
            given: n.given.clone(),
            prefix: n.prefix.clone(),
            suffix: n.suffix.clone(),
        }
    }
}

impl From<&Address> for FhirAddress {
    fn from(a: &Address) -> Self {
        let mut line = Vec::new();
        if let Some(ref l1) = a.line1 {
            line.push(l1.clone());
        }
        if let Some(ref l2) = a.line2 {
            line.push(l2.clone());
        }
        Self {
            use_field: a.use_type.map(|u| address_use_to_str(u).to_string()),
            line,
            city: a.city.clone(),
            state: a.state.clone(),
            postal_code: a.postal_code.clone(),
            country: a.country.clone(),
        }
    }
}

impl From<&FhirAddress> for Address {
    fn from(a: &FhirAddress) -> Self {
        Self {
            use_type: a.use_field.as_deref().and_then(address_use_from_str),
            line1: a.line.first().cloned(),
            line2: a.line.get(1).cloned(),
            city: a.city.clone(),
            state: a.state.clone(),
            postal_code: a.postal_code.clone(),
            country: a.country.clone(),
        }
    }
}

impl From<&ContactPoint> for FhirContactPoint {
    fn from(cp: &ContactPoint) -> Self {
        Self {
            system: Some(contact_system_to_str(cp.system).to_string()),
            value: Some(cp.value.clone()),
            use_field: cp.use_type.map(|u| contact_use_to_str(u).to_string()),
        }
    }
}

impl FhirContactPoint {
    fn into_domain(self) -> Option<ContactPoint> {
        let system = self.system.as_deref().and_then(contact_system_from_str)?;
        let value = self.value?;
        Some(ContactPoint {
            system,
            value,
            use_type: self.use_field.as_deref().and_then(contact_use_from_str),
        })
    }
}

impl From<&Identifier> for FhirIdentifier {
    fn from(i: &Identifier) -> Self {
        Self {
            system: Some(i.system.clone()),
            value: Some(i.value.clone()),
            use_field: i.use_type.map(|u| identifier_use_to_str(u).to_string()),
        }
    }
}

impl FhirIdentifier {
    fn into_domain(self) -> Option<Identifier> {
        let system = self.system?;
        let value = self.value?;
        // We don't know which IdentifierType the wire system corresponds to.
        // For v0.1 we map to `Other` and preserve `system`/`value` verbatim.
        Some(Identifier {
            use_type: self.use_field.as_deref().and_then(identifier_use_from_str),
            identifier_type: IdentifierType::Other,
            system,
            value,
            assigner: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Patient
// ---------------------------------------------------------------------------

/// FHIR R5 `Patient` (minimal subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirPatient {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name: Vec<FhirHumanName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gender: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub telecom: Vec<FhirContactPoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub address: Vec<FhirAddress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identifier: Vec<FhirIdentifier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deceased_boolean: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deceased_date_time: Option<String>,
    /// Merge tombstone link (v0.11). When the domain row is a
    /// tombstone (`Patient.replaced_by = Some(target)`), this carries
    /// one entry of `type = "replaced-by"` pointing to `Patient/{target}`.
    /// Live patients serialize an empty array (which is then skipped
    /// by `skip_serializing_if`). The inverse direction (a survivor
    /// listing what it replaces) is **not** auto-emitted here — that
    /// would require an extra DB query per FHIR read; callers who want
    /// it use `GET /api/patients/{id}/replaces`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link: Vec<FhirPatientLink>,
}

/// One entry in `Patient.link`. The FHIR R5 spec defines several
/// `type` codes (`replaced-by`, `replaces`, `refer`, `seealso`); the
/// PAS only ever emits `replaced-by` on tombstones today.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirPatientLink {
    pub other: FhirReference,
    /// FHIR `Patient.link.type` value. The PAS uses `"replaced-by"`
    /// (kebab-case, per FHIR convention).
    #[serde(rename = "type")]
    pub link_type: String,
}

impl From<&Patient> for FhirPatient {
    fn from(p: &Patient) -> Self {
        let mut name = vec![FhirHumanName::from(&p.name)];
        for additional in &p.additional_names {
            name.push(FhirHumanName::from(additional));
        }
        let (deceased_boolean, deceased_date_time) = match (p.deceased, p.deceased_datetime) {
            (true, Some(dt)) => (None, Some(dt.to_rfc3339())),
            (true, None) => (Some(true), None),
            (false, _) => (None, None),
        };
        // v0.11: when this row is a merge tombstone, surface the link
        // so FHIR consumers can chase the survivor without an extra
        // round-trip to /api/patients/{id}/replaces.
        let link = match p.replaced_by {
            Some(target) => vec![FhirPatientLink {
                other: FhirReference {
                    reference: format!("Patient/{target}"),
                },
                link_type: "replaced-by".to_string(),
            }],
            None => Vec::new(),
        };
        Self {
            resource_type: "Patient".into(),
            id: Some(p.id.to_string()),
            active: p.active,
            name,
            gender: Some(gender_to_str(p.gender).to_string()),
            birth_date: p.birth_date.map(format_date),
            telecom: p.telecom.iter().map(FhirContactPoint::from).collect(),
            address: p.addresses.iter().map(FhirAddress::from).collect(),
            identifier: p.identifiers.iter().map(FhirIdentifier::from).collect(),
            deceased_boolean,
            deceased_date_time,
            link,
        }
    }
}

impl FhirPatient {
    /// Convert a wire `FhirPatient` into a domain `Patient`. Returns a FHIR
    /// error if the resource is missing required pieces (e.g. at least one
    /// name).
    pub fn into_domain(self) -> Result<Patient> {
        let id = match self.id.as_deref() {
            Some(s) => Uuid::parse_str(s).map_err(|e| Error::Fhir(format!("bad id: {e}")))?,
            None => Uuid::new_v4(),
        };
        let mut names_iter = self.name.into_iter();
        let primary = names_iter
            .next()
            .ok_or_else(|| Error::Fhir("Patient.name must not be empty".into()))?;
        let primary_name = HumanName::from(&primary);
        let additional_names: Vec<HumanName> = names_iter.map(|n| HumanName::from(&n)).collect();

        let gender = self
            .gender
            .as_deref()
            .map(gender_from_str)
            .unwrap_or(Gender::Unknown);

        let birth_date = self.birth_date.as_deref().and_then(parse_date);

        let telecom = self
            .telecom
            .into_iter()
            .filter_map(|cp| cp.into_domain())
            .collect();

        let addresses = self.address.iter().map(Address::from).collect();

        let identifiers = self
            .identifier
            .into_iter()
            .filter_map(|i| i.into_domain())
            .collect();

        let (deceased, deceased_datetime) = match (self.deceased_boolean, self.deceased_date_time) {
            (_, Some(s)) => (true, Some(parse_datetime(&s)?)),
            (Some(true), None) => (true, None),
            _ => (false, None),
        };

        let now = Utc::now();
        Ok(Patient {
            id,
            mpi_id: None,
            identifiers,
            active: self.active,
            name: primary_name,
            additional_names,
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
}

// ---------------------------------------------------------------------------
// Encounter
// ---------------------------------------------------------------------------

/// FHIR R5 `Encounter` (minimal subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirEncounter {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub status: String,
    pub class: FhirCoding,
    pub subject: FhirReference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<FhirPeriod>,
}

impl From<&Encounter> for FhirEncounter {
    fn from(e: &Encounter) -> Self {
        let (code, display) = encounter_class_to_code(e.class);
        let class = FhirCoding {
            system: Some("http://terminology.hl7.org/CodeSystem/v3-ActCode".to_string()),
            code: code.to_string(),
            display: Some(display.to_string()),
        };
        let period = Some(FhirPeriod {
            start: Some(e.period_start.to_rfc3339()),
            end: e.period_end.map(|d| d.to_rfc3339()),
        });
        Self {
            resource_type: "Encounter".into(),
            id: Some(e.id.to_string()),
            status: encounter_status_to_str(e.status).to_string(),
            class,
            subject: FhirReference {
                reference: format!("Patient/{}", e.patient_id),
            },
            period,
        }
    }
}

impl FhirEncounter {
    /// Convert a wire `FhirEncounter` into a domain `Encounter`.
    pub fn into_domain(self) -> Result<Encounter> {
        let id = match self.id.as_deref() {
            Some(s) => Uuid::parse_str(s).map_err(|e| Error::Fhir(format!("bad id: {e}")))?,
            None => Uuid::new_v4(),
        };
        let patient_id = self
            .subject
            .reference
            .strip_prefix("Patient/")
            .ok_or_else(|| Error::Fhir("Encounter.subject must be Patient/{id}".into()))
            .and_then(|s| {
                Uuid::parse_str(s).map_err(|e| Error::Fhir(format!("bad subject uuid: {e}")))
            })?;
        let status = encounter_status_from_str(&self.status)?;
        let class = encounter_class_from_code(&self.class.code);
        let (period_start, period_end) = match self.period {
            Some(p) => {
                let start = match p.start.as_deref() {
                    Some(s) => parse_datetime(s)?,
                    None => Utc::now(),
                };
                let end = match p.end.as_deref() {
                    Some(s) => Some(parse_datetime(s)?),
                    None => None,
                };
                (start, end)
            }
            None => (Utc::now(), None),
        };
        let now = Utc::now();
        Ok(Encounter {
            id,
            patient_id,
            class,
            status,
            period_start,
            period_end,
            practitioner_id: None,
            department_id: None,
            reason: None,
            created_at: now,
            updated_at: now,
        })
    }
}

// ---------------------------------------------------------------------------
// Appointment
// ---------------------------------------------------------------------------

/// FHIR R5 `Appointment.participant` (minimal).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirAppointmentParticipant {
    pub actor: FhirReference,
    pub status: String,
}

/// FHIR R5 `Appointment` (minimal subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirAppointment {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub status: String,
    pub start: String,
    pub end: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participant: Vec<FhirAppointmentParticipant>,
}

impl From<&Appointment> for FhirAppointment {
    fn from(a: &Appointment) -> Self {
        let participant = vec![FhirAppointmentParticipant {
            actor: FhirReference {
                reference: format!("Patient/{}", a.patient_id),
            },
            status: "accepted".to_string(),
        }];
        Self {
            resource_type: "Appointment".into(),
            id: Some(a.id.to_string()),
            status: appointment_status_to_str(a.status).to_string(),
            start: a.start_datetime.to_rfc3339(),
            end: a.end_datetime.to_rfc3339(),
            participant,
        }
    }
}

impl FhirAppointment {
    pub fn into_domain(self) -> Result<Appointment> {
        let id = match self.id.as_deref() {
            Some(s) => Uuid::parse_str(s).map_err(|e| Error::Fhir(format!("bad id: {e}")))?,
            None => Uuid::new_v4(),
        };
        let patient_id = self
            .participant
            .iter()
            .find_map(|p| p.actor.reference.strip_prefix("Patient/"))
            .ok_or_else(|| Error::Fhir("Appointment.participant must include Patient/{id}".into()))
            .and_then(|s| {
                Uuid::parse_str(s).map_err(|e| Error::Fhir(format!("bad patient uuid: {e}")))
            })?;
        let status = appointment_status_from_str(&self.status)?;
        let start = parse_datetime(&self.start)?;
        let end = parse_datetime(&self.end)?;
        let now = Utc::now();
        Ok(Appointment {
            id,
            patient_id,
            slot_id: None,
            practitioner_id: None,
            start_datetime: start,
            end_datetime: end,
            status,
            reason: None,
            from_waitlist_entry_id: None,
            cancellation_reason: None,
            series_id: None,
            created_at: now,
            updated_at: now,
        })
    }
}

// ---------------------------------------------------------------------------
// Schedule
// ---------------------------------------------------------------------------

/// FHIR R5 `Schedule` (minimal subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirSchedule {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub active: bool,
    pub actor: Vec<FhirReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_type: Vec<FhirCodeableConcept>,
}

impl From<&Schedule> for FhirSchedule {
    fn from(s: &Schedule) -> Self {
        let actor_ref = match s.owner {
            ScheduleOwner::Practitioner(id) => format!("Practitioner/{id}"),
            ScheduleOwner::Bed(id) => format!("Location/{id}"),
            ScheduleOwner::Room(id) => format!("Location/{id}"),
        };
        let service_type = vec![FhirCodeableConcept {
            text: Some(s.service_type.clone()),
        }];
        Self {
            resource_type: "Schedule".into(),
            id: Some(s.id.to_string()),
            active: s.active,
            actor: vec![FhirReference {
                reference: actor_ref,
            }],
            service_type,
        }
    }
}

impl FhirSchedule {
    pub fn into_domain(self) -> Result<Schedule> {
        let id = match self.id.as_deref() {
            Some(s) => Uuid::parse_str(s).map_err(|e| Error::Fhir(format!("bad id: {e}")))?,
            None => Uuid::new_v4(),
        };
        let actor_ref = self
            .actor
            .first()
            .ok_or_else(|| Error::Fhir("Schedule.actor must not be empty".into()))?;
        let owner = if let Some(rest) = actor_ref.reference.strip_prefix("Practitioner/") {
            ScheduleOwner::Practitioner(
                Uuid::parse_str(rest)
                    .map_err(|e| Error::Fhir(format!("bad practitioner uuid: {e}")))?,
            )
        } else if let Some(rest) = actor_ref.reference.strip_prefix("Location/") {
            // We can't tell Bed vs Room from a Location reference alone;
            // default to Bed.
            ScheduleOwner::Bed(
                Uuid::parse_str(rest)
                    .map_err(|e| Error::Fhir(format!("bad location uuid: {e}")))?,
            )
        } else {
            return Err(Error::Fhir(format!(
                "unrecognized actor reference '{}'",
                actor_ref.reference
            )));
        };
        let service_type = self
            .service_type
            .into_iter()
            .next()
            .and_then(|c| c.text)
            .unwrap_or_default();
        let now = Utc::now();
        Ok(Schedule {
            id,
            owner,
            service_type,
            active: self.active,
            created_at: now,
            updated_at: now,
        })
    }
}

// ---------------------------------------------------------------------------
// Slot
// ---------------------------------------------------------------------------

/// FHIR R5 `Slot` (minimal subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirSlot {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub schedule: FhirReference,
    pub status: String,
    pub start: String,
    pub end: String,
}

impl From<&Slot> for FhirSlot {
    fn from(s: &Slot) -> Self {
        Self {
            resource_type: "Slot".into(),
            id: Some(s.id.to_string()),
            schedule: FhirReference {
                reference: format!("Schedule/{}", s.schedule_id),
            },
            status: slot_status_to_str(s.status).to_string(),
            start: s.start_datetime.to_rfc3339(),
            end: s.end_datetime.to_rfc3339(),
        }
    }
}

impl FhirSlot {
    pub fn into_domain(self) -> Result<Slot> {
        let id = match self.id.as_deref() {
            Some(s) => Uuid::parse_str(s).map_err(|e| Error::Fhir(format!("bad id: {e}")))?,
            None => Uuid::new_v4(),
        };
        let schedule_id = self
            .schedule
            .reference
            .strip_prefix("Schedule/")
            .ok_or_else(|| Error::Fhir("Slot.schedule must be Schedule/{id}".into()))
            .and_then(|s| {
                Uuid::parse_str(s).map_err(|e| Error::Fhir(format!("bad schedule uuid: {e}")))
            })?;
        let status = slot_status_from_str(&self.status)?;
        let start = parse_datetime(&self.start)?;
        let end = parse_datetime(&self.end)?;
        let now = Utc::now();
        Ok(Slot {
            id,
            schedule_id,
            start_datetime: start,
            end_datetime: end,
            status,
            created_at: now,
            updated_at: now,
        })
    }
}

// ---------------------------------------------------------------------------
// Practitioner
// ---------------------------------------------------------------------------

/// FHIR R5 `Practitioner` (minimal subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirPractitioner {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name: Vec<FhirHumanName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub telecom: Vec<FhirContactPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gender: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identifier: Vec<FhirIdentifier>,
}

impl From<&Practitioner> for FhirPractitioner {
    fn from(p: &Practitioner) -> Self {
        Self {
            resource_type: "Practitioner".into(),
            id: Some(p.id.to_string()),
            active: p.active,
            name: vec![FhirHumanName::from(&p.name)],
            telecom: p.telecom.iter().map(FhirContactPoint::from).collect(),
            gender: Some(gender_to_str(p.gender).to_string()),
            birth_date: p.birth_date.map(format_date),
            identifier: p.identifiers.iter().map(FhirIdentifier::from).collect(),
        }
    }
}

impl FhirPractitioner {
    pub fn into_domain(self) -> Result<Practitioner> {
        let id = match self.id.as_deref() {
            Some(s) => Uuid::parse_str(s).map_err(|e| Error::Fhir(format!("bad id: {e}")))?,
            None => Uuid::new_v4(),
        };
        let primary = self
            .name
            .first()
            .ok_or_else(|| Error::Fhir("Practitioner.name must not be empty".into()))?;
        let name = HumanName::from(primary);
        let gender = self
            .gender
            .as_deref()
            .map(gender_from_str)
            .unwrap_or(Gender::Unknown);
        let birth_date = self.birth_date.as_deref().and_then(parse_date);
        let telecom = self
            .telecom
            .into_iter()
            .filter_map(|cp| cp.into_domain())
            .collect();
        let identifiers = self
            .identifier
            .into_iter()
            .filter_map(|i| i.into_domain())
            .collect();
        let now = Utc::now();
        Ok(Practitioner {
            id,
            identifiers,
            active: self.active,
            name,
            telecom,
            addresses: Vec::new(),
            gender,
            birth_date,
            created_at: now,
            updated_at: now,
        })
    }
}

// ---------------------------------------------------------------------------
// Location (PAS represents a Bed as a Location/instance)
// ---------------------------------------------------------------------------

/// FHIR R5 `Location` (minimal subset). v0.1 maps PAS `Bed` → `Location`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirLocation {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub status: String,
    pub name: String,
    pub mode: String,
    pub physical_type: FhirCodeableConcept,
}

impl From<&Bed> for FhirLocation {
    fn from(b: &Bed) -> Self {
        Self {
            resource_type: "Location".into(),
            id: Some(b.id.to_string()),
            status: bed_status_to_location_status(b.status).to_string(),
            name: b.name.clone(),
            mode: "instance".to_string(),
            physical_type: FhirCodeableConcept {
                text: Some("Bed".to_string()),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Coverage (v0.10)
// ---------------------------------------------------------------------------

/// FHIR R5 `Coverage` (minimal subset).
///
/// Mapping from the PAS [`crate::models::coverage::Coverage`] domain row:
///
/// | PAS field           | FHIR field                              |
/// |---------------------|------------------------------------------|
/// | `id`                | `id`                                     |
/// | `status`            | `status` (active/cancelled/draft/entered-in-error) |
/// | `kind`              | `type.text` (`insurance` / `self_pay` / `other`) |
/// | `patient_id`        | `beneficiary` (`Patient/{id}`)           |
/// | `subscriber_id`     | `subscriber` (`Patient/{id}`, falls back to beneficiary) |
/// | `policy_number`     | `subscriberId`                           |
/// | `payor_name`        | `payor[0].display`                       |
/// | `payor_identifier`  | `payor[0].identifier.value` (when present)|
/// | `start_date` / `end_date` | `period.start` / `period.end`     |
/// | `relationship`      | `relationship.text`                      |
///
/// Read available since v0.10. Write supported as of v0.13 via the
/// `POST /fhir` Bundle endpoint (`type: batch` or `type: transaction`)
/// — `FhirCoverage::into_domain` parses the wire shape back to a
/// `Coverage`, and the bundle handler dispatches Coverage entries to
/// `CoverageRepository::create`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirCoverage {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// `active` / `cancelled` / `draft` / `entered-in-error`. FHIR uses
    /// kebab-case for the last one, where our domain uses snake_case
    /// (`entered_in_error`); the converter translates.
    pub status: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_: Option<FhirCodeableConcept>,
    pub beneficiary: FhirReference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscriber: Option<FhirReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscriber_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<FhirCodeableConcept>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<FhirPeriod>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub payor: Vec<FhirCoveragePayor>,
}

/// One entry in `Coverage.payor`. FHIR R5 allows several payors per
/// coverage (e.g. primary + secondary); the PAS surfaces exactly one
/// (the row's own payer). Both shapes ship the same name + identifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirCoveragePayor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<FhirIdentifier>,
}

impl From<&crate::models::coverage::Coverage> for FhirCoverage {
    fn from(c: &crate::models::coverage::Coverage) -> Self {
        // FHIR uses kebab-case for entered-in-error; everything else
        // already matches our snake_case repr.
        let status = match c.status {
            crate::models::coverage::CoverageStatus::Active => "active",
            crate::models::coverage::CoverageStatus::Cancelled => "cancelled",
            crate::models::coverage::CoverageStatus::Draft => "draft",
            crate::models::coverage::CoverageStatus::EnteredInError => "entered-in-error",
        };
        let beneficiary = FhirReference {
            reference: format!("Patient/{}", c.patient_id),
        };
        let subscriber = Some(FhirReference {
            reference: format!("Patient/{}", c.subscriber_id.unwrap_or(c.patient_id)),
        });
        let period = Some(FhirPeriod {
            start: Some(c.start_date.format("%Y-%m-%d").to_string()),
            end: c.end_date.map(|d| d.format("%Y-%m-%d").to_string()),
        });
        let type_ = Some(FhirCodeableConcept {
            text: Some(c.kind.as_str().to_string()),
        });
        let relationship = Some(FhirCodeableConcept {
            text: Some(c.relationship.clone()),
        });
        let payor_identifier = c.payor_identifier.as_ref().map(|val| FhirIdentifier {
            system: None,
            value: Some(val.clone()),
            use_field: None,
        });
        let payor = vec![FhirCoveragePayor {
            display: Some(c.payor_name.clone()),
            identifier: payor_identifier,
        }];
        Self {
            resource_type: "Coverage".into(),
            id: Some(c.id.to_string()),
            status: status.to_string(),
            type_,
            beneficiary,
            subscriber,
            subscriber_id: Some(c.policy_number.clone()),
            relationship,
            period,
            payor,
        }
    }
}

impl FhirCoverage {
    /// Parse a wire `FhirCoverage` into the domain model. Mirrors the
    /// `From<&Coverage>` direction:
    ///
    /// - `beneficiary.reference` must be `Patient/{uuid}` — that's the patient
    ///   linkage. Missing or malformed → 400.
    /// - `subscriber.reference`, when present, is parsed as `Patient/{uuid}`
    ///   and populates `subscriber_id`. Anything else is ignored (treated as
    ///   "no subscriber link").
    /// - `subscriber_id` (the FHIR string field — opaque policy id) maps back
    ///   to the domain `policy_number`. Required.
    /// - `payor[0].display` → `payor_name`. At least one payor entry with a
    ///   `display` is required.
    /// - `payor[0].identifier.value` → `payor_identifier` (optional).
    /// - `status` accepts both the FHIR kebab-case (`entered-in-error`) and
    ///   the domain snake_case (`entered_in_error`).
    /// - `type.text` → `CoverageKind`. Defaults to `insurance` when missing.
    /// - `period.start` → `start_date` (required). `period.end` → `end_date`.
    /// - `relationship.text` → `relationship` string. Defaults to `"self"`.
    pub fn into_domain(self) -> Result<crate::models::coverage::Coverage> {
        use crate::models::coverage::{Coverage, CoverageKind, CoverageStatus};
        let id = match self.id.as_deref() {
            Some(s) => Uuid::parse_str(s).map_err(|e| Error::Fhir(format!("bad id: {e}")))?,
            None => Uuid::new_v4(),
        };
        let patient_id = self
            .beneficiary
            .reference
            .strip_prefix("Patient/")
            .ok_or_else(|| Error::Fhir("Coverage.beneficiary must be Patient/{id}".into()))
            .and_then(|s| {
                Uuid::parse_str(s).map_err(|e| Error::Fhir(format!("bad beneficiary uuid: {e}")))
            })?;
        let subscriber_id = self
            .subscriber
            .as_ref()
            .and_then(|s| s.reference.strip_prefix("Patient/"))
            .and_then(|s| Uuid::parse_str(s).ok());
        let status = match self.status.as_str() {
            "active" => CoverageStatus::Active,
            "cancelled" => CoverageStatus::Cancelled,
            "draft" => CoverageStatus::Draft,
            "entered-in-error" | "entered_in_error" => CoverageStatus::EnteredInError,
            other => return Err(Error::Fhir(format!("unknown Coverage.status: {other}"))),
        };
        let kind = match self.type_.as_ref().and_then(|t| t.text.as_deref()) {
            None | Some("insurance") => CoverageKind::Insurance,
            Some("self_pay") | Some("self-pay") => CoverageKind::SelfPay,
            Some("other") => CoverageKind::Other,
            Some(other) => return Err(Error::Fhir(format!("unknown Coverage.type.text: {other}"))),
        };
        let policy_number = self.subscriber_id.clone().ok_or_else(|| {
            Error::Fhir("Coverage.subscriberId (policy number) is required".into())
        })?;
        let payor = self
            .payor
            .first()
            .ok_or_else(|| Error::Fhir("Coverage.payor[0] is required".into()))?;
        let payor_name = payor
            .display
            .clone()
            .ok_or_else(|| Error::Fhir("Coverage.payor[0].display is required".into()))?;
        let payor_identifier = payor.identifier.as_ref().and_then(|i| i.value.clone());
        let relationship = self
            .relationship
            .as_ref()
            .and_then(|r| r.text.clone())
            .unwrap_or_else(|| "self".into());
        let period = self
            .period
            .as_ref()
            .ok_or_else(|| Error::Fhir("Coverage.period is required".into()))?;
        let start_str = period
            .start
            .as_deref()
            .ok_or_else(|| Error::Fhir("Coverage.period.start is required".into()))?;
        let start_date = parse_date(start_str)
            .ok_or_else(|| Error::Fhir(format!("bad Coverage.period.start: {start_str}")))?;
        let end_date = match period.end.as_deref() {
            Some(s) => Some(
                parse_date(s)
                    .ok_or_else(|| Error::Fhir(format!("bad Coverage.period.end: {s}")))?,
            ),
            None => None,
        };
        let now = Utc::now();
        Ok(Coverage {
            id,
            patient_id,
            account_id: None,
            status,
            kind,
            subscriber_id,
            payor_name,
            payor_identifier,
            policy_number,
            group_number: None,
            relationship,
            start_date,
            end_date,
            created_at: now,
            updated_at: now,
        })
    }
}

// ---------------------------------------------------------------------------
// Bundle (collection)
// ---------------------------------------------------------------------------

/// Wire shape for one entry in a FHIR `Bundle`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirBundleEntry<T> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_url: Option<String>,
    pub resource: T,
}

/// FHIR R5 `Bundle` — collection variant only (no transaction support).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FhirBundle<T> {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(rename = "type")]
    pub bundle_type: String,
    pub total: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entry: Vec<FhirBundleEntry<T>>,
}

impl<T> FhirBundle<T> {
    pub fn collection(entries: Vec<FhirBundleEntry<T>>) -> Self {
        Self {
            resource_type: "Bundle".to_string(),
            bundle_type: "collection".to_string(),
            total: entries.len(),
            entry: entries,
        }
    }
}

/// Build a collection [`FhirBundle`] of patients with `fullUrl` populated
/// against the given base path (e.g. `"Patient/{id}"`).
pub fn patient_bundle(patients: &[Patient]) -> FhirBundle<FhirPatient> {
    let entries = patients
        .iter()
        .map(|p| FhirBundleEntry {
            full_url: Some(format!("Patient/{}", p.id)),
            resource: FhirPatient::from(p),
        })
        .collect();
    FhirBundle::collection(entries)
}

// ---------------------------------------------------------------------------
// Bundle (write — batch / transaction)
// ---------------------------------------------------------------------------

/// FHIR `Bundle.entry.request` — what the client wants done with this entry.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FhirBundleRequest {
    pub method: String,
    pub url: String,
}

/// FHIR `Bundle.entry.response` — what the server did with this entry.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FhirBundleResponse {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// One entry in a write Bundle. The `resource` is left as
/// `serde_json::Value` because batch entries are heterogeneous (Patient one
/// row, Encounter the next, …).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FhirBundleWriteEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_url: Option<String>,
    #[schema(value_type = Object)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<FhirBundleRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<FhirBundleResponse>,
}

/// FHIR `Bundle` — write shape (batch or transaction).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FhirWriteBundle {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(rename = "type")]
    pub bundle_type: String,
    #[serde(default)]
    pub entry: Vec<FhirBundleWriteEntry>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_name() -> HumanName {
        HumanName {
            use_type: Some(NameUse::Official),
            family: "Doe".into(),
            given: vec!["Jane".into()],
            prefix: vec![],
            suffix: vec![],
        }
    }

    fn sample_patient() -> Patient {
        let mut p = Patient::new(sample_name(), Gender::Female);
        p.birth_date = Some(NaiveDate::from_ymd_opt(1990, 1, 15).unwrap());
        p.telecom = vec![ContactPoint {
            system: ContactPointSystem::Phone,
            value: "+1-555-0100".into(),
            use_type: Some(ContactPointUse::Home),
        }];
        p.addresses = vec![Address {
            use_type: Some(AddressUse::Home),
            line1: Some("123 Elm".into()),
            line2: None,
            city: Some("Springfield".into()),
            state: Some("IL".into()),
            postal_code: Some("62701".into()),
            country: Some("US".into()),
        }];
        p.identifiers = vec![Identifier::mrn("urn:oid:facility:1", "MRN-001")];
        p
    }

    #[test]
    fn test_patient_to_fhir_basic_fields() {
        let p = sample_patient();
        let f = FhirPatient::from(&p);
        assert_eq!(f.resource_type, "Patient");
        assert_eq!(f.id.as_deref(), Some(p.id.to_string()).as_deref());
        assert!(f.active);
        assert_eq!(f.gender.as_deref(), Some("female"));
        assert_eq!(f.birth_date.as_deref(), Some("1990-01-15"));
        assert_eq!(f.name.len(), 1);
        assert_eq!(f.name[0].family.as_deref(), Some("Doe"));
        assert_eq!(f.telecom.len(), 1);
        assert_eq!(f.telecom[0].system.as_deref(), Some("phone"));
        assert_eq!(f.address.len(), 1);
        assert_eq!(f.address[0].line, vec!["123 Elm".to_string()]);
        assert_eq!(f.identifier.len(), 1);
        assert_eq!(f.identifier[0].value.as_deref(), Some("MRN-001"));
    }

    #[test]
    fn test_patient_json_roundtrip_preserves_core_fields() {
        let p = sample_patient();
        let f = FhirPatient::from(&p);
        let json = serde_json::to_string(&f).expect("serialize fhir patient");
        // Sanity: camelCase + resourceType present
        assert!(json.contains("\"resourceType\":\"Patient\""));
        assert!(json.contains("\"birthDate\":\"1990-01-15\""));
        let back: FhirPatient = serde_json::from_str(&json).expect("deserialize");
        let domain = back.into_domain().expect("into_domain");
        assert_eq!(domain.id, p.id);
        assert_eq!(domain.name.family, "Doe");
        assert_eq!(domain.gender, Gender::Female);
        assert_eq!(domain.birth_date, p.birth_date);
        assert_eq!(domain.telecom.len(), 1);
        assert_eq!(domain.telecom[0].value, "+1-555-0100");
        assert_eq!(domain.addresses.len(), 1);
        assert_eq!(domain.addresses[0].line1.as_deref(), Some("123 Elm"));
        assert_eq!(domain.identifiers.len(), 1);
        assert_eq!(domain.identifiers[0].value, "MRN-001");
    }

    #[test]
    fn test_patient_into_domain_requires_name() {
        let f = FhirPatient {
            resource_type: "Patient".into(),
            id: None,
            active: true,
            name: vec![],
            gender: None,
            birth_date: None,
            telecom: vec![],
            address: vec![],
            identifier: vec![],
            deceased_boolean: None,
            deceased_date_time: None,
            link: vec![],
        };
        let err = f.into_domain().expect_err("empty name must fail");
        assert!(matches!(err, Error::Fhir(_)));
    }

    #[test]
    fn test_patient_to_fhir_emits_replaced_by_link_on_tombstone() {
        // Live patient → no link entries.
        let mut p = sample_patient();
        let f_live = FhirPatient::from(&p);
        assert!(
            f_live.link.is_empty(),
            "live patient must not emit a link array: {:?}",
            f_live.link
        );

        // Tombstone → exactly one link, type=replaced-by, pointing at
        // the survivor. The kebab-case `replaced-by` is FHIR spec.
        let survivor_id = Uuid::new_v4();
        p.replaced_by = Some(survivor_id);
        let f_tomb = FhirPatient::from(&p);
        assert_eq!(f_tomb.link.len(), 1);
        assert_eq!(f_tomb.link[0].link_type, "replaced-by");
        assert_eq!(
            f_tomb.link[0].other.reference,
            format!("Patient/{survivor_id}")
        );
    }

    #[test]
    fn test_patient_deceased_datetime_roundtrip() {
        let mut p = sample_patient();
        p.deceased = true;
        p.deceased_datetime = Some(Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap());
        let f = FhirPatient::from(&p);
        assert!(f.deceased_boolean.is_none());
        assert!(f.deceased_date_time.is_some());
        let domain = f.into_domain().expect("into_domain");
        assert!(domain.deceased);
        assert_eq!(domain.deceased_datetime, p.deceased_datetime);
    }

    #[test]
    fn test_encounter_status_mapping_to_fhir() {
        let cases = [
            (EncounterStatus::Planned, "planned"),
            (EncounterStatus::Arrived, "arrived"),
            (EncounterStatus::InProgress, "in-progress"),
            (EncounterStatus::OnLeave, "onleave"),
            (EncounterStatus::Finished, "finished"),
            (EncounterStatus::Cancelled, "cancelled"),
        ];
        for (s, expected) in cases {
            assert_eq!(encounter_status_to_str(s), expected);
            assert_eq!(encounter_status_from_str(expected).unwrap(), s);
        }
    }

    #[test]
    fn test_encounter_to_fhir_and_back() {
        let patient_id = Uuid::new_v4();
        let e = Encounter::new(patient_id, EncounterClass::Inpatient);
        let f = FhirEncounter::from(&e);
        assert_eq!(f.resource_type, "Encounter");
        assert_eq!(f.status, "planned");
        assert_eq!(f.class.code, "IMP");
        assert_eq!(f.subject.reference, format!("Patient/{patient_id}"));
        let back = f.into_domain().expect("into_domain");
        assert_eq!(back.id, e.id);
        assert_eq!(back.patient_id, patient_id);
        assert_eq!(back.status, EncounterStatus::Planned);
        assert_eq!(back.class, EncounterClass::Inpatient);
    }

    #[test]
    fn test_encounter_into_domain_rejects_bad_subject() {
        let f = FhirEncounter {
            resource_type: "Encounter".into(),
            id: None,
            status: "planned".into(),
            class: FhirCoding {
                system: None,
                code: "IMP".into(),
                display: None,
            },
            subject: FhirReference {
                reference: "Group/abc".into(),
            },
            period: None,
        };
        let err = f.into_domain().expect_err("bad subject must fail");
        assert!(matches!(err, Error::Fhir(_)));
    }

    #[test]
    fn test_appointment_status_mapping() {
        let cases = [
            (AppointmentStatus::Proposed, "proposed"),
            (AppointmentStatus::Booked, "booked"),
            (AppointmentStatus::Arrived, "arrived"),
            (AppointmentStatus::Fulfilled, "fulfilled"),
            (AppointmentStatus::Cancelled, "cancelled"),
            (AppointmentStatus::NoShow, "noshow"),
        ];
        for (s, expected) in cases {
            assert_eq!(appointment_status_to_str(s), expected);
            assert_eq!(appointment_status_from_str(expected).unwrap(), s);
        }
    }

    #[test]
    fn test_appointment_to_fhir_roundtrip() {
        let patient_id = Uuid::new_v4();
        let start = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap();
        let a = Appointment::new(patient_id, start, end);
        let f = FhirAppointment::from(&a);
        assert_eq!(f.resource_type, "Appointment");
        assert_eq!(f.status, "proposed");
        assert_eq!(f.participant.len(), 1);
        let back = f.into_domain().expect("into_domain");
        assert_eq!(back.id, a.id);
        assert_eq!(back.patient_id, patient_id);
        assert_eq!(back.start_datetime, start);
        assert_eq!(back.end_datetime, end);
    }

    #[test]
    fn test_slot_status_mapping() {
        let cases = [
            (SlotStatus::Free, "free"),
            (SlotStatus::Busy, "busy"),
            (SlotStatus::BlockedOut, "blocked-out"),
        ];
        for (s, expected) in cases {
            assert_eq!(slot_status_to_str(s), expected);
            assert_eq!(slot_status_from_str(expected).unwrap(), s);
        }
    }

    #[test]
    fn test_schedule_practitioner_owner_roundtrip() {
        let pid = Uuid::new_v4();
        let s = Schedule::new(ScheduleOwner::Practitioner(pid), "cardiology".into());
        let f = FhirSchedule::from(&s);
        assert_eq!(f.resource_type, "Schedule");
        assert!(f.actor[0].reference.starts_with("Practitioner/"));
        assert_eq!(f.service_type[0].text.as_deref(), Some("cardiology"));
        let back = f.into_domain().expect("into_domain");
        assert_eq!(back.id, s.id);
        assert_eq!(back.owner, ScheduleOwner::Practitioner(pid));
        assert_eq!(back.service_type, "cardiology");
    }

    #[test]
    fn test_practitioner_to_fhir_basic() {
        let p = Practitioner::new(sample_name(), Gender::Female);
        let f = FhirPractitioner::from(&p);
        assert_eq!(f.resource_type, "Practitioner");
        assert!(f.active);
        assert_eq!(f.gender.as_deref(), Some("female"));
        let back = f.into_domain().expect("into_domain");
        assert_eq!(back.id, p.id);
        assert_eq!(back.name.family, "Doe");
        assert_eq!(back.gender, Gender::Female);
    }

    #[test]
    fn test_bed_to_location() {
        let b = Bed::new(Uuid::new_v4(), "Bed A".into(), "B-A".into());
        let f = FhirLocation::from(&b);
        assert_eq!(f.resource_type, "Location");
        assert_eq!(f.name, "Bed A");
        assert_eq!(f.mode, "instance");
        assert_eq!(f.physical_type.text.as_deref(), Some("Bed"));
        assert_eq!(f.status, "active"); // Available
    }

    #[test]
    fn test_bed_out_of_service_location_is_inactive() {
        let mut b = Bed::new(Uuid::new_v4(), "Bed A".into(), "B-A".into());
        b.status = BedStatus::OutOfService;
        let f = FhirLocation::from(&b);
        assert_eq!(f.status, "inactive");
    }

    #[test]
    fn test_patient_bundle_collection_shape() {
        let p1 = sample_patient();
        let p2 = sample_patient();
        let bundle = patient_bundle(&[p1.clone(), p2.clone()]);
        assert_eq!(bundle.resource_type, "Bundle");
        assert_eq!(bundle.bundle_type, "collection");
        assert_eq!(bundle.total, 2);
        assert_eq!(bundle.entry.len(), 2);
        assert_eq!(
            bundle.entry[0].full_url.as_deref(),
            Some(format!("Patient/{}", p1.id)).as_deref()
        );
        assert_eq!(bundle.entry[0].resource.resource_type, "Patient");
    }

    #[test]
    fn test_write_bundle_deserializes_transaction_shape() {
        let json = r#"{
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                {
                    "fullUrl": "urn:uuid:11111111-1111-4111-8111-111111111111",
                    "resource": {
                        "resourceType": "Patient",
                        "active": true,
                        "name": [{ "family": "Doe", "given": ["Jane"] }]
                    },
                    "request": { "method": "POST", "url": "Patient" }
                }
            ]
        }"#;
        let parsed: FhirWriteBundle = serde_json::from_str(json).expect("parse");
        assert_eq!(parsed.bundle_type, "transaction");
        assert_eq!(parsed.entry.len(), 1);
        let req = parsed.entry[0].request.as_ref().expect("request");
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "Patient");
        let res = parsed.entry[0].resource.as_ref().expect("resource");
        assert_eq!(res["resourceType"], "Patient");
    }

    #[test]
    fn test_patient_bundle_empty_is_well_formed() {
        let bundle = patient_bundle(&[]);
        assert_eq!(bundle.total, 0);
        assert!(bundle.entry.is_empty());
        let json = serde_json::to_string(&bundle).expect("serialize");
        assert!(json.contains("\"resourceType\":\"Bundle\""));
        assert!(json.contains("\"type\":\"collection\""));
    }

    #[test]
    fn test_fhir_coverage_round_trip_through_into_domain() {
        use crate::models::coverage::{Coverage, CoverageKind, CoverageStatus};
        let mut c = Coverage::new(Uuid::new_v4(), "Aetna", "AET-12345");
        c.kind = CoverageKind::Insurance;
        c.status = CoverageStatus::Active;
        c.payor_identifier = Some("AET-EIN-001".into());
        c.relationship = "spouse".into();
        c.subscriber_id = Some(Uuid::new_v4());
        c.end_date = Some(c.start_date + chrono::Duration::days(180));
        let f = FhirCoverage::from(&c);
        assert_eq!(f.resource_type, "Coverage");
        let back = f.into_domain().expect("into_domain");
        assert_eq!(back.id, c.id);
        assert_eq!(back.patient_id, c.patient_id);
        assert_eq!(back.status, c.status);
        assert_eq!(back.kind, c.kind);
        assert_eq!(back.subscriber_id, c.subscriber_id);
        assert_eq!(back.payor_name, c.payor_name);
        assert_eq!(back.payor_identifier, c.payor_identifier);
        assert_eq!(back.policy_number, c.policy_number);
        assert_eq!(back.relationship, c.relationship);
        assert_eq!(back.start_date, c.start_date);
        assert_eq!(back.end_date, c.end_date);
    }

    #[test]
    fn test_fhir_coverage_into_domain_requires_beneficiary_patient() {
        let mut f = FhirCoverage::from(&crate::models::coverage::Coverage::new(
            Uuid::new_v4(),
            "Aetna",
            "P-1",
        ));
        f.beneficiary.reference = "Organization/abc".into();
        let err = f.into_domain().expect_err("must fail without Patient/");
        assert!(format!("{err}").contains("beneficiary"));
    }

    #[test]
    fn test_fhir_coverage_into_domain_accepts_entered_in_error_kebab_case() {
        let mut f = FhirCoverage::from(&crate::models::coverage::Coverage::new(
            Uuid::new_v4(),
            "Aetna",
            "P-1",
        ));
        f.status = "entered-in-error".into();
        let back = f.into_domain().expect("into_domain");
        assert_eq!(
            back.status,
            crate::models::coverage::CoverageStatus::EnteredInError
        );
    }

    #[test]
    fn test_fhir_coverage_into_domain_requires_payor_display() {
        let mut f = FhirCoverage::from(&crate::models::coverage::Coverage::new(
            Uuid::new_v4(),
            "Aetna",
            "P-1",
        ));
        f.payor[0].display = None;
        let err = f
            .into_domain()
            .expect_err("must fail without payor display");
        assert!(format!("{err}").contains("payor"));
    }
}
