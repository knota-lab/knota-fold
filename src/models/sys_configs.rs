use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter,
};
use uuid::Uuid;

pub use super::_entities::sys_configs::{self, ActiveModel, Entity, Model};

#[async_trait]
impl ActiveModelBehavior for super::_entities::sys_configs::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        let mut this = self;
        if insert {
            this.id = ActiveValue::Set(crate::utils::id::generate_id());
            let now = chrono::Utc::now().fixed_offset();
            this.created_at = ActiveValue::Set(now);
            this.updated_at = ActiveValue::Set(now);
        } else {
            this.updated_at = ActiveValue::Set(chrono::Utc::now().fixed_offset());
        }
        Ok(this)
    }
}

impl Model {
    /// Find global config by key (`tenant_id` IS NULL).
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_global_by_key(
        db: &DatabaseConnection,
        key: &str,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(sys_configs::Column::Key.eq(key))
            .filter(sys_configs::Column::TenantId.is_null())
            .one(db)
            .await
    }

    /// Find tenant override by key for a specific tenant.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_tenant_by_key(
        db: &DatabaseConnection,
        key: &str,
        tenant_id: Uuid,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(sys_configs::Column::Key.eq(key))
            .filter(sys_configs::Column::TenantId.eq(tenant_id))
            .one(db)
            .await
    }

    /// List all global configs (`tenant_id` IS NULL).
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn list_global(db: &DatabaseConnection) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(sys_configs::Column::TenantId.is_null())
            .all(db)
            .await
    }

    /// List all tenant override configs for a specific tenant.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn list_tenant_overrides(
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(sys_configs::Column::TenantId.eq(tenant_id))
            .all(db)
            .await
    }

    /// Delete all tenant overrides for a given key (used when global config is deleted).
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn delete_tenant_overrides_for_key(
        db: &DatabaseConnection,
        key: &str,
    ) -> Result<u64, DbErr> {
        let res = Entity::delete_many()
            .filter(sys_configs::Column::Key.eq(key))
            .filter(sys_configs::Column::TenantId.is_not_null())
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }
}
