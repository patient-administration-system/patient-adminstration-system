//! waitlist — Waitlist and RTT clock services

use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set, TransactionTrait};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::entities::rtt_pathway;
use crate::db::repositories::{
    audit::{AuditLogRepository, UserContext},
    outbox::OutboxRepository,
    rtt::RttRepository,
    waitlist::WaitlistRepository,
};
use crate::models::rtt::{RTTClockEvent, RTTPathway, RTTStatus, compute_active_weeks};
use crate::models::waitlist::{Priority, WaitlistEntry, WaitlistStatus};
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

pub struct WaitlistService {
    db: DatabaseConnection,
    publisher: Arc<dyn EventPublisher>,
}

impl WaitlistService {
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self { db, publisher }
    }

    pub async fn add(
        &self,
        referral_id: Option<Uuid>,
        patient_id: Uuid,
        target_service: String,
        priority: Priority,
        ctx: &UserContext,
    ) -> Result<WaitlistEntry> {
        let ctx_clone = ctx.clone();
        let mut entry = WaitlistEntry::new(patient_id, target_service, priority);
        entry.referral_id = referral_id;
        let entry_clone = entry.clone();
        let res = self
            .db
            .transaction::<_, WaitlistEntry, Error>(|txn| {
                Box::pin(async move {
                    let e = WaitlistRepository::create_entry(txn, &entry_clone).await?;
                    AuditLogRepository::log(
                        txn,
                        "waitlist_entry",
                        e.id,
                        "add",
                        None,
                        Some(serde_json::to_value(&e).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "WaitlistAdded",
                        &serde_json::json!({
                            "entry_id": e.id, "patient_id": patient_id,
                        }),
                    )
                    .await?;
                    Ok(e)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "WaitlistAdded",
                serde_json::json!({ "entry_id": res.id }),
            ))
            .await;
        Ok(res)
    }

    pub async fn remove(&self, entry_id: Uuid, ctx: &UserContext) -> Result<WaitlistEntry> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .transaction::<_, WaitlistEntry, Error>(|txn| {
                Box::pin(async move {
                    let mut e = WaitlistRepository::find_entry_by_id(txn, entry_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("waitlist entry {entry_id}")))?;
                    e.status = WaitlistStatus::Removed;
                    e.updated_at = chrono::Utc::now();
                    let e = WaitlistRepository::update_entry(txn, &e).await?;
                    AuditLogRepository::log(
                        txn,
                        "waitlist_entry",
                        e.id,
                        "remove",
                        None,
                        None,
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "WaitlistRemoved",
                        &serde_json::json!({ "entry_id": e.id }),
                    )
                    .await?;
                    Ok(e)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        Ok(res)
    }

    pub async fn list_by_service(&self, service: &str) -> Result<Vec<WaitlistEntry>> {
        WaitlistRepository::list_by_service(&self.db, service).await
    }
}

pub struct RttService {
    db: DatabaseConnection,
    publisher: Arc<dyn EventPublisher>,
}

impl RttService {
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self { db, publisher }
    }

    pub async fn start(
        &self,
        patient_id: Uuid,
        target_service: String,
        ctx: &UserContext,
    ) -> Result<RTTPathway> {
        let ctx_clone = ctx.clone();
        let pathway = RTTPathway::new(patient_id, target_service);
        let pathway_clone = pathway.clone();
        let res = self
            .db
            .transaction::<_, RTTPathway, Error>(|txn| {
                Box::pin(async move {
                    let p = RttRepository::create_pathway(txn, &pathway_clone).await?;
                    let ev = RTTClockEvent::started(p.id);
                    RttRepository::create_event(txn, &ev).await?;
                    AuditLogRepository::log(
                        txn,
                        "rtt_pathway",
                        p.id,
                        "start",
                        None,
                        Some(serde_json::to_value(&p).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "RTTClockStarted",
                        &serde_json::json!({ "pathway_id": p.id }),
                    )
                    .await?;
                    Ok(p)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "RTTClockStarted",
                serde_json::json!({ "pathway_id": res.id }),
            ))
            .await;
        Ok(res)
    }

    pub async fn pause(
        &self,
        pathway_id: Uuid,
        reason: String,
        ctx: &UserContext,
    ) -> Result<RTTClockEvent> {
        self.transition(
            pathway_id,
            RTTClockEvent::paused(pathway_id, reason),
            RTTStatus::Paused,
            "pause",
            "RTTClockPaused",
            ctx,
        )
        .await
    }

    pub async fn resume(&self, pathway_id: Uuid, ctx: &UserContext) -> Result<RTTClockEvent> {
        self.transition(
            pathway_id,
            RTTClockEvent::resumed(pathway_id),
            RTTStatus::Active,
            "resume",
            "RTTClockResumed",
            ctx,
        )
        .await
    }

    pub async fn stop(
        &self,
        pathway_id: Uuid,
        reason: String,
        ctx: &UserContext,
    ) -> Result<RTTClockEvent> {
        self.transition(
            pathway_id,
            RTTClockEvent::stopped(pathway_id, reason),
            RTTStatus::Stopped,
            "stop",
            "RTTClockStopped",
            ctx,
        )
        .await
    }

    async fn transition(
        &self,
        pathway_id: Uuid,
        event: RTTClockEvent,
        new_status: RTTStatus,
        action: &'static str,
        event_type: &'static str,
        ctx: &UserContext,
    ) -> Result<RTTClockEvent> {
        let ctx_clone = ctx.clone();
        let event_clone = event.clone();
        let res = self
            .db
            .transaction::<_, RTTClockEvent, Error>(|txn| {
                Box::pin(async move {
                    let e = RttRepository::create_event(txn, &event_clone).await?;
                    // Update pathway status inline via ActiveModel
                    let m = rtt_pathway::Entity::find_by_id(pathway_id)
                        .one(txn)
                        .await
                        .map_err(Error::Database)?
                        .ok_or_else(|| Error::not_found(format!("rtt_pathway {pathway_id}")))?;
                    let status_str = match new_status {
                        RTTStatus::Active => "active",
                        RTTStatus::Paused => "paused",
                        RTTStatus::Stopped => "stopped",
                    };
                    let mut am: rtt_pathway::ActiveModel = m.into();
                    am.status = Set(status_str.to_string());
                    if matches!(new_status, RTTStatus::Stopped) {
                        am.stopped_at = Set(Some(chrono::Utc::now().fixed_offset()));
                    }
                    am.updated_at = Set(chrono::Utc::now().fixed_offset());
                    am.update(txn).await.map_err(Error::Database)?;
                    AuditLogRepository::log(
                        txn,
                        "rtt_pathway",
                        pathway_id,
                        action,
                        None,
                        Some(serde_json::to_value(&e).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        event_type,
                        &serde_json::json!({ "pathway_id": pathway_id }),
                    )
                    .await?;
                    Ok(e)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                event_type,
                serde_json::json!({ "pathway_id": pathway_id }),
            ))
            .await;
        Ok(res)
    }

    pub async fn weeks_waiting(&self, pathway_id: Uuid) -> Result<u32> {
        let events = RttRepository::list_events_for_pathway(&self.db, pathway_id).await?;
        Ok(compute_active_weeks(&events, chrono::Utc::now()))
    }

    pub async fn is_breaching(&self, pathway_id: Uuid) -> Result<bool> {
        let pathway = RttRepository::find_pathway_by_id(&self.db, pathway_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("rtt_pathway {pathway_id}")))?;
        let events = RttRepository::list_events_for_pathway(&self.db, pathway_id).await?;
        Ok(pathway.is_breaching(&events, chrono::Utc::now()))
    }
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

    #[test]
    fn test_compute_weeks_empty() {
        assert_eq!(compute_active_weeks(&[], chrono::Utc::now()), 0);
    }
}
