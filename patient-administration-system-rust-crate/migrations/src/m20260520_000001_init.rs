//! Initial migration for the Patient Administration System (PAS).
//!
//! PRAGMATIC SIMPLIFICATION (v0.1):
//! - Nested Vec data is stored inline on parent rows as JSONB columns instead
//!   of separate child tables, to keep the schema tractable in v0.1.
//! - `patients` stores `identifiers`, `additional_names`, `telecom`, `addresses`,
//!   `emergency_contacts` (and `name`) as JSONB columns.
//! - `practitioners` follows the same pattern for `identifiers`, `telecom`,
//!   `addresses`, and `name`.
//! - This eliminates auxiliary tables (`patient_identifiers`, `patient_names`,
//!   `patient_addresses`, `patient_contacts`, standalone `emergency_contacts`).
//! - This is a v0.1 trade-off; later versions can normalize.
//!
//! Decimal money columns are stored at the DB layer as `decimal(20,4)`.
//! Conversion to/from `crate::models::Money` happens in repositories.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ---------- patients ----------
        manager
            .create_table(
                Table::create()
                    .table(Patients::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Patients::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Patients::MpiId).uuid().null())
                    .col(
                        ColumnDef::new(Patients::Active)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(ColumnDef::new(Patients::Name).json_binary().not_null())
                    .col(
                        ColumnDef::new(Patients::AdditionalNames)
                            .json_binary()
                            .not_null()
                            .default(Expr::cust("'[]'::jsonb")),
                    )
                    .col(
                        ColumnDef::new(Patients::Identifiers)
                            .json_binary()
                            .not_null()
                            .default(Expr::cust("'[]'::jsonb")),
                    )
                    .col(
                        ColumnDef::new(Patients::Telecom)
                            .json_binary()
                            .not_null()
                            .default(Expr::cust("'[]'::jsonb")),
                    )
                    .col(
                        ColumnDef::new(Patients::Addresses)
                            .json_binary()
                            .not_null()
                            .default(Expr::cust("'[]'::jsonb")),
                    )
                    .col(ColumnDef::new(Patients::Gender).text().not_null())
                    .col(ColumnDef::new(Patients::BirthDate).date().null())
                    .col(
                        ColumnDef::new(Patients::Deceased)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Patients::DeceasedDatetime)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Patients::EmergencyContacts)
                            .json_binary()
                            .not_null()
                            .default(Expr::cust("'[]'::jsonb")),
                    )
                    .col(ColumnDef::new(Patients::MaritalStatus).text().null())
                    .col(
                        ColumnDef::new(Patients::DeletedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Patients::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Patients::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- practitioners ----------
        manager
            .create_table(
                Table::create()
                    .table(Practitioners::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Practitioners::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Practitioners::Active)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(ColumnDef::new(Practitioners::Name).json_binary().not_null())
                    .col(
                        ColumnDef::new(Practitioners::Identifiers)
                            .json_binary()
                            .not_null()
                            .default(Expr::cust("'[]'::jsonb")),
                    )
                    .col(
                        ColumnDef::new(Practitioners::Telecom)
                            .json_binary()
                            .not_null()
                            .default(Expr::cust("'[]'::jsonb")),
                    )
                    .col(
                        ColumnDef::new(Practitioners::Addresses)
                            .json_binary()
                            .not_null()
                            .default(Expr::cust("'[]'::jsonb")),
                    )
                    .col(ColumnDef::new(Practitioners::Gender).text().not_null())
                    .col(ColumnDef::new(Practitioners::BirthDate).date().null())
                    .col(
                        ColumnDef::new(Practitioners::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Practitioners::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- departments ----------
        manager
            .create_table(
                Table::create()
                    .table(Departments::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Departments::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Departments::FacilityId).uuid().not_null())
                    .col(ColumnDef::new(Departments::Name).text().not_null())
                    .col(ColumnDef::new(Departments::Code).text().not_null())
                    .col(
                        ColumnDef::new(Departments::Active)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(Departments::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Departments::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- practitioner_roles ----------
        manager
            .create_table(
                Table::create()
                    .table(PractitionerRoles::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PractitionerRoles::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PractitionerRoles::PractitionerId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PractitionerRoles::DepartmentId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PractitionerRoles::Role).text().not_null())
                    .col(ColumnDef::new(PractitionerRoles::Specialty).text().null())
                    .col(
                        ColumnDef::new(PractitionerRoles::Active)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(PractitionerRoles::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PractitionerRoles::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- facilities ----------
        manager
            .create_table(
                Table::create()
                    .table(Facilities::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Facilities::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Facilities::Name).text().not_null())
                    .col(
                        ColumnDef::new(Facilities::Code)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Facilities::Address).json_binary().not_null())
                    .col(
                        ColumnDef::new(Facilities::Active)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(Facilities::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Facilities::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- wards ----------
        manager
            .create_table(
                Table::create()
                    .table(Wards::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Wards::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Wards::FacilityId).uuid().not_null())
                    .col(ColumnDef::new(Wards::Name).text().not_null())
                    .col(ColumnDef::new(Wards::Code).text().not_null())
                    .col(ColumnDef::new(Wards::Capacity).integer().not_null())
                    .col(
                        ColumnDef::new(Wards::Active)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(Wards::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Wards::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .index(
                        Index::create()
                            .name("uq_wards_facility_code")
                            .col(Wards::FacilityId)
                            .col(Wards::Code)
                            .unique(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- rooms ----------
        manager
            .create_table(
                Table::create()
                    .table(Rooms::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Rooms::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Rooms::WardId).uuid().not_null())
                    .col(ColumnDef::new(Rooms::Name).text().not_null())
                    .col(ColumnDef::new(Rooms::Code).text().not_null())
                    .col(
                        ColumnDef::new(Rooms::Active)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(Rooms::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Rooms::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .index(
                        Index::create()
                            .name("uq_rooms_ward_code")
                            .col(Rooms::WardId)
                            .col(Rooms::Code)
                            .unique(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- beds ----------
        manager
            .create_table(
                Table::create()
                    .table(Beds::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Beds::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Beds::RoomId).uuid().not_null())
                    .col(ColumnDef::new(Beds::Name).text().not_null())
                    .col(ColumnDef::new(Beds::Code).text().not_null())
                    .col(
                        ColumnDef::new(Beds::Status)
                            .text()
                            .not_null()
                            .default("available"),
                    )
                    .col(
                        ColumnDef::new(Beds::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Beds::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .index(
                        Index::create()
                            .name("uq_beds_room_code")
                            .col(Beds::RoomId)
                            .col(Beds::Code)
                            .unique(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- encounters ----------
        manager
            .create_table(
                Table::create()
                    .table(Encounters::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Encounters::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Encounters::PatientId).uuid().not_null())
                    .col(ColumnDef::new(Encounters::Class).text().not_null())
                    .col(ColumnDef::new(Encounters::Status).text().not_null())
                    .col(
                        ColumnDef::new(Encounters::PeriodStart)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Encounters::PeriodEnd)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(ColumnDef::new(Encounters::PractitionerId).uuid().null())
                    .col(ColumnDef::new(Encounters::DepartmentId).uuid().null())
                    .col(ColumnDef::new(Encounters::Reason).text().null())
                    .col(
                        ColumnDef::new(Encounters::DeletedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Encounters::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Encounters::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- admissions ----------
        manager
            .create_table(
                Table::create()
                    .table(Admissions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Admissions::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Admissions::EncounterId).uuid().not_null())
                    .col(ColumnDef::new(Admissions::BedId).uuid().not_null())
                    .col(
                        ColumnDef::new(Admissions::AdmittingPractitionerId)
                            .uuid()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Admissions::AdmittedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Admissions::Reason).text().null())
                    .col(
                        ColumnDef::new(Admissions::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Admissions::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- transfers ----------
        manager
            .create_table(
                Table::create()
                    .table(Transfers::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Transfers::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Transfers::AdmissionId).uuid().not_null())
                    .col(ColumnDef::new(Transfers::FromBedId).uuid().not_null())
                    .col(ColumnDef::new(Transfers::ToBedId).uuid().not_null())
                    .col(ColumnDef::new(Transfers::Reason).text().null())
                    .col(
                        ColumnDef::new(Transfers::TransferredAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Transfers::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- discharges ----------
        manager
            .create_table(
                Table::create()
                    .table(Discharges::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Discharges::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Discharges::AdmissionId).uuid().not_null())
                    .col(
                        ColumnDef::new(Discharges::DischargingPractitionerId)
                            .uuid()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Discharges::DischargedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Discharges::Disposition).text().null())
                    .col(ColumnDef::new(Discharges::Notes).text().null())
                    .col(
                        ColumnDef::new(Discharges::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- bed_assignments ----------
        manager
            .create_table(
                Table::create()
                    .table(BedAssignments::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(BedAssignments::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(BedAssignments::EncounterId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(BedAssignments::BedId).uuid().not_null())
                    .col(
                        ColumnDef::new(BedAssignments::AssignedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(BedAssignments::ReleasedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(BedAssignments::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(BedAssignments::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- schedules ----------
        manager
            .create_table(
                Table::create()
                    .table(Schedules::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Schedules::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Schedules::OwnerKind).text().not_null())
                    .col(ColumnDef::new(Schedules::OwnerId).uuid().not_null())
                    .col(ColumnDef::new(Schedules::ServiceType).text().not_null())
                    .col(
                        ColumnDef::new(Schedules::Active)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(Schedules::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Schedules::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- slots ----------
        manager
            .create_table(
                Table::create()
                    .table(Slots::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Slots::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Slots::ScheduleId).uuid().not_null())
                    .col(
                        ColumnDef::new(Slots::StartDatetime)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Slots::EndDatetime)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Slots::Status)
                            .text()
                            .not_null()
                            .default("free"),
                    )
                    .col(
                        ColumnDef::new(Slots::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Slots::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- appointments ----------
        manager
            .create_table(
                Table::create()
                    .table(Appointments::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Appointments::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Appointments::PatientId).uuid().not_null())
                    .col(ColumnDef::new(Appointments::SlotId).uuid().null())
                    .col(ColumnDef::new(Appointments::PractitionerId).uuid().null())
                    .col(
                        ColumnDef::new(Appointments::StartDatetime)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Appointments::EndDatetime)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Appointments::Status).text().not_null())
                    .col(ColumnDef::new(Appointments::Reason).text().null())
                    .col(
                        ColumnDef::new(Appointments::FromWaitlistEntryId)
                            .uuid()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Appointments::CancellationReason)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Appointments::DeletedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Appointments::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Appointments::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- referrals ----------
        manager
            .create_table(
                Table::create()
                    .table(Referrals::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Referrals::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Referrals::PatientId).uuid().not_null())
                    .col(
                        ColumnDef::new(Referrals::ReferringPractitionerId)
                            .uuid()
                            .null(),
                    )
                    .col(ColumnDef::new(Referrals::TargetService).text().not_null())
                    .col(ColumnDef::new(Referrals::Reason).text().null())
                    .col(
                        ColumnDef::new(Referrals::ReceivedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Referrals::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- waitlist_entries ----------
        manager
            .create_table(
                Table::create()
                    .table(WaitlistEntries::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(WaitlistEntries::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(WaitlistEntries::ReferralId).uuid().null())
                    .col(ColumnDef::new(WaitlistEntries::PatientId).uuid().not_null())
                    .col(
                        ColumnDef::new(WaitlistEntries::TargetService)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(WaitlistEntries::Priority).text().not_null())
                    .col(
                        ColumnDef::new(WaitlistEntries::Status)
                            .text()
                            .not_null()
                            .default("waiting"),
                    )
                    .col(
                        ColumnDef::new(WaitlistEntries::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WaitlistEntries::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- rtt_pathways ----------
        manager
            .create_table(
                Table::create()
                    .table(RttPathways::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RttPathways::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(RttPathways::PatientId).uuid().not_null())
                    .col(ColumnDef::new(RttPathways::TargetService).text().not_null())
                    .col(
                        ColumnDef::new(RttPathways::BreachWeeks)
                            .integer()
                            .not_null()
                            .default(18),
                    )
                    .col(
                        ColumnDef::new(RttPathways::Status)
                            .text()
                            .not_null()
                            .default("active"),
                    )
                    .col(
                        ColumnDef::new(RttPathways::StartedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RttPathways::StoppedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(RttPathways::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RttPathways::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- rtt_clock_events ----------
        manager
            .create_table(
                Table::create()
                    .table(RttClockEvents::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RttClockEvents::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(RttClockEvents::PathwayId).uuid().not_null())
                    .col(ColumnDef::new(RttClockEvents::Kind).text().not_null())
                    .col(ColumnDef::new(RttClockEvents::Reason).text().null())
                    .col(
                        ColumnDef::new(RttClockEvents::EventAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RttClockEvents::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- letter_templates ----------
        manager
            .create_table(
                Table::create()
                    .table(LetterTemplates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(LetterTemplates::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(LetterTemplates::Name).text().not_null())
                    .col(ColumnDef::new(LetterTemplates::Subject).text().not_null())
                    .col(ColumnDef::new(LetterTemplates::BodyTera).text().not_null())
                    .col(
                        ColumnDef::new(LetterTemplates::RequiredVariables)
                            .json_binary()
                            .not_null()
                            .default(Expr::cust("'[]'::jsonb")),
                    )
                    .col(
                        ColumnDef::new(LetterTemplates::Channels)
                            .json_binary()
                            .not_null()
                            .default(Expr::cust("'[]'::jsonb")),
                    )
                    .col(
                        ColumnDef::new(LetterTemplates::Active)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(LetterTemplates::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LetterTemplates::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- generated_letters ----------
        manager
            .create_table(
                Table::create()
                    .table(GeneratedLetters::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(GeneratedLetters::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(GeneratedLetters::TemplateId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GeneratedLetters::PatientId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GeneratedLetters::AppointmentId)
                            .uuid()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(GeneratedLetters::RenderedSubject)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GeneratedLetters::RenderedBody)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(GeneratedLetters::Channel).text().not_null())
                    .col(
                        ColumnDef::new(GeneratedLetters::Status)
                            .text()
                            .not_null()
                            .default("pending"),
                    )
                    .col(
                        ColumnDef::new(GeneratedLetters::SentAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(GeneratedLetters::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GeneratedLetters::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- accounts ----------
        manager
            .create_table(
                Table::create()
                    .table(Accounts::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Accounts::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Accounts::PatientId).uuid().not_null())
                    .col(
                        ColumnDef::new(Accounts::Status)
                            .text()
                            .not_null()
                            .default("open"),
                    )
                    .col(ColumnDef::new(Accounts::Currency).text().not_null())
                    .col(
                        ColumnDef::new(Accounts::OpenedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Accounts::ClosedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Accounts::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Accounts::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- charges ----------
        manager
            .create_table(
                Table::create()
                    .table(Charges::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Charges::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Charges::AccountId).uuid().not_null())
                    .col(ColumnDef::new(Charges::EncounterId).uuid().null())
                    .col(ColumnDef::new(Charges::AppointmentId).uuid().null())
                    .col(ColumnDef::new(Charges::Code).text().not_null())
                    .col(ColumnDef::new(Charges::Description).text().not_null())
                    // Stored as TEXT (matches `db::entities::charge::Model::
                    // amount_value: String`). The repo converts to/from
                    // `rust_decimal::Decimal` at the domain boundary.
                    // Using TEXT rather than NUMERIC sidesteps the
                    // sea-orm/Postgres type mismatch you'd hit if the
                    // column were `numeric` and the entity were `String`.
                    .col(ColumnDef::new(Charges::AmountValue).text().not_null())
                    .col(ColumnDef::new(Charges::AmountCurrency).text().not_null())
                    .col(
                        ColumnDef::new(Charges::PostedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Charges::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- invoices ----------
        manager
            .create_table(
                Table::create()
                    .table(Invoices::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Invoices::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Invoices::AccountId).uuid().not_null())
                    .col(
                        ColumnDef::new(Invoices::Status)
                            .text()
                            .not_null()
                            .default("draft"),
                    )
                    // Same rationale as `charges.amount_value` — TEXT
                    // matches `db::entities::invoice::Model`.
                    .col(ColumnDef::new(Invoices::TotalValue).text().not_null())
                    .col(ColumnDef::new(Invoices::TotalCurrency).text().not_null())
                    .col(
                        ColumnDef::new(Invoices::ChargeIds)
                            .json_binary()
                            .not_null()
                            .default(Expr::cust("'[]'::jsonb")),
                    )
                    .col(
                        ColumnDef::new(Invoices::FinalizedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Invoices::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Invoices::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- payments ----------
        manager
            .create_table(
                Table::create()
                    .table(Payments::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Payments::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Payments::InvoiceId).uuid().not_null())
                    // Same rationale as `charges.amount_value` — TEXT
                    // matches `db::entities::payment::Model`.
                    .col(ColumnDef::new(Payments::AmountValue).text().not_null())
                    .col(ColumnDef::new(Payments::AmountCurrency).text().not_null())
                    .col(ColumnDef::new(Payments::Method).text().not_null())
                    .col(ColumnDef::new(Payments::Reference).text().null())
                    .col(
                        ColumnDef::new(Payments::PostedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Payments::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- consents ----------
        manager
            .create_table(
                Table::create()
                    .table(Consents::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Consents::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Consents::PatientId).uuid().not_null())
                    .col(ColumnDef::new(Consents::ConsentType).text().not_null())
                    .col(
                        ColumnDef::new(Consents::Status)
                            .text()
                            .not_null()
                            .default("active"),
                    )
                    .col(ColumnDef::new(Consents::GrantedDate).date().not_null())
                    .col(ColumnDef::new(Consents::ExpiryDate).date().null())
                    .col(ColumnDef::new(Consents::RevokedDate).date().null())
                    .col(ColumnDef::new(Consents::Purpose).text().null())
                    .col(ColumnDef::new(Consents::Method).text().null())
                    .col(
                        ColumnDef::new(Consents::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Consents::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- audit_log ----------
        manager
            .create_table(
                Table::create()
                    .table(AuditLog::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(AuditLog::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(AuditLog::EntityType).text().not_null())
                    .col(ColumnDef::new(AuditLog::EntityId).uuid().not_null())
                    .col(ColumnDef::new(AuditLog::Action).text().not_null())
                    .col(ColumnDef::new(AuditLog::OldValue).json_binary().null())
                    .col(ColumnDef::new(AuditLog::NewValue).json_binary().null())
                    .col(ColumnDef::new(AuditLog::UserId).text().null())
                    .col(ColumnDef::new(AuditLog::UserIp).text().null())
                    .col(ColumnDef::new(AuditLog::UserAgent).text().null())
                    .col(
                        ColumnDef::new(AuditLog::At)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::cust("now()")),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- outbox_events ----------
        manager
            .create_table(
                Table::create()
                    .table(OutboxEvents::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(OutboxEvents::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(OutboxEvents::EventType).text().not_null())
                    .col(
                        ColumnDef::new(OutboxEvents::Payload)
                            .json_binary()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OutboxEvents::Published)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(OutboxEvents::At)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::cust("now()")),
                    )
                    .to_owned(),
            )
            .await?;

        // ---------- Non-partial indexes (via schema builder) ----------
        manager
            .create_index(
                Index::create()
                    .name("idx_appointments_patient_start")
                    .table(Appointments::Table)
                    .col(Appointments::PatientId)
                    .col(Appointments::StartDatetime)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_appointments_practitioner_start")
                    .table(Appointments::Table)
                    .col(Appointments::PractitionerId)
                    .col(Appointments::StartDatetime)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_encounters_patient_status")
                    .table(Encounters::Table)
                    .col(Encounters::PatientId)
                    .col(Encounters::Status)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_waitlist_service_priority")
                    .table(WaitlistEntries::Table)
                    .col(WaitlistEntries::TargetService)
                    .col(WaitlistEntries::Priority)
                    .col(WaitlistEntries::CreatedAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_rtt_clock_events_pathway_at")
                    .table(RttClockEvents::Table)
                    .col(RttClockEvents::PathwayId)
                    .col(RttClockEvents::EventAt)
                    .to_owned(),
            )
            .await?;

        // ---------- Partial indexes (raw SQL) ----------
        let conn = manager.get_connection();
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS idx_patients_active \
             ON patients (active) WHERE deleted_at IS NULL",
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS idx_slots_schedule_start_free \
             ON slots (schedule_id, start_datetime) WHERE status = 'free'",
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS idx_bed_assignments_active \
             ON bed_assignments (bed_id) WHERE released_at IS NULL",
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS idx_outbox_unpublished \
             ON outbox_events (id) WHERE published = false",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop partial indexes (raw SQL) first
        let conn = manager.get_connection();
        conn.execute_unprepared("DROP INDEX IF EXISTS idx_outbox_unpublished")
            .await?;
        conn.execute_unprepared("DROP INDEX IF EXISTS idx_bed_assignments_active")
            .await?;
        conn.execute_unprepared("DROP INDEX IF EXISTS idx_slots_schedule_start_free")
            .await?;
        conn.execute_unprepared("DROP INDEX IF EXISTS idx_patients_active")
            .await?;

        // Drop tables in reverse order
        manager
            .drop_table(Table::drop().table(OutboxEvents::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(AuditLog::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Consents::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Payments::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Invoices::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Charges::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Accounts::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(GeneratedLetters::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(LetterTemplates::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(RttClockEvents::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(RttPathways::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(WaitlistEntries::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Referrals::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Appointments::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Slots::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Schedules::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(BedAssignments::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Discharges::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Transfers::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Admissions::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Encounters::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Beds::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Rooms::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Wards::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Facilities::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(PractitionerRoles::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Departments::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Practitioners::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Patients::Table).to_owned())
            .await?;

        Ok(())
    }
}

// ---------- Identifier enums (Iden) ----------

#[derive(DeriveIden)]
enum Patients {
    Table,
    Id,
    MpiId,
    Active,
    Name,
    AdditionalNames,
    Identifiers,
    Telecom,
    Addresses,
    Gender,
    BirthDate,
    Deceased,
    DeceasedDatetime,
    EmergencyContacts,
    MaritalStatus,
    DeletedAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Practitioners {
    Table,
    Id,
    Active,
    Name,
    Identifiers,
    Telecom,
    Addresses,
    Gender,
    BirthDate,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Departments {
    Table,
    Id,
    FacilityId,
    Name,
    Code,
    Active,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum PractitionerRoles {
    Table,
    Id,
    PractitionerId,
    DepartmentId,
    Role,
    Specialty,
    Active,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Facilities {
    Table,
    Id,
    Name,
    Code,
    Address,
    Active,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Wards {
    Table,
    Id,
    FacilityId,
    Name,
    Code,
    Capacity,
    Active,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Rooms {
    Table,
    Id,
    WardId,
    Name,
    Code,
    Active,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Beds {
    Table,
    Id,
    RoomId,
    Name,
    Code,
    Status,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Encounters {
    Table,
    Id,
    PatientId,
    Class,
    Status,
    PeriodStart,
    PeriodEnd,
    PractitionerId,
    DepartmentId,
    Reason,
    DeletedAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Admissions {
    Table,
    Id,
    EncounterId,
    BedId,
    AdmittingPractitionerId,
    AdmittedAt,
    Reason,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Transfers {
    Table,
    Id,
    AdmissionId,
    FromBedId,
    ToBedId,
    Reason,
    TransferredAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Discharges {
    Table,
    Id,
    AdmissionId,
    DischargingPractitionerId,
    DischargedAt,
    Disposition,
    Notes,
    CreatedAt,
}

#[derive(DeriveIden)]
enum BedAssignments {
    Table,
    Id,
    EncounterId,
    BedId,
    AssignedAt,
    ReleasedAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Schedules {
    Table,
    Id,
    OwnerKind,
    OwnerId,
    ServiceType,
    Active,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Slots {
    Table,
    Id,
    ScheduleId,
    StartDatetime,
    EndDatetime,
    Status,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Appointments {
    Table,
    Id,
    PatientId,
    SlotId,
    PractitionerId,
    StartDatetime,
    EndDatetime,
    Status,
    Reason,
    FromWaitlistEntryId,
    CancellationReason,
    DeletedAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Referrals {
    Table,
    Id,
    PatientId,
    ReferringPractitionerId,
    TargetService,
    Reason,
    ReceivedAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum WaitlistEntries {
    Table,
    Id,
    ReferralId,
    PatientId,
    TargetService,
    Priority,
    Status,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum RttPathways {
    Table,
    Id,
    PatientId,
    TargetService,
    BreachWeeks,
    Status,
    StartedAt,
    StoppedAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum RttClockEvents {
    Table,
    Id,
    PathwayId,
    Kind,
    Reason,
    EventAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum LetterTemplates {
    Table,
    Id,
    Name,
    Subject,
    BodyTera,
    RequiredVariables,
    Channels,
    Active,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum GeneratedLetters {
    Table,
    Id,
    TemplateId,
    PatientId,
    AppointmentId,
    RenderedSubject,
    RenderedBody,
    Channel,
    Status,
    SentAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Accounts {
    Table,
    Id,
    PatientId,
    Status,
    Currency,
    OpenedAt,
    ClosedAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Charges {
    Table,
    Id,
    AccountId,
    EncounterId,
    AppointmentId,
    Code,
    Description,
    AmountValue,
    AmountCurrency,
    PostedAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Invoices {
    Table,
    Id,
    AccountId,
    Status,
    TotalValue,
    TotalCurrency,
    ChargeIds,
    FinalizedAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Payments {
    Table,
    Id,
    InvoiceId,
    AmountValue,
    AmountCurrency,
    Method,
    Reference,
    PostedAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Consents {
    Table,
    Id,
    PatientId,
    ConsentType,
    Status,
    GrantedDate,
    ExpiryDate,
    RevokedDate,
    Purpose,
    Method,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum AuditLog {
    Table,
    Id,
    EntityType,
    EntityId,
    Action,
    OldValue,
    NewValue,
    UserId,
    UserIp,
    UserAgent,
    At,
}

#[derive(DeriveIden)]
enum OutboxEvents {
    Table,
    Id,
    EventType,
    Payload,
    Published,
    At,
}
