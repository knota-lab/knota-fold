use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter, TransactionTrait,
};
use serde_json;
use uuid::Uuid;

use super::super::errors::NotificationError;
use crate::models::_entities::{
    notification_recipients, notifications, roles, user_roles, users,
};
use crate::utils::error::IntoAppError;

/// Parameters for [`create_notification`].
#[derive(Debug)]
pub struct CreateNotificationParams<'a> {
    pub tenant_id: Option<Uuid>,
    pub created_by: Uuid,
    pub title: &'a str,
    pub content: &'a str,
    pub notification_type: &'a str,
    pub priority: &'a str,
    pub target_role_codes: Option<&'a [String]>,
}

/// Unified entry point for creating notifications (called by controller).
#[tracing::instrument(skip(db))]
pub async fn create_notification(
    db: &DatabaseConnection,
    params: &CreateNotificationParams<'_>,
) -> loco_rs::Result<notifications::Model> {
    let CreateNotificationParams {
        tenant_id,
        created_by,
        title,
        content,
        notification_type,
        priority,
        target_role_codes,
    } = *params;
    let txn = db.begin().await.db_err()?;

    // 1. Insert notification record
    let notification = notifications::ActiveModel {
        tenant_id: ActiveValue::Set(tenant_id),
        title: ActiveValue::Set(title.to_string()),
        content: ActiveValue::Set(content.to_string()),
        notification_type: ActiveValue::Set(notification_type.to_string()),
        priority: ActiveValue::Set(priority.to_string()),
        created_by: ActiveValue::Set(created_by),
        target_role_codes: ActiveValue::Set(
            target_role_codes
                .map(|codes| serde_json::to_string(codes).unwrap_or_default()),
        ),
        status: ActiveValue::Set("active".to_string()),
        ..Default::default()
    }
    .insert(&txn)
    .await
    .db_err()?;

    // 2. Resolve recipients based on type
    let recipient_user_ids = match notification_type {
        "platform" => resolve_platform_recipients(&txn).await?,
        "tenant_all" => {
            let tid = tenant_id.ok_or_else(|| NotificationError::Forbidden.to_err())?;
            resolve_tenant_all_recipients(&txn, tid).await?
        }
        "tenant_role" => {
            let tid = tenant_id.ok_or_else(|| NotificationError::Forbidden.to_err())?;
            let codes = target_role_codes
                .ok_or_else(|| NotificationError::NoRolesSelected.to_err())?;
            resolve_role_recipients(&txn, tid, codes).await?
        }
        _ => {
            return Err(NotificationError::UnsupportedType.to_err());
        }
    };

    // 3. Batch insert recipient records
    if !recipient_user_ids.is_empty() {
        let now = Utc::now().fixed_offset();
        let recipient_rows: Vec<notification_recipients::ActiveModel> =
            recipient_user_ids
                .into_iter()
                .map(|uid| notification_recipients::ActiveModel {
                    id: ActiveValue::Set(Uuid::now_v7()),
                    notification_id: ActiveValue::Set(notification.id),
                    user_id: ActiveValue::Set(uid),
                    read_at: ActiveValue::Set(None),
                    created_at: ActiveValue::Set(now),
                })
                .collect();

        // Insert in batches of 500 to avoid SQLite limits
        for chunk in recipient_rows.chunks(500) {
            notification_recipients::Entity::insert_many(chunk.to_vec())
                .exec(&txn)
                .await
                .db_err()?;
        }
    }

    txn.commit().await.db_err()?;

    Ok(notification)
}

/// Public API for backend business logic to send user notifications.
#[tracing::instrument(skip(db, user_ids))]
pub async fn notify_users(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    created_by: Uuid,
    title: &str,
    content: &str,
    user_ids: &[Uuid],
) -> loco_rs::Result<notifications::Model> {
    create_user_notification(
        db,
        Some(tenant_id),
        created_by,
        title,
        content,
        "normal",
        user_ids,
    )
    .await
}

/// Urgent variant — sends high-priority user notifications.
#[tracing::instrument(skip(db, user_ids))]
pub async fn notify_users_urgent(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    created_by: Uuid,
    title: &str,
    content: &str,
    user_ids: &[Uuid],
) -> loco_rs::Result<notifications::Model> {
    create_user_notification(
        db,
        Some(tenant_id),
        created_by,
        title,
        content,
        "high",
        user_ids,
    )
    .await
}

