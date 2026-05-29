use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter, QueryOrder,
};

pub use super::_entities::i18n_supported_locales::{self, ActiveModel, Entity, Model};

#[async_trait]
impl ActiveModelBehavior for super::_entities::i18n_supported_locales::ActiveModel {
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
        }
        this.updated_at = ActiveValue::Set(now);
        Ok(this)
    }
}

impl Model {
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_by_locale(
        db: &DatabaseConnection,
        locale: &str,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(i18n_supported_locales::Column::Locale.eq(locale))
            .one(db)
            .await
    }

    /// All rows (enabled + disabled), ordered by `sort_order` then locale.
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn list_all(db: &DatabaseConnection) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .order_by_asc(i18n_supported_locales::Column::SortOrder)
            .order_by_asc(i18n_supported_locales::Column::Locale)
            .all(db)
            .await
    }

    /// Only `is_enabled = true` rows — what the public locale endpoint returns.
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn list_enabled(db: &DatabaseConnection) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(i18n_supported_locales::Column::IsEnabled.eq(true))
            .order_by_asc(i18n_supported_locales::Column::SortOrder)
            .order_by_asc(i18n_supported_locales::Column::Locale)
            .all(db)
            .await
    }
}
