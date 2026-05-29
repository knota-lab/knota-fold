use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter,
};
use uuid::Uuid;

pub use super::_entities::i18n_bundle_revisions::{self, ActiveModel, Entity, Model};

pub const SCOPE_GLOBAL: &str = "global";
pub const SCOPE_TENANT: &str = "tenant";

#[async_trait]
impl ActiveModelBehavior for super::_entities::i18n_bundle_revisions::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        let mut this = self;
        if insert && matches!(this.id, ActiveValue::NotSet) {
            this.id = ActiveValue::Set(crate::utils::id::generate_id());
        }
        this.updated_at = ActiveValue::Set(chrono::Utc::now().fixed_offset());
        Ok(this)
    }
}

impl Model {
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_global(
        db: &DatabaseConnection,
        locale: &str,
        namespace: &str,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(i18n_bundle_revisions::Column::Locale.eq(locale))
            .filter(i18n_bundle_revisions::Column::Namespace.eq(namespace))
            .filter(i18n_bundle_revisions::Column::Scope.eq(SCOPE_GLOBAL))
            .filter(i18n_bundle_revisions::Column::TenantId.is_null())
            .one(db)
            .await
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_tenant(
        db: &DatabaseConnection,
        locale: &str,
        namespace: &str,
        tenant_id: Uuid,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(i18n_bundle_revisions::Column::Locale.eq(locale))
            .filter(i18n_bundle_revisions::Column::Namespace.eq(namespace))
            .filter(i18n_bundle_revisions::Column::Scope.eq(SCOPE_TENANT))
            .filter(i18n_bundle_revisions::Column::TenantId.eq(tenant_id))
            .one(db)
            .await
    }
}
