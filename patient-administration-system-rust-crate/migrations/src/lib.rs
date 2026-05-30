pub use sea_orm_migration::prelude::*;

mod m20260520_000001_init;
mod m20260525_000002_outbox_dlq;
mod m20260526_000003_appointment_series;
mod m20260527_000004_coverage;
mod m20260528_000005_patient_replaced_by;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260520_000001_init::Migration),
            Box::new(m20260525_000002_outbox_dlq::Migration),
            Box::new(m20260526_000003_appointment_series::Migration),
            Box::new(m20260527_000004_coverage::Migration),
            Box::new(m20260528_000005_patient_replaced_by::Migration),
        ]
    }
}
