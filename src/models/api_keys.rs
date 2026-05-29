use async_trait::async_trait;
use chrono::Utc;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter,
};
use uuid::Uuid;

pub use super::_entities::api_keys::{self, ActiveModel, Entity, Model};

#[async_trait]
impl ActiveModelBehavior for super::_entities::api_keys::ActiveModel {
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
            .filter(api_keys::Column::KeyHash.eq(hash))
            .one(db)
            .await
    }

    pub async fn find_by_id_and_tenant(
        db: &DatabaseConnection,
        id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(api_keys::Column::Id.eq(id))
            .filter(api_keys::Column::TenantId.eq(tenant_id))
            .one(db)
            .await
    }

    pub async fn find_active_by_tenant(
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> Result<Vec<Self>, DbErr> {
        Ok(Entity::find()
            .filter(api_keys::Column::TenantId.eq(tenant_id))
            .all(db)
            .await?
            .into_iter()
            .filter(Self::is_valid)
            .collect())
    }

    pub async fn count_active_by_tenant(
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> Result<u64, DbErr> {
        Ok(Self::find_active_by_tenant(db, tenant_id).await?.len() as u64)
    }

    pub async fn touch_last_used(db: &DatabaseConnection, id: Uuid) -> Result<(), DbErr> {
        if let Some(model) = Entity::find_by_id(id).one(db).await? {
            let mut active_model: ActiveModel = model.into();
            active_model.last_used_at = ActiveValue::Set(Some(Utc::now().fixed_offset()));
            active_model.update(db).await?;
        }
        Ok(())
    }

    pub fn is_valid(&self) -> bool {
        if self.revoked_at.is_some() {
            return false;
        }

        self.expires_at
            .map(|expires_at| expires_at > Utc::now().fixed_offset())
            .unwrap_or(true)
    }
}
