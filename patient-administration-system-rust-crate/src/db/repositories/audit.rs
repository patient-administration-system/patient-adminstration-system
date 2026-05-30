//! audit repository — writes rows to `audit_log` within the same DB
//! transaction as the entity change.

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::audit_log;
use crate::{Error, Result};

/// Per-request context attached to audit rows. Populated from request
/// headers (`X-User-Id`, `X-User-Ip`, `X-User-Agent`) by the HTTP layer.
///
/// All fields are optional; the audit trail still records the action even
/// when the caller is anonymous.
#[derive(Debug, Clone, Default)]
pub struct UserContext {
    pub user_id: Option<String>,
    pub user_ip: Option<String>,
    pub user_agent: Option<String>,
}

impl UserContext {
    /// Build a UserContext with no caller details. Useful for system
    /// background tasks (e.g. outbox dispatcher).
    pub fn system() -> Self {
        Self {
            user_id: Some("system".to_string()),
            user_ip: None,
            user_agent: None,
        }
    }
}

pub struct AuditLogRepository;

impl AuditLogRepository {
    /// Insert one audit row.
    ///
    /// Pass `old_value = None` for creates, `new_value = None` for deletes,
    /// and both for updates.
    pub async fn log<C: ConnectionTrait>(
        conn: &C,
        entity_type: &str,
        entity_id: Uuid,
        action: &str,
        old_value: Option<serde_json::Value>,
        new_value: Option<serde_json::Value>,
        ctx: &UserContext,
    ) -> Result<()> {
        let am = audit_log::ActiveModel {
            id: Set(Uuid::new_v4()),
            entity_type: Set(entity_type.to_string()),
            entity_id: Set(entity_id),
            action: Set(action.to_string()),
            old_value: Set(old_value),
            new_value: Set(new_value),
            user_id: Set(ctx.user_id.clone()),
            user_ip: Set(ctx.user_ip.clone()),
            user_agent: Set(ctx.user_agent.clone()),
            at: Set(chrono::Utc::now().fixed_offset()),
        };
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(())
    }

    /// Recent audit rows across all entities, newest first.
    pub async fn list_recent<C: ConnectionTrait>(
        conn: &C,
        limit: u64,
    ) -> Result<Vec<audit_log::Model>> {
        audit_log::Entity::find()
            .order_by_desc(audit_log::Column::At)
            .limit(limit)
            .all(conn)
            .await
            .map_err(Error::Database)
    }

    /// Audit rows for a single entity, newest first.
    pub async fn list_for_entity<C: ConnectionTrait>(
        conn: &C,
        entity_type: &str,
        entity_id: Uuid,
        limit: u64,
    ) -> Result<Vec<audit_log::Model>> {
        audit_log::Entity::find()
            .filter(audit_log::Column::EntityType.eq(entity_type))
            .filter(audit_log::Column::EntityId.eq(entity_id))
            .order_by_desc(audit_log::Column::At)
            .limit(limit)
            .all(conn)
            .await
            .map_err(Error::Database)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_context_default_is_empty() {
        let c = UserContext::default();
        assert!(c.user_id.is_none());
        assert!(c.user_ip.is_none());
        assert!(c.user_agent.is_none());
    }

    #[test]
    fn test_user_context_system_marks_system_user() {
        let c = UserContext::system();
        assert_eq!(c.user_id.as_deref(), Some("system"));
        assert!(c.user_ip.is_none());
        assert!(c.user_agent.is_none());
    }
}
