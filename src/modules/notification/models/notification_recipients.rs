use chrono::Utc;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde::Serialize;
use uuid::Uuid;

pub use crate::models::_entities::notification_recipients::{
    self, ActiveModel, Entity, Model,
};

/// Joined inbox item: notification_recipients JOIN notifications + users + tenants.
#[derive(Debug, FromQueryResult, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub id: Uuid,
    pub notification_id: Uuid,
    pub title: String,
    pub content: String,
    pub notification_type: String,
    pub priority: String,
    pub read_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub sender_name: String,
    pub sender_tenant_name: String,
}

impl Model {
    /// Count unread notifications for a user.
    pub async fn unread_count(
        db: &DatabaseConnection,
        user_id: Uuid,
    ) -> ModelResult<i64> {
        use crate::models::_entities::notifications as n;

        Ok(Entity::find()
            .filter(notification_recipients::Column::UserId.eq(user_id))
            .filter(notification_recipients::Column::ReadAt.is_null())
            .inner_join(n::Entity)
            .filter(n::Column::Status.eq("active"))
            .count(db)
            .await
            .map(|c| c as i64)?)
    }

    /// Check if a user has any unread forced (high-priority) notifications.
    pub async fn has_forced_unread(
        db: &DatabaseConnection,
        user_id: Uuid,
    ) -> ModelResult<bool> {
        use crate::models::_entities::notifications as n;

        Ok(Entity::find()
            .filter(notification_recipients::Column::UserId.eq(user_id))
            .filter(notification_recipients::Column::ReadAt.is_null())
            .filter(n::Column::Priority.eq("high"))
            .filter(n::Column::Status.eq("active"))
            .inner_join(n::Entity)
            .one(db)
            .await
            .map(|opt| opt.is_some())?)
    }

    /// Inbox list: JOIN notifications for a user, paginated.
    /// Sender info is fetched separately to avoid complex multi-JOIN.
    pub async fn inbox_list(
        db: &DatabaseConnection,
        user_id: Uuid,
        read_filter: Option<bool>,
        page: u64,
        page_size: u64,
    ) -> ModelResult<Vec<Self>> {
        let mut query = Entity::find();

        query = query.filter(notification_recipients::Column::UserId.eq(user_id));

        if let Some(read) = read_filter {
            if read {
                query =
                    query.filter(notification_recipients::Column::ReadAt.is_not_null());
            } else {
                query = query.filter(notification_recipients::Column::ReadAt.is_null());
            }
        }

        query = query.order_by_desc(notification_recipients::Column::CreatedAt);

        let items: Vec<Self> = query
            .offset((page.saturating_sub(1)) * page_size)
            .limit(page_size)
            .all(db)
            .await?;

        Ok(items)
    }

    /// Mark a single notification as read.
    pub async fn mark_read(
        db: &DatabaseConnection,
        recipient_id: Uuid,
        user_id: Uuid,
    ) -> ModelResult<()> {
        let recipient = Entity::find()
            .filter(notification_recipients::Column::Id.eq(recipient_id))
            .filter(notification_recipients::Column::UserId.eq(user_id))
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)?;

        let mut active: ActiveModel = recipient.into();
        active.read_at = ActiveValue::Set(Some(Utc::now().fixed_offset()));
        active.update(db).await?;

        Ok(())
    }

    /// Mark all notifications as read for a user.
    pub async fn mark_all_read(
        db: &DatabaseConnection,
        user_id: Uuid,
    ) -> ModelResult<u64> {
        let unread = Entity::find()
            .filter(notification_recipients::Column::UserId.eq(user_id))
            .filter(notification_recipients::Column::ReadAt.is_null())
            .all(db)
            .await?;

        let mut count = 0u64;
        for recipient in unread {
            let mut active: ActiveModel = recipient.into();
            active.read_at = ActiveValue::Set(Some(Utc::now().fixed_offset()));
            active.update(db).await?;
            count += 1;
        }

        Ok(count)
    }

    /// Get forced (high-priority, unread) notification recipient IDs for a user.
    pub async fn forced_recipient_ids(
        db: &DatabaseConnection,
        user_id: Uuid,
    ) -> ModelResult<Vec<Self>> {
        use crate::models::_entities::notifications as n;

        Ok(Entity::find()
            .filter(notification_recipients::Column::UserId.eq(user_id))
            .filter(notification_recipients::Column::ReadAt.is_null())
            .inner_join(n::Entity)
            .filter(n::Column::Priority.eq("high"))
            .filter(n::Column::Status.eq("active"))
            .order_by_asc(notification_recipients::Column::CreatedAt)
            .all(db)
            .await?)
    }
}
