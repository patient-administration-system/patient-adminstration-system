//! Outbox dead-letter queue (v0.5.0).
//!
//! Extends `outbox_events` with retry tracking (`retry_count`,
//! `last_attempted_at`, `last_error`) and adds the `outbox_dead_letters`
//! table. When a publish has failed `PAS_OUTBOX_MAX_RETRIES` times, the
//! dispatcher moves the row from `outbox_events` to `outbox_dead_letters`
//! so the hot path stays cheap and an operator can review / replay the
//! failure via the admin endpoints.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // --- Extend outbox_events with retry-tracking columns. ---
        manager
            .alter_table(
                Table::alter()
                    .table(OutboxEvents::Table)
                    .add_column(
                        ColumnDef::new(OutboxEvents::RetryCount)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(OutboxEvents::Table)
                    .add_column(
                        ColumnDef::new(OutboxEvents::LastAttemptedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(OutboxEvents::Table)
                    .add_column(ColumnDef::new(OutboxEvents::LastError).text().null())
                    .to_owned(),
            )
            .await?;

        // --- Create outbox_dead_letters. ---
        manager
            .create_table(
                Table::create()
                    .table(OutboxDeadLetters::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(OutboxDeadLetters::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(OutboxDeadLetters::OriginalId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OutboxDeadLetters::EventType)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OutboxDeadLetters::Payload)
                            .json_binary()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OutboxDeadLetters::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OutboxDeadLetters::DeadLetteredAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::cust("now()")),
                    )
                    .col(
                        ColumnDef::new(OutboxDeadLetters::RetryCount)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OutboxDeadLetters::LastError)
                            .text()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Newest-first listing index for the admin endpoint.
        manager
            .create_index(
                Index::create()
                    .name("idx_outbox_dead_letters_at")
                    .table(OutboxDeadLetters::Table)
                    .col(OutboxDeadLetters::DeadLetteredAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(OutboxDeadLetters::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(OutboxEvents::Table)
                    .drop_column(OutboxEvents::RetryCount)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(OutboxEvents::Table)
                    .drop_column(OutboxEvents::LastAttemptedAt)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(OutboxEvents::Table)
                    .drop_column(OutboxEvents::LastError)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum OutboxEvents {
    Table,
    RetryCount,
    LastAttemptedAt,
    LastError,
}

#[derive(DeriveIden)]
enum OutboxDeadLetters {
    Table,
    Id,
    OriginalId,
    EventType,
    Payload,
    CreatedAt,
    DeadLetteredAt,
    RetryCount,
    LastError,
}
