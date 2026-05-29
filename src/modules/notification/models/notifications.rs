use chrono::Utc;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use uuid::Uuid;

pub use crate::models::_entities::notifications::{self, ActiveModel, Entity, Model};

#[async_trait::async_trait]
impl ActiveModelBehavior for crate::models::_entities::notifications::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        let mut this = self;
        if insert {
            if this.id.is_not_set() {
                this.id = ActiveValue::Set(Uuid::now_v7());
            }
            if this.created_at.is_not_set() {
                this.created_at = ActiveValue::Set(Utc::now().fixed_offset());
            }
            if this.updated_at.is_not_set() {
                this.updated_at = ActiveValue::Set(Utc::now().fixed_offset());
            }
        } else {
            this.updated_at = ActiveValue::Set(Utc::now().fixed_offset());
        }
        Ok(this)
    }
}

impl Model {
    /// Find a notification by ID, only if status is 'active'.
    pub async fn find_by_id(db: &DatabaseConnection, id: Uuid) -> ModelResult<Self> {
        Entity::find()
            .filter(notifications::Column::Id.eq(id))
            .filter(notifications::Column::Status.eq("active"))
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Managed list: query by tenant or globally (super admin), ordered by newest first.
    pub async fn list_managed(
        db: &DatabaseConnection,
        tenant_filter: Option<Uuid>,
        notification_type: Option<String>,
        page: u64,
        page_size: u64,
    ) -> ModelResult<(Vec<Self>, u64)> {
        let mut query = Entity::find();

        if let Some(tid) = tenant_filter {
            query = query.filter(notifications::Column::TenantId.eq(tid));
        }
        if let Some(nt) = notification_type {
            query = query.filter(notifications::Column::NotificationType.eq(nt));
        }

        query = query.order_by_desc(notifications::Column::CreatedAt);

        let total = query.clone().count(db).await?;
        let items = query
            .offset((page.saturating_sub(1)) * page_size)
            .limit(page_size)
            .all(db)
            .await?;

        Ok((items, total))
    }
}
