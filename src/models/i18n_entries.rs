use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter,
};

pub use super::_entities::i18n_entries::{self, ActiveModel, Entity, Model};

pub const STATUS_ACTIVE: &str = "active";
pub const STATUS_STALE: &str = "stale";

#[async_trait]
impl ActiveModelBehavior for super::_entities::i18n_entries::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        let mut this = self;
        let now = chrono::Utc::now().fixed_offset();
        if insert {
            if matches!(this.id, ActiveValue::NotSet) {
                this.id = ActiveValue::Set(crate::utils::id::generate_id());
            }
            this.created_at = ActiveValue::Set(now);
            if matches!(this.last_seen_at, ActiveValue::NotSet) {
                this.last_seen_at = ActiveValue::Set(now);
            }
        }
        this.updated_at = ActiveValue::Set(now);
        Ok(this)
    }
}

impl Model {
    /// Computed stable id used in API/manifest payloads: `{namespace}.{key}`.
    #[must_use]
    pub fn stable_id(&self) -> String {
        format!("{}.{}", self.namespace, self.key)
    }

    pub async fn find_by_namespace_key<C>(
        db: &C,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Self>, DbErr>
    where
        C: ConnectionTrait,
    {
        Entity::find()
            .filter(i18n_entries::Column::Namespace.eq(namespace))
            .filter(i18n_entries::Column::Key.eq(key))
            .one(db)
            .await
    }

    pub async fn list_active_in_namespace(
        db: &DatabaseConnection,
        namespace: &str,
    ) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(i18n_entries::Column::Namespace.eq(namespace))
            .filter(i18n_entries::Column::Status.eq(STATUS_ACTIVE))
            .all(db)
            .await
    }
}
