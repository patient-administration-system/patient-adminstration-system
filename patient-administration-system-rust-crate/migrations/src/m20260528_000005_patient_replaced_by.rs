//! Patient `replaced_by` / merge tombstone (v0.11.0).
//!
//! Adds a nullable `replaced_by UUID` column to `patients`. When the
//! sister MPI crate (or an operator) determines two patient rows refer
//! to the same person, one row becomes the *survivor* and the other
//! becomes a *tombstone* pointing at the survivor:
//!
//! - `tombstone.replaced_by = survivor.id`
//! - `tombstone.active = false`
//!
//! Tombstones are never hard-deleted — their `created_at` /
//! `updated_at` / sub-records remain so the audit trail can replay the
//! merge. Default list / search paths filter them out via
//! `replaced_by IS NULL`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Patients::Table)
                    .add_column(ColumnDef::new(Patients::ReplacedBy).uuid().null())
                    .to_owned(),
            )
            .await?;

        // Partial index for the inverse lookup: given a survivor id,
        // find every tombstone that points at it. Skips the (vast)
        // majority of rows whose replaced_by is NULL.
        let conn = manager.get_connection();
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS idx_patients_replaced_by \
             ON patients (replaced_by) WHERE replaced_by IS NOT NULL",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        conn.execute_unprepared("DROP INDEX IF EXISTS idx_patients_replaced_by")
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Patients::Table)
                    .drop_column(Patients::ReplacedBy)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Patients {
    Table,
    ReplacedBy,
}
