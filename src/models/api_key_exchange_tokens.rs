use async_trait::async_trait;
use chrono::Utc;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter,
};
use uuid::Uuid;

pub use super::_entities::api_key_exchange_tokens::{self, ActiveModel, Entity, Model};

#[async_trait]
impl ActiveModelBehavior for super::_entities::api_key_exchange_tokens::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if insert {
            let mut this = self;
            let now = Utc::now().fixed_offset();
            if this.id.is_not_set() {
                this.id = ActiveValue::Set(crate::utils::id::generate_id());
            }
            this.created_at = ActiveValue::Set(now);
            this.updated_at = ActiveValue::Set(now);
            Ok(this)
        } else {
            let mut this = self;
            this.updated_at = ActiveValue::Set(Utc::now().fixed_offset());
            Ok(this)
        }
    }
}

impl Model {
    pub async fn find_by_hash(
        db: &DatabaseConnection,
        hash: &str,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(api_key_exchange_tokens::Column::TokenHash.eq(hash))
            .one(db)
            .await
    }

    pub async fn find_by_id_and_tenant(
        db: &DatabaseConnection,
        id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(api_key_exchange_tokens::Column::Id.eq(id))
            .filter(api_key_exchange_tokens::Column::TenantId.eq(tenant_id))
            .one(db)
            .await
    }

    pub async fn find_by_tenant(
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(api_key_exchange_tokens::Column::TenantId.eq(tenant_id))
            .all(db)
            .await
    }

    pub async fn count_valid_by_tenant(
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> Result<u64, DbErr> {
        let items = Self::find_by_tenant(db, tenant_id).await?;
        Ok(items.into_iter().filter(Self::is_valid).count() as u64)
    }

    pub async fn increment_usage(
        db: &DatabaseConnection,
        id: Uuid,
    ) -> Result<i32, DbErr> {
        let model = Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| DbErr::RecordNotFound(id.to_string()))?;
        let next = model.used_count + 1;
        let mut active_model: ActiveModel = model.into();
        active_model.used_count = ActiveValue::Set(next);
        active_model.updated_at = ActiveValue::Set(Utc::now().fixed_offset());
        active_model.update(db).await?;
        Ok(next)
    }

    pub fn is_valid(&self) -> bool {
        self.used_count < self.max_usage && self.expires_at > Utc::now().fixed_offset()
    }
}
