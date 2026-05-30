//! OpenAPI spec aggregator.
//!
//! Builds an [`utoipa::OpenApi`] document covering the v0.2/v0.3 endpoints
//! (bulk interchange, FHIR Bundle writes, HL7 v2 ADT). v0.1 endpoints are
//! intentionally not annotated yet — adding `#[utoipa::path]` attributes
//! across the ~60 v0.1 handlers is a follow-on task. The spec is served at
//! `/api-docs/openapi.json` and the Swagger UI mounts at `/swagger-ui`.

use utoipa::OpenApi;

use crate::api::fhir::handlers as fhir_handlers;
use crate::api::fhir::operation_outcome::{Issue, OperationOutcome};
use crate::api::fhir::resources::{
    FhirBundleRequest, FhirBundleResponse, FhirBundleWriteEntry, FhirWriteBundle,
};
use crate::api::rest::handlers as rest_handlers;
use crate::interchange::PatientRow;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Patient Administration System",
        description = "Full OpenAPI spec covering every public REST + FHIR + HL7 v2 endpoint exposed by the PAS — ADT, scheduling, waitlist + RTT, resources, billing, communication, patient/practitioner/consent CRUD, audit query, facility setup, plus the v0.2/v0.3 interchange + FHIR Bundle + HL7 v2 surface.",
        version = "0.3.0"
    ),
    tags(
        (name = "Health", description = "Health + readiness checks."),
        (name = "Patient", description = "Patient CRUD + search (Tantivy-backed full-text)."),
        (name = "Privacy", description = "GDPR/HIPAA-style masked views and subject-access exports."),
        (name = "ADT", description = "Admission / Discharge / Transfer over REST."),
        (name = "Encounter", description = "Encounter reads + lifecycle (cancel / set_status)."),
        (name = "Scheduling", description = "Schedule + slot CRUD, slot booking, appointment lifecycle."),
        (name = "Waitlist", description = "Referral waitlist add / remove / update."),
        (name = "RTT", description = "Referral-To-Treatment clock and breach detection."),
        (name = "Resources", description = "Ward occupancy, bed reads, bed status transitions."),
        (name = "FacilitySetup", description = "Bootstrap Facility → Ward → Room → Bed hierarchy."),
        (name = "Workforce", description = "Practitioner CRUD."),
        (name = "Billing", description = "Accounts, charges, invoices, payments."),
        (name = "Communication", description = "Letter templates and generated-letter lifecycle."),
        (name = "Consent", description = "GDPR consent records."),
        (name = "Audit", description = "Audit query endpoints (per-patient, recent, by entity)."),
        (name = "Admin", description = "Operational diagnostics — outbox status etc."),
        (name = "Interchange", description = "Bulk patient export/import in JSON / XML / TSV / CSV. Lossy `PatientRow` projection — see AGENTS/interchange.md."),
        (name = "FHIR", description = "FHIR R5 batch / transaction Bundle writes. Transaction Bundles are all-or-nothing."),
        (name = "HL7v2", description = "HL7 v2 ADT message ingest over HTTP. The same handlers also serve MLLP traffic when `HL7V2_MLLP_BIND` is set.")
    ),
    paths(
        rest_handlers::health,
        rest_handlers::admit,
        rest_handlers::pre_admit,
        rest_handlers::change_to_inpatient,
        rest_handlers::cancel_admit,
        rest_handlers::cancel_pre_admit,
        rest_handlers::leave_start,
        rest_handlers::leave_end,
        rest_handlers::transfer,
        rest_handlers::cancel_transfer,
        rest_handlers::discharge,
        rest_handlers::book_slot,
        rest_handlers::cancel_appointment,
        rest_handlers::check_in_appointment,
        rest_handlers::complete_appointment,
        rest_handlers::add_to_waitlist,
        rest_handlers::remove_from_waitlist,
        rest_handlers::rtt_start,
        rest_handlers::rtt_pause,
        rest_handlers::rtt_resume,
        rest_handlers::rtt_stop,
        rest_handlers::rtt_weeks_waiting,
        rest_handlers::ward_occupancy,
        rest_handlers::set_bed_status,
        rest_handlers::open_account,
        rest_handlers::post_charge,
        rest_handlers::finalize_invoice,
        rest_handlers::post_payment,
        rest_handlers::generate_letter,
        rest_handlers::create_patient,
        rest_handlers::get_patient,
        rest_handlers::update_patient,
        rest_handlers::delete_patient,
        rest_handlers::search_patients,
        rest_handlers::list_patients,
        rest_handlers::get_patient_masked,
        rest_handlers::get_patient_export,
        rest_handlers::get_patient_audit,
        rest_handlers::get_recent_audit,
        rest_handlers::get_entity_audit,
        rest_handlers::get_unpublished_outbox,
        rest_handlers::get_encounter,
        rest_handlers::list_patient_encounters,
        rest_handlers::cancel_encounter,
        rest_handlers::set_encounter_status,
        rest_handlers::create_encounter,
        rest_handlers::get_appointment,
        rest_handlers::list_appointments,
        rest_handlers::list_waitlist,
        rest_handlers::list_patient_rtt,
        rest_handlers::get_patient_account,
        rest_handlers::create_letter_template,
        rest_handlers::list_letter_templates,
        rest_handlers::get_generated_letter,
        rest_handlers::mark_letter_sent,
        rest_handlers::mark_letter_failed,
        rest_handlers::update_letter_template,
        rest_handlers::delete_letter_template,
        rest_handlers::delete_schedule,
        rest_handlers::delete_slot,
        rest_handlers::get_schedule,
        rest_handlers::list_schedule_slots,
        rest_handlers::create_schedule,
        rest_handlers::create_slot,
        rest_handlers::bulk_create_slots,
        rest_handlers::list_ward_beds,
        rest_handlers::get_bed,
        rest_handlers::create_facility,
        rest_handlers::create_ward,
        rest_handlers::create_room,
        rest_handlers::create_bed,
        rest_handlers::update_bed,
        rest_handlers::list_facilities,
        rest_handlers::list_wards,
        rest_handlers::create_practitioner,
        rest_handlers::get_practitioner,
        rest_handlers::update_practitioner,
        rest_handlers::delete_practitioner,
        rest_handlers::list_practitioners,
        rest_handlers::create_consent,
        rest_handlers::list_patient_consents,
        rest_handlers::revoke_consent,
        rest_handlers::list_rtt_breaches,
        rest_handlers::update_waitlist_entry,
        rest_handlers::export_patients_json,
        rest_handlers::export_patients_xml,
        rest_handlers::export_patients_tsv,
        rest_handlers::export_patients_csv,
        rest_handlers::import_patients,
        rest_handlers::hl7_v2_parse,
        rest_handlers::hl7_v2_batch,
        rest_handlers::hl7_v2_patient,
        rest_handlers::hl7_v2_update,
        rest_handlers::hl7_v2_admit,
        rest_handlers::hl7_v2_pre_admit,
        rest_handlers::hl7_v2_cancel_pre_admit,
        rest_handlers::hl7_v2_leave_start,
        rest_handlers::hl7_v2_leave_end,
        rest_handlers::hl7_v2_delete_patient,
        rest_handlers::hl7_v2_change_to_inpatient,
        rest_handlers::hl7_v2_change_to_outpatient,
        rest_handlers::hl7_v2_register,
        rest_handlers::hl7_v2_transfer,
        rest_handlers::hl7_v2_discharge,
        rest_handlers::hl7_v2_cancel_admit,
        rest_handlers::hl7_v2_cancel_transfer,
        rest_handlers::hl7_v2_cancel_discharge,
        rest_handlers::hl7_v2_merge,
        rest_handlers::hl7_v2_dft,
        rest_handlers::hl7_v2_mfn_staff,
        rest_handlers::hl7_v2_mfn_location,
        rest_handlers::hl7_v2_schedule_book,
        rest_handlers::hl7_v2_schedule_reschedule,
        rest_handlers::hl7_v2_schedule_modify,
        rest_handlers::hl7_v2_schedule_cancel,
        rest_handlers::list_dead_letter_outbox,
        rest_handlers::replay_dead_letter_outbox,
        rest_handlers::preview_appointment_series,
        rest_handlers::create_appointment_series,
        rest_handlers::get_appointment_series,
        rest_handlers::cancel_appointment_series,
        rest_handlers::list_patient_appointment_series,
        rest_handlers::create_coverage,
        rest_handlers::get_coverage,
        rest_handlers::update_coverage,
        rest_handlers::delete_coverage,
        rest_handlers::list_patient_coverages,
        rest_handlers::list_account_coverages,
        rest_handlers::merge_patient_into,
        rest_handlers::list_patient_replaces,
        fhir_handlers::process_bundle,
    ),
    components(schemas(
        PatientRow,
        rest_handlers::ImportSummary,
        rest_handlers::AdmitRequest,
        rest_handlers::TransferRequest,
        rest_handlers::BookRequest,
        rest_handlers::CancelRequest,
        rest_handlers::WaitlistAddRequest,
        rest_handlers::RttStartRequest,
        rest_handlers::RttReasonRequest,
        rest_handlers::SetBedStatusRequest,
        rest_handlers::OpenAccountRequest,
        rest_handlers::PostChargeRequest,
        rest_handlers::CreateSeriesRequest,
        rest_handlers::CancelSeriesRequest,
        rest_handlers::CreateCoverageRequest,
        rest_handlers::UpdateCoverageRequest,
        rest_handlers::FinalizeInvoiceRequest,
        rest_handlers::PostPaymentRequest,
        rest_handlers::GenerateLetterRequest,
        rest_handlers::CreatePatientRequest,
        rest_handlers::UpdatePatientRequest,
        rest_handlers::CreateEncounterRequest,
        rest_handlers::EncounterStatusRequest,
        rest_handlers::CreateLetterTemplateRequest,
        rest_handlers::UpdateLetterTemplateRequest,
        rest_handlers::CreateScheduleRequest,
        rest_handlers::ScheduleOwnerInput,
        rest_handlers::CreateSlotRequest,
        rest_handlers::BulkSlotRequest,
        rest_handlers::CreateFacilityRequest,
        rest_handlers::CreateWardRequest,
        rest_handlers::CreateRoomRequest,
        rest_handlers::CreateBedRequest,
        rest_handlers::CreatePractitionerRequest,
        rest_handlers::UpdatePractitionerRequest,
        rest_handlers::CreateConsentRequest,
        rest_handlers::UpdateWaitlistRequest,
        rest_handlers::RttBreach,
        FhirWriteBundle,
        FhirBundleWriteEntry,
        FhirBundleRequest,
        FhirBundleResponse,
        OperationOutcome,
        Issue,
    ))
)]
pub struct ApiDoc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openapi_doc_is_well_formed_json() {
        let doc = ApiDoc::openapi();
        let json = doc.to_pretty_json().expect("to_pretty_json");
        // Sanity checks: title + every annotated path appears.
        assert!(json.contains("\"title\": \"Patient Administration System\""));
        for path in [
            "/api/health",
            "/api/admissions",
            "/api/patients",
            "/api/patients/search",
            "/api/patients/{id}",
            "/api/encounters",
            "/api/practitioners",
            "/api/consents/{id}/revoke",
            "/api/letter-templates",
            "/api/facilities",
            "/api/wards",
            "/api/rooms",
            "/api/beds",
            "/api/schedules",
            "/api/audit/recent",
            "/api/admin/outbox/unpublished",
            "/api/slots/{slot_id}/book",
            "/api/waitlist",
            "/api/waitlist/breaches",
            "/api/rtt/start",
            "/api/wards/{ward_id}/occupancy",
            "/api/accounts",
            "/api/charges",
            "/api/letters/generate",
            "/api/patients/export.json",
            "/api/patients/export.xml",
            "/api/patients/export.tsv",
            "/api/patients/export.csv",
            "/api/patients/import",
            "/api/hl7/v2/parse",
            "/api/hl7/v2/patient",
            "/api/hl7/v2/admit",
            "/api/hl7/v2/transfer",
            "/api/hl7/v2/discharge",
            "/fhir",
        ] {
            assert!(
                json.contains(&format!("\"{path}\"")),
                "expected {path} in OpenAPI spec"
            );
        }
        // Schemas section includes every component schema we registered.
        for schema in [
            "PatientRow",
            "ImportSummary",
            "FhirWriteBundle",
            "FhirBundleWriteEntry",
            "FhirBundleRequest",
            "FhirBundleResponse",
            "OperationOutcome",
        ] {
            assert!(
                json.contains(&format!("\"{schema}\"")),
                "expected schema {schema} in OpenAPI spec"
            );
        }
    }

    #[test]
    fn test_openapi_doc_tags_present() {
        let doc = ApiDoc::openapi();
        let json = serde_json::to_string(&doc).expect("serialize");
        for tag in ["Interchange", "FHIR", "HL7v2"] {
            assert!(json.contains(tag), "expected tag {tag}");
        }
    }
}
