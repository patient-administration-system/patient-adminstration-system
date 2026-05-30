//! REST API routing table.

use axum::{
    Router,
    routing::{delete, get, post, put},
};

use super::{handlers, state::AppState};

/// Build the PAS REST router and attach the shared [`AppState`].
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(handlers::health))
        .route(
            "/api/patients",
            post(handlers::create_patient).get(handlers::list_patients),
        )
        .route("/api/patients/search", get(handlers::search_patients))
        .route(
            "/api/patients/export.json",
            get(handlers::export_patients_json),
        )
        .route(
            "/api/patients/export.xml",
            get(handlers::export_patients_xml),
        )
        .route(
            "/api/patients/export.tsv",
            get(handlers::export_patients_tsv),
        )
        .route(
            "/api/patients/export.csv",
            get(handlers::export_patients_csv),
        )
        .route("/api/patients/import", post(handlers::import_patients))
        .route("/api/hl7/v2/parse", post(handlers::hl7_v2_parse))
        .route("/api/hl7/v2/batch", post(handlers::hl7_v2_batch))
        .route("/api/hl7/v2/patient", post(handlers::hl7_v2_patient))
        .route("/api/hl7/v2/update", post(handlers::hl7_v2_update))
        .route("/api/hl7/v2/admit", post(handlers::hl7_v2_admit))
        .route("/api/hl7/v2/pre-admit", post(handlers::hl7_v2_pre_admit))
        .route(
            "/api/hl7/v2/cancel-pre-admit",
            post(handlers::hl7_v2_cancel_pre_admit),
        )
        .route(
            "/api/hl7/v2/leave-start",
            post(handlers::hl7_v2_leave_start),
        )
        .route("/api/hl7/v2/leave-end", post(handlers::hl7_v2_leave_end))
        .route(
            "/api/hl7/v2/delete-patient",
            post(handlers::hl7_v2_delete_patient),
        )
        .route(
            "/api/hl7/v2/change-to-inpatient",
            post(handlers::hl7_v2_change_to_inpatient),
        )
        .route(
            "/api/hl7/v2/change-to-outpatient",
            post(handlers::hl7_v2_change_to_outpatient),
        )
        .route("/api/hl7/v2/register", post(handlers::hl7_v2_register))
        .route("/api/hl7/v2/transfer", post(handlers::hl7_v2_transfer))
        .route("/api/hl7/v2/discharge", post(handlers::hl7_v2_discharge))
        .route(
            "/api/hl7/v2/cancel-admit",
            post(handlers::hl7_v2_cancel_admit),
        )
        .route(
            "/api/hl7/v2/cancel-transfer",
            post(handlers::hl7_v2_cancel_transfer),
        )
        .route(
            "/api/hl7/v2/cancel-discharge",
            post(handlers::hl7_v2_cancel_discharge),
        )
        .route("/api/hl7/v2/merge", post(handlers::hl7_v2_merge))
        .route("/api/hl7/v2/dft", post(handlers::hl7_v2_dft))
        .route("/api/hl7/v2/mfn-staff", post(handlers::hl7_v2_mfn_staff))
        .route(
            "/api/hl7/v2/mfn-location",
            post(handlers::hl7_v2_mfn_location),
        )
        .route(
            "/api/hl7/v2/schedule-book",
            post(handlers::hl7_v2_schedule_book),
        )
        .route(
            "/api/hl7/v2/schedule-reschedule",
            post(handlers::hl7_v2_schedule_reschedule),
        )
        .route(
            "/api/hl7/v2/schedule-modify",
            post(handlers::hl7_v2_schedule_modify),
        )
        .route(
            "/api/hl7/v2/schedule-cancel",
            post(handlers::hl7_v2_schedule_cancel),
        )
        .route(
            "/api/practitioners",
            post(handlers::create_practitioner).get(handlers::list_practitioners),
        )
        .route(
            "/api/practitioners/:id",
            get(handlers::get_practitioner)
                .put(handlers::update_practitioner)
                .delete(handlers::delete_practitioner),
        )
        .route(
            "/api/patients/:id/consents",
            post(handlers::create_consent).get(handlers::list_patient_consents),
        )
        .route("/api/consents/:id/revoke", post(handlers::revoke_consent))
        .route(
            "/api/patients/:id",
            get(handlers::get_patient)
                .put(handlers::update_patient)
                .delete(handlers::delete_patient),
        )
        .route(
            "/api/patients/:id/masked",
            get(handlers::get_patient_masked),
        )
        .route(
            "/api/patients/:id/export",
            get(handlers::get_patient_export),
        )
        .route("/api/patients/:id/audit", get(handlers::get_patient_audit))
        .route(
            "/api/patients/:id/merge-into/:target_id",
            post(handlers::merge_patient_into),
        )
        .route(
            "/api/patients/:id/replaces",
            get(handlers::list_patient_replaces),
        )
        .route("/api/audit/recent", get(handlers::get_recent_audit))
        .route("/api/audit/entity", get(handlers::get_entity_audit))
        .route(
            "/api/admin/outbox/unpublished",
            get(handlers::get_unpublished_outbox),
        )
        .route(
            "/api/admin/outbox/dead-letters",
            get(handlers::list_dead_letter_outbox),
        )
        .route(
            "/api/admin/outbox/dead-letters/:id/replay",
            post(handlers::replay_dead_letter_outbox),
        )
        .route("/api/encounters/:id", get(handlers::get_encounter))
        .route(
            "/api/patients/:id/encounters",
            get(handlers::list_patient_encounters),
        )
        .route("/api/appointments", get(handlers::list_appointments))
        .route(
            "/api/appointment-series/preview",
            post(handlers::preview_appointment_series),
        )
        .route(
            "/api/appointment-series",
            post(handlers::create_appointment_series),
        )
        .route(
            "/api/appointment-series/:id",
            get(handlers::get_appointment_series),
        )
        .route(
            "/api/appointment-series/:id/cancel",
            post(handlers::cancel_appointment_series),
        )
        .route(
            "/api/patients/:id/appointment-series",
            get(handlers::list_patient_appointment_series),
        )
        .route("/api/appointments/:id", get(handlers::get_appointment))
        .route("/api/patients/:id/rtt", get(handlers::list_patient_rtt))
        .route(
            "/api/patients/:id/account",
            get(handlers::get_patient_account),
        )
        .route(
            "/api/letter-templates",
            get(handlers::list_letter_templates).post(handlers::create_letter_template),
        )
        .route(
            "/api/letter-templates/:id",
            put(handlers::update_letter_template).delete(handlers::delete_letter_template),
        )
        .route("/api/admissions", post(handlers::admit))
        .route("/api/admissions/pre-admit", post(handlers::pre_admit))
        .route(
            "/api/admissions/change-to-inpatient",
            post(handlers::change_to_inpatient),
        )
        .route(
            "/api/admissions/cancel-pre-admit",
            post(handlers::cancel_pre_admit),
        )
        .route("/api/admissions/:id/transfer", post(handlers::transfer))
        .route(
            "/api/admissions/:id/cancel-admit",
            post(handlers::cancel_admit),
        )
        .route(
            "/api/admissions/:id/cancel-transfer",
            post(handlers::cancel_transfer),
        )
        .route(
            "/api/admissions/:id/leave-start",
            post(handlers::leave_start),
        )
        .route("/api/admissions/:id/leave-end", post(handlers::leave_end))
        .route("/api/admissions/:id/discharge", post(handlers::discharge))
        .route("/api/slots/:id/book", post(handlers::book_slot))
        .route(
            "/api/appointments/:id/cancel",
            post(handlers::cancel_appointment),
        )
        .route(
            "/api/appointments/:id/check-in",
            post(handlers::check_in_appointment),
        )
        .route(
            "/api/appointments/:id/complete",
            post(handlers::complete_appointment),
        )
        .route(
            "/api/waitlist",
            get(handlers::list_waitlist).post(handlers::add_to_waitlist),
        )
        .route("/api/waitlist/:id", delete(handlers::remove_from_waitlist))
        .route("/api/rtt/start", post(handlers::rtt_start))
        .route("/api/rtt/:id/pause", post(handlers::rtt_pause))
        .route("/api/rtt/:id/resume", post(handlers::rtt_resume))
        .route("/api/rtt/:id/stop", post(handlers::rtt_stop))
        .route(
            "/api/rtt/:id/weeks-waiting",
            get(handlers::rtt_weeks_waiting),
        )
        .route("/api/wards/:id/occupancy", get(handlers::ward_occupancy))
        .route("/api/beds/:id/status", put(handlers::set_bed_status))
        .route("/api/accounts", post(handlers::open_account))
        .route(
            "/api/accounts/:id/coverages",
            get(handlers::list_account_coverages),
        )
        .route("/api/coverages", post(handlers::create_coverage))
        .route(
            "/api/coverages/:id",
            get(handlers::get_coverage)
                .put(handlers::update_coverage)
                .delete(handlers::delete_coverage),
        )
        .route(
            "/api/patients/:id/coverages",
            get(handlers::list_patient_coverages),
        )
        .route("/api/charges", post(handlers::post_charge))
        .route("/api/invoices", post(handlers::finalize_invoice))
        .route("/api/payments", post(handlers::post_payment))
        .route("/api/letters/generate", post(handlers::generate_letter))
        .route("/api/letters/:id", get(handlers::get_generated_letter))
        .route("/api/letters/:id/sent", post(handlers::mark_letter_sent))
        .route(
            "/api/letters/:id/failed",
            post(handlers::mark_letter_failed),
        )
        .route("/api/schedules", post(handlers::create_schedule))
        .route(
            "/api/schedules/:id",
            get(handlers::get_schedule).delete(handlers::delete_schedule),
        )
        .route("/api/slots/:id", delete(handlers::delete_slot))
        .route(
            "/api/schedules/:id/slots",
            get(handlers::list_schedule_slots).post(handlers::create_slot),
        )
        .route(
            "/api/schedules/:id/slots/bulk",
            post(handlers::bulk_create_slots),
        )
        .route("/api/wards/:id/beds", get(handlers::list_ward_beds))
        .route(
            "/api/beds/:id",
            get(handlers::get_bed).put(handlers::update_bed),
        )
        .route("/api/encounters", post(handlers::create_encounter))
        .route(
            "/api/encounters/:id/cancel",
            post(handlers::cancel_encounter),
        )
        .route(
            "/api/encounters/:id/status",
            put(handlers::set_encounter_status),
        )
        .route(
            "/api/facilities",
            post(handlers::create_facility).get(handlers::list_facilities),
        )
        .route(
            "/api/wards",
            post(handlers::create_ward).get(handlers::list_wards),
        )
        .route("/api/rooms", post(handlers::create_room))
        .route("/api/beds", post(handlers::create_bed))
        .route("/api/waitlist/:id", put(handlers::update_waitlist_entry))
        .route("/api/waitlist/breaches", get(handlers::list_rtt_breaches))
        .with_state(state)
}
