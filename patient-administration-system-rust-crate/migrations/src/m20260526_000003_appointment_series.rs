//! Recurring appointment series (v0.9.0).
//!
//! Adds the `appointment_series` table (the plan) and a nullable
//! `series_id` column on `appointments` (the link from each occurrence
//! back to its series). Occurrences are concrete `appointments` rows
//! generated at series-create time; they remain singleton rows in every
//! respect except for this backlink.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // --- appointment_series ---
        manager
            .create_table(
                Table::create()
                    .table(AppointmentSeries::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AppointmentSeries::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AppointmentSeries::PatientId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppointmentSeries::PractitionerId)
                            .uuid()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AppointmentSeries::ServiceType)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppointmentSeries::StartDatetime)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppointmentSeries::DurationMinutes)
                            .integer()
                            .not_null(),
                    )
                    // Whole recurrence rule (frequency / interval / by_weekday /
                    // end) encoded as one JSONB blob — no need for a separate
                    // table; we never query into it.
                    .col(
                        ColumnDef::new(AppointmentSeries::Rule)
                            .json_binary()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AppointmentSeries::Status).text().not_null())
                    .col(ColumnDef::new(AppointmentSeries::Reason).text().null())
                    .col(
                        ColumnDef::new(AppointmentSeries::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::cust("now()")),
                    )
                    .col(
                        ColumnDef::new(AppointmentSeries::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::cust("now()")),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_appointment_series_patient")
                    .table(AppointmentSeries::Table)
                    .col(AppointmentSeries::PatientId)
                    .to_owned(),
            )
            .await?;

        // --- appointments.series_id ---
        manager
            .alter_table(
                Table::alter()
                    .table(Appointments::Table)
                    .add_column(ColumnDef::new(Appointments::SeriesId).uuid().null())
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_appointments_series")
                    .table(Appointments::Table)
                    .col(Appointments::SeriesId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop in reverse order so the index goes before its column.
        manager
            .drop_index(
                Index::drop()
                    .name("idx_appointments_series")
                    .table(Appointments::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Appointments::Table)
                    .drop_column(Appointments::SeriesId)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(AppointmentSeries::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum AppointmentSeries {
    Table,
    Id,
    PatientId,
    PractitionerId,
    ServiceType,
    StartDatetime,
    DurationMinutes,
    Rule,
    Status,
    Reason,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Appointments {
    Table,
    SeriesId,
}
