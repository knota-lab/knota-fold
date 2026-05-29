use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter,
};
use uuid::Uuid;

use super::super::errors::NotificationError;
use crate::models::_entities::notifications;
use crate::utils::error::IntoAppError;

/// Revoke a notification (set status = 'revoked').
#[tracing::instrument(skip(db))]
pub async fn revoke_notification(
    db: &DatabaseConnection,
    notification_id: Uuid,
    operator_id: Uuid,
    is_super_admin: bool,
    tenant_filter: Option<Uuid>,
) -> loco_rs::Result<()> {
    // 1. Find the notification
    let notif = notifications::Entity::find()
        .filter(notifications::Column::Id.eq(notification_id))
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| NotificationError::NotFound.to_err())?;

    // 2. Check already revoked
    if notif.status == "revoked" {
        return Err(NotificationError::AlreadyRevoked.to_err());
    }

    // 3. Permission check: non-super-admin can only revoke own tenant's notifications
    if !is_super_admin {
        let tid = tenant_filter.ok_or_else(|| NotificationError::Forbidden.to_err())?;
        if notif.tenant_id != Some(tid) {
            return Err(NotificationError::Forbidden.to_err());
        }
    }

    // 4. Update status to revoked
    let mut active = notif.into_active_model();
    active.status = ActiveValue::Set("revoked".to_string());
    active.updated_at = ActiveValue::Set(Utc::now().fixed_offset());
    active.update(db).await.db_err()?;

    Ok(())
}
