use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
};
use uuid::Uuid;

use super::super::models::notification_recipients as recipient_model;
use super::super::models::notifications as notif_model;
use super::super::views;
use crate::models::_entities::{notification_recipients, tenants, users};
use crate::utils::error::IntoModelResult;

/// Inbox list (paginated). Returns recipient records with notification details.
#[tracing::instrument(skip(db))]
pub async fn get_inbox(
    db: &DatabaseConnection,
    user_id: Uuid,
    read_filter: Option<bool>,
    page: u64,
    page_size: u64,
) -> loco_rs::Result<crate::views::pagination::PaginatedResponse<views::InboxItemResponse>>
{
    let page = page.max(1);
    let page_size = page_size.clamp(1, 100);

    // Fetch recipient records
    let recipients =
        recipient_model::Model::inbox_list(db, user_id, read_filter, page, page_size)
            .await
            .model_err()?;

    // Get total count for pagination
    let total_items = notification_recipients::Entity::find()
        .filter(notification_recipients::Column::UserId.eq(user_id))
        .count(db)
        .await
        .model_err()?;

    let total_pages = if total_items == 0 {
        0
    } else {
        total_items.div_ceil(page_size) as u64
    };

    // Resolve notification + sender info for each recipient
    let mut items = Vec::with_capacity(recipients.len());
    for r in &recipients {
        let notif = notif_model::Model::find_by_id(db, r.notification_id).await;
        let Ok(notif) = notif else { continue };

        let sender: Option<users::Model> = users::Entity::find()
            .filter(users::Column::Id.eq(notif.created_by))
            .one(db)
            .await
            .model_err()?;

        let (sender_name, sender_tenant_name) = if let Some(sender) = sender {
            let tenant_name = tenants::Entity::find()
                .filter(tenants::Column::Id.eq(sender.tenant_id))
                .one(db)
                .await
                .model_err()?
                .map(|t| t.name)
                .unwrap_or_default();
            (sender.name, tenant_name)
        } else {
            ("未知用户".to_string(), String::new())
        };

        items.push(views::InboxItemResponse {
            id: r.id.to_string(),
            notification_id: r.notification_id.to_string(),
            title: notif.title,
            content: notif.content,
            notification_type: notif.notification_type,
            priority: notif.priority,
            read_at: r.read_at.map(|dt| dt.to_rfc3339()),
            created_at: notif.created_at.to_rfc3339(),
            sender_name,
            sender_tenant_name,
        });
    }

    Ok(crate::views::pagination::PaginatedResponse {
        items,
        total_pages,
        total_items: total_items as u64,
        page,
        page_size,
    })
}

/// Unread count + whether forced notifications exist.
#[tracing::instrument(skip(db))]
pub async fn get_unread_count(
    db: &DatabaseConnection,
    user_id: Uuid,
) -> loco_rs::Result<views::UnreadCountResponse> {
    let count = recipient_model::Model::unread_count(db, user_id)
        .await
        .model_err()?;

    let has_forced = recipient_model::Model::has_forced_unread(db, user_id)
        .await
        .model_err()?;

    Ok(views::UnreadCountResponse { count, has_forced })
}

/// Forced notification list (for modal popup).
#[tracing::instrument(skip(db))]
pub async fn get_forced_notifications(
    db: &DatabaseConnection,
    user_id: Uuid,
) -> loco_rs::Result<Vec<views::InboxItemResponse>> {
    let recipients = recipient_model::Model::forced_recipient_ids(db, user_id)
        .await
        .model_err()?;

    let mut items = Vec::with_capacity(recipients.len());
    for r in &recipients {
        let Ok(notif) = notif_model::Model::find_by_id(db, r.notification_id).await
        else {
            continue;
        };

        let sender: Option<users::Model> = users::Entity::find()
            .filter(users::Column::Id.eq(notif.created_by))
            .one(db)
            .await
            .model_err()?;

        let (sender_name, sender_tenant_name) = if let Some(sender) = sender {
            let tenant_name = tenants::Entity::find()
                .filter(tenants::Column::Id.eq(sender.tenant_id))
                .one(db)
                .await
                .model_err()?
                .map(|t| t.name)
                .unwrap_or_default();
            (sender.name, tenant_name)
        } else {
            ("未知用户".to_string(), String::new())
        };

        items.push(views::InboxItemResponse {
            id: r.id.to_string(),
            notification_id: r.notification_id.to_string(),
            title: notif.title,
            content: notif.content,
            notification_type: notif.notification_type,
            priority: notif.priority,
            read_at: r.read_at.map(|dt| dt.to_rfc3339()),
            created_at: notif.created_at.to_rfc3339(),
            sender_name,
            sender_tenant_name,
        });
    }

    Ok(items)
}

/// Mark a single notification as read.
#[tracing::instrument(skip(db))]
pub async fn mark_read(
    db: &DatabaseConnection,
    recipient_id: Uuid,
    user_id: Uuid,
) -> loco_rs::Result<()> {
    recipient_model::Model::mark_read(db, recipient_id, user_id)
        .await
        .model_err()
}

/// Mark all notifications as read for a user.
#[tracing::instrument(skip(db))]
pub async fn mark_all_read(
    db: &DatabaseConnection,
    user_id: Uuid,
) -> loco_rs::Result<u64> {
    recipient_model::Model::mark_all_read(db, user_id)
        .await
        .model_err()
}
