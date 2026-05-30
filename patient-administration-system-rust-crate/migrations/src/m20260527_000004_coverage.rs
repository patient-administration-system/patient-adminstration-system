//! Coverage / insurance records (v0.10.0).
//!
//! Adds the `coverages` table — one row per insurance / self-pay /
//! other-payer record carried by a patient. Optionally linked to a
//! billing `accounts.id` so invoices can surface payer detail.
//!
//! No `deleted_at` column on this table — invariant §5.3 keeps soft-
//! delete to patients/encounters/appointments only. Retirement is via
//! `status = 'cancelled'` or `status = 'entered_in_error'`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Coverages::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Coverages::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Coverages::PatientId).uuid().not_null())
                    .col(ColumnDef::new(Coverages::AccountId).uuid().null())
                    .col(ColumnDef::new(Coverages::Status).text().not_null())
                    .col(ColumnDef::new(Coverages::Kind).text().not_null())
                    .col(ColumnDef::new(Coverages::SubscriberId).uuid().null())
                    .col(ColumnDef::new(Coverages::PayorName).text().not_null())
                    .col(ColumnDef::new(Coverages::PayorIdentifier).text().null())
                    .col(ColumnDef::new(Coverages::PolicyNumber).text().not_null())
                    .col(ColumnDef::new(Coverages::GroupNumber).text().null())
                    .col(ColumnDef::new(Coverages::Relationship).text().not_null())
                    .col(ColumnDef::new(Coverages::StartDate).date().not_null())
                    .col(ColumnDef::new(Coverages::EndDate).date().null())
                    .col(
                        ColumnDef::new(Coverages::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::cust("now()")),
                    )
                    .col(
                        ColumnDef::new(Coverages::UpdatedAt)
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
                    .name("idx_coverages_patient")
                    .table(Coverages::Table)
                    .col(Coverages::PatientId)
                    .to_owned(),
            )
            .await?;

        // Partial index: only coverage rows attached to an account get
        // indexed by account_id. Most are at create time still unattached,
        // so this keeps the index lean.
        let conn = manager.get_connection();
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS idx_coverages_account \
             ON coverages (account_id) WHERE account_id IS NOT NULL",
        )
        .await?;

        // Partial uniqueness: at most one active row per
        // (patient, payor, policy). Doesn't prevent active duplicates
        // with the same policy number from different payors (legal —
        // e.g. switching plans mid-year).
        conn.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS uq_coverages_active_policy \
             ON coverages (patient_id, payor_name, policy_number) \
             WHERE status = 'active'",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        conn.execute_unprepared("DROP INDEX IF EXISTS uq_coverages_active_policy")
            .await?;
        conn.execute_unprepared("DROP INDEX IF EXISTS idx_coverages_account")
            .await?;
        manager
            .drop_table(Table::drop().table(Coverages::Table).if_exists().to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Coverages {
    Table,
    Id,
    PatientId,
    AccountId,
    Status,
    Kind,
    SubscriberId,
    PayorName,
    PayorIdentifier,
    PolicyNumber,
    GroupNumber,
    Relationship,
    StartDate,
    EndDate,
    CreatedAt,
    UpdatedAt,
}
