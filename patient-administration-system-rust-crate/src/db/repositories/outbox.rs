//! outbox repository — writes domain events into `outbox_events` for the
//! background dispatcher to publish, and manages the v0.5 dead-letter
//! queue (`outbox_dead_letters`) for events that exceed the retry budget.

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::{outbox_dead_letter, outbox_event};
use crate::{Error, Result};

pub struct OutboxRepository;

impl OutboxRepository {
    /// Insert a new outbox row with `published = false`, `retry_count = 0`.
    ///
    /// Call inside the same transaction as the entity write to guarantee
    /// "exactly the rows we committed produce events".
    pub async fn publish<C: ConnectionTrait>(
        conn: &C,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        let am = outbox_event::ActiveModel {
            id: Set(Uuid::new_v4()),
            event_type: Set(event_type.to_string()),
            payload: Set(payload.clone()),
            published: Set(false),
            at: Set(chrono::Utc::now().fixed_offset()),
            retry_count: Set(0),
            last_attempted_at: Set(None),
            last_error: Set(None),
        };
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(())
    }

    /// Fetch the next batch of unpublished outbox rows, oldest first.
    pub async fn fetch_unpublished<C: ConnectionTrait>(
        conn: &C,
        limit: u64,
    ) -> Result<Vec<outbox_event::Model>> {
        outbox_event::Entity::find()
            .filter(outbox_event::Column::Published.eq(false))
            .order_by_asc(outbox_event::Column::At)
            .limit(limit)
            .all(conn)
            .await
            .map_err(Error::Database)
    }

    /// Mark a single outbox row as published. Idempotent.
    pub async fn mark_published<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<()> {
        let m = outbox_event::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        if let Some(m) = m {
            let mut am: outbox_event::ActiveModel = m.into();
            am.published = Set(true);
            am.last_attempted_at = Set(Some(chrono::Utc::now().fixed_offset()));
            am.last_error = Set(None);
            am.update(conn).await.map_err(Error::Database)?;
        }
        Ok(())
    }

    /// Record a failed publish attempt: increment `retry_count`, store the
    /// error string in `last_error`, stamp `last_attempted_at`. Returns the
    /// new retry count (so the dispatcher can decide whether to dead-letter).
    ///
    /// Returns `Ok(0)` and a no-op if the row is gone (idempotent against
    /// concurrent dead-lettering / replay).
    pub async fn record_failure<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        error: &str,
    ) -> Result<i32> {
        let m = match outbox_event::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
        {
            Some(m) => m,
            None => return Ok(0),
        };
        let new_count = m.retry_count.saturating_add(1);
        let mut am: outbox_event::ActiveModel = m.into();
        am.retry_count = Set(new_count);
        am.last_attempted_at = Set(Some(chrono::Utc::now().fixed_offset()));
        am.last_error = Set(Some(error.to_string()));
        am.update(conn).await.map_err(Error::Database)?;
        Ok(new_count)
    }

    /// Move an outbox row to the dead-letter queue: insert a corresponding
    /// `outbox_dead_letters` row carrying the same payload + the recorded
    /// failure metadata, then delete the original `outbox_events` row.
    /// Idempotent — returns `Ok(())` if the row is already gone.
    ///
    /// The caller should pass a `DatabaseTransaction` so the insert + delete
    /// commit atomically.
    pub async fn move_to_dead_letter<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        error: &str,
    ) -> Result<()> {
        let m = match outbox_event::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
        {
            Some(m) => m,
            None => return Ok(()),
        };
        let now = chrono::Utc::now().fixed_offset();
        let dl = outbox_dead_letter::ActiveModel {
            id: Set(Uuid::new_v4()),
            original_id: Set(m.id),
            event_type: Set(m.event_type.clone()),
            payload: Set(m.payload.clone()),
            created_at: Set(m.at),
            dead_lettered_at: Set(now),
            retry_count: Set(m.retry_count),
            last_error: Set(error.to_string()),
        };
        dl.insert(conn).await.map_err(Error::Database)?;
        outbox_event::Entity::delete_by_id(id)
            .exec(conn)
            .await
            .map_err(Error::Database)?;
        Ok(())
    }
}

/// Read + replay surface for the v0.5 dead-letter queue.
pub struct DeadLetterRepository;

impl DeadLetterRepository {
    /// Newest-first list of dead-letter rows, capped at `limit`.
    pub async fn list<C: ConnectionTrait>(
        conn: &C,
        limit: u64,
    ) -> Result<Vec<outbox_dead_letter::Model>> {
        outbox_dead_letter::Entity::find()
            .order_by_desc(outbox_dead_letter::Column::DeadLetteredAt)
            .limit(limit)
            .all(conn)
            .await
            .map_err(Error::Database)
    }

    /// Find one DLQ row by id (the DLQ id, not the `original_id`).
    pub async fn find_by_id<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<outbox_dead_letter::Model>> {
        outbox_dead_letter::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)
    }

    /// Count of DLQ rows. Cheap admin / dashboard read.
    pub async fn count<C: ConnectionTrait>(conn: &C) -> Result<u64> {
        outbox_dead_letter::Entity::find()
            .count(conn)
            .await
            .map_err(Error::Database)
    }

    /// Replay a DLQ row back into `outbox_events`: insert a fresh outbox
    /// row carrying the same payload (and the original `at` so dispatcher
    /// ordering reflects the original event order, not the replay time),
    /// then delete the DLQ row. The new row has `retry_count = 0` and
    /// `last_error = NULL` so the dispatcher gets a clean slate.
    ///
    /// Returns the new outbox row id. `Error::NotFound` if the DLQ id is
    /// already gone. The caller should pass a `DatabaseTransaction`.
    pub async fn replay<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Uuid> {
        let dl = outbox_dead_letter::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("outbox_dead_letter {id}")))?;
        let new_id = Uuid::new_v4();
        let am = outbox_event::ActiveModel {
            id: Set(new_id),
            event_type: Set(dl.event_type.clone()),
            payload: Set(dl.payload.clone()),
            published: Set(false),
            at: Set(dl.created_at),
            retry_count: Set(0),
            last_attempted_at: Set(None),
            last_error: Set(None),
        };
        am.insert(conn).await.map_err(Error::Database)?;
        outbox_dead_letter::Entity::delete_by_id(id)
            .exec(conn)
            .await
            .map_err(Error::Database)?;
        Ok(new_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outbox_repository_is_zero_sized() {
        // The repo is intentionally a unit struct — no per-instance state.
        assert_eq!(std::mem::size_of::<OutboxRepository>(), 0);
        assert_eq!(std::mem::size_of::<DeadLetterRepository>(), 0);
    }

    #[test]
    fn test_payload_value_constructs_cleanly() {
        // Confirms the payload type used by `publish` is well-formed JSON.
        let v = serde_json::json!({
            "patient_id": Uuid::new_v4(),
            "name": "alice",
        });
        assert!(v.is_object());
    }
}