/// Internal: create a user-targeted notification.
async fn create_user_notification(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    created_by: Uuid,
    title: &str,
    content: &str,
    priority: &str,
    user_ids: &[Uuid],
) -> loco_rs::Result<notifications::Model> {
    let txn = db.begin().await.db_err()?;

    let notification = notifications::ActiveModel {
        tenant_id: ActiveValue::Set(tenant_id),
        title: ActiveValue::Set(title.to_string()),
        content: ActiveValue::Set(content.to_string()),
        notification_type: ActiveValue::Set("user".to_string()),
        priority: ActiveValue::Set(priority.to_string()),
        created_by: ActiveValue::Set(created_by),
        target_role_codes: ActiveValue::Set(None),
        status: ActiveValue::Set("active".to_string()),
        ..Default::default()
    }
    .insert(&txn)
    .await
    .db_err()?;

    if !user_ids.is_empty() {
        let now = Utc::now().fixed_offset();
        let rows: Vec<notification_recipients::ActiveModel> = user_ids
            .iter()
            .map(|uid| notification_recipients::ActiveModel {
                id: ActiveValue::Set(Uuid::now_v7()),
                notification_id: ActiveValue::Set(notification.id),
                user_id: ActiveValue::Set(*uid),
                read_at: ActiveValue::Set(None),
                created_at: ActiveValue::Set(now),
            })
            .collect();

        for chunk in rows.chunks(500) {
            notification_recipients::Entity::insert_many(chunk.to_vec())
                .exec(&txn)
                .await
                .db_err()?;
        }
    }

    txn.commit().await.db_err()?;
    Ok(notification)
}

/// Platform notification: find all active tenant admins (cross-tenant).
async fn resolve_platform_recipients(
    db: &impl ConnectionTrait,
) -> loco_rs::Result<Vec<Uuid>> {
    let admin_role_ids: Vec<Uuid> = roles::Entity::find()
        .filter(roles::Column::Code.eq("TENANT_ADMIN"))
        .all(db)
        .await
        .db_err()?
        .into_iter()
        .map(|r| r.id)
        .collect();

    if admin_role_ids.is_empty() {
        return Ok(vec![]);
    }

    let admin_user_ids: Vec<Uuid> = user_roles::Entity::find()
        .filter(user_roles::Column::RoleId.is_in(admin_role_ids))
        .all(db)
        .await
        .db_err()?
        .into_iter()
        .map(|ur| ur.user_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    // Filter by active users
    let active_admins: Vec<Uuid> = users::Entity::find()
        .filter(users::Column::Id.is_in(admin_user_ids))
        .filter(users::Column::Status.eq("active"))
        .all(db)
        .await
        .db_err()?
        .into_iter()
        .map(|u| u.id)
        .collect();

    Ok(active_admins)
}

/// Tenant all: find all active users in a tenant.
async fn resolve_tenant_all_recipients(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
) -> loco_rs::Result<Vec<Uuid>> {
    let users_list = users::Entity::find()
        .filter(users::Column::TenantId.eq(tenant_id))
        .filter(users::Column::Status.eq("active"))
        .all(db)
        .await
        .db_err()?;

    Ok(users_list.into_iter().map(|u| u.id).collect())
}

/// Role-based: resolve role codes to user IDs within a tenant.
async fn resolve_role_recipients(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
    role_codes: &[String],
) -> loco_rs::Result<Vec<Uuid>> {
    let matched_roles: Vec<Uuid> = roles::Entity::find()
        .filter(roles::Column::TenantId.eq(tenant_id))
        .filter(
            roles::Column::Code
                .is_in(role_codes.iter().map(String::as_str).collect::<Vec<&str>>()),
        )
        .all(db)
        .await
        .db_err()?
        .into_iter()
        .map(|r| r.id)
        .collect();

    if matched_roles.is_empty() {
        return Ok(vec![]);
    }

    let role_user_ids: Vec<Uuid> = user_roles::Entity::find()
        .filter(user_roles::Column::TenantId.eq(tenant_id))
        .filter(user_roles::Column::RoleId.is_in(matched_roles))
        .all(db)
        .await
        .db_err()?
        .into_iter()
        .map(|ur| ur.user_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let active_users: Vec<Uuid> = users::Entity::find()
        .filter(users::Column::Id.is_in(role_user_ids))
        .filter(users::Column::Status.eq("active"))
        .all(db)
        .await
        .db_err()?
        .into_iter()
        .map(|u| u.id)
        .collect();

    Ok(active_users)
}
