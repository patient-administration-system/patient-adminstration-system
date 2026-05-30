//! resources — Bed/Ward operations service

use sea_orm::{DatabaseConnection, TransactionTrait};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::{
    audit::{AuditLogRepository, UserContext},
    bed::BedRepository,
    outbox::OutboxRepository,
};
use crate::models::facility::{Bed, BedStatus};
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct WardOccupancy {
    pub ward_id: Uuid,
    pub total_beds: usize,
    pub available: usize,
    pub occupied: usize,
    pub cleaning: usize,
    pub reserved: usize,
    pub out_of_service: usize,
}

pub struct ResourcesService {
    db: DatabaseConnection,
    publisher: Arc<dyn EventPublisher>,
}

impl ResourcesService {
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self { db, publisher }
    }

    pub async fn ward_occupancy(&self, ward_id: Uuid) -> Result<WardOccupancy> {
        let beds = BedRepository::list_by_ward(&self.db, ward_id).await?;
        Ok(tally_occupancy(ward_id, &beds))
    }

    pub async fn set_bed_status(
        &self,
        bed_id: Uuid,
        new_status: BedStatus,
        ctx: &UserContext,
    ) -> Result<Bed> {
        let ctx_clone = ctx.clone();
        let bed = self
            .db
            .transaction::<_, Bed, Error>(|txn| {
                Box::pin(async move {
                    let bed = BedRepository::update_status(txn, bed_id, new_status).await?;
                    AuditLogRepository::log(
                        txn,
                        "bed",
                        bed_id,
                        "set_status",
                        None,
                        Some(serde_json::json!({ "status": format!("{:?}", new_status) })),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "BedStatusChanged",
                        &serde_json::json!({
                            "bed_id": bed_id,
                            "status": format!("{:?}", new_status),
                        }),
                    )
                    .await?;
                    if new_status == BedStatus::OutOfService {
                        OutboxRepository::publish(
                            txn,
                            "BedRetired",
                            &serde_json::json!({
                                "bed_id": bed_id,
                                "bed_code": bed.code.clone(),
                            }),
                        )
                        .await?;
                    }
                    Ok(bed)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "BedStatusChanged",
                serde_json::json!({ "bed_id": bed_id }),
            ))
            .await;
        Ok(bed)
    }
}

fn tally_occupancy(ward_id: Uuid, beds: &[Bed]) -> WardOccupancy {
    let mut occ = WardOccupancy {
        ward_id,
        total_beds: beds.len(),
        available: 0,
        occupied: 0,
        cleaning: 0,
        reserved: 0,
        out_of_service: 0,
    };
    for b in beds {
        match b.status {
            BedStatus::Available => occ.available += 1,
            BedStatus::Occupied => occ.occupied += 1,
            BedStatus::Cleaning => occ.cleaning += 1,
            BedStatus::Reserved => occ.reserved += 1,
            BedStatus::OutOfService => occ.out_of_service += 1,
        }
    }
    occ
}

fn unwrap_txn_err(e: sea_orm::TransactionError<Error>) -> Error {
    match e {
        sea_orm::TransactionError::Connection(c) => Error::Database(c),
        sea_orm::TransactionError::Transaction(t) => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bed(status: BedStatus) -> Bed {
        let mut b = Bed::new(Uuid::new_v4(), "B1".into(), "B1".into());
        b.status = status;
        b
    }

    #[test]
    fn test_tally_occupancy_mixed() {
        let ward_id = Uuid::new_v4();
        let beds = vec![
            make_bed(BedStatus::Available),
            make_bed(BedStatus::Available),
            make_bed(BedStatus::Occupied),
            make_bed(BedStatus::Cleaning),
            make_bed(BedStatus::OutOfService),
        ];
        let occ = tally_occupancy(ward_id, &beds);
        assert_eq!(occ.total_beds, 5);
        assert_eq!(occ.available, 2);
        assert_eq!(occ.occupied, 1);
        assert_eq!(occ.cleaning, 1);
        assert_eq!(occ.out_of_service, 1);
        assert_eq!(occ.reserved, 0);
    }
}
