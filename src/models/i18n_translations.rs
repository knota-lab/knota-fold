use std::collections::HashSet;

use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
};
use uuid::Uuid;

pub use super::_entities::i18n_translations::{self, ActiveModel, Entity, Model};

pub const SCOPE_GLOBAL: &str = "global";
pub const SCOPE_TENANT: &str = "tenant";

#[async_trait]
impl ActiveModelBehavior for super::_entities::i18n_translations::ActiveModel {
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
    /// Batch-check which `(namespace, key, locale)` triples already exist in global scope.
    pub async fn find_existing_global_keys<C: ConnectionTrait>(
        db: &C,
        triples: &[(String, String, String)],
    ) -> Result<HashSet<(String, String, String)>, DbErr> {
        if triples.is_empty() {
            return Ok(HashSet::new());
        }

        let triples: HashSet<(String, String, String)> =
            triples.iter().cloned().collect();
        let mut filter = Condition::any();
        for (namespace, key, locale) in &triples {
            filter = filter.add(
                Condition::all()
                    .add(i18n_translations::Column::Namespace.eq(namespace.as_str()))
                    .add(i18n_translations::Column::Key.eq(key.as_str()))
                    .add(i18n_translations::Column::Locale.eq(locale.as_str()))
                    .add(i18n_translations::Column::TenantId.is_null()),
            );
        }

        let rows = Entity::find().filter(filter).all(db).await?;

        Ok(rows
            .into_iter()
            .map(|row| (row.namespace, row.key, row.locale))
            .collect())
    }

    /// Batch-check which `(namespace, key, locale)` triples already exist for a tenant.
    pub async fn find_existing_tenant_keys<C: ConnectionTrait>(
        db: &C,
        tenant_id: Uuid,
        triples: &[(String, String, String)],
    ) -> Result<HashSet<(String, String, String)>, DbErr> {
        if triples.is_empty() {
            return Ok(HashSet::new());
        }

        let triples: HashSet<(String, String, String)> =
            triples.iter().cloned().collect();
        let mut filter = Condition::any();
        for (namespace, key, locale) in &triples {
            filter = filter.add(
                Condition::all()
                    .add(i18n_translations::Column::Namespace.eq(namespace.as_str()))
                    .add(i18n_translations::Column::Key.eq(key.as_str()))
                    .add(i18n_translations::Column::Locale.eq(locale.as_str()))
                    .add(i18n_translations::Column::TenantId.eq(tenant_id)),
            );
        }

        let rows = Entity::find().filter(filter).all(db).await?;

        Ok(rows
            .into_iter()
            .map(|row| (row.namespace, row.key, row.locale))
            .collect())
    }

    pub async fn find_global<C: ConnectionTrait>(
        db: &C,
        namespace: &str,
        key: &str,
        locale: &str,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(i18n_translations::Column::Namespace.eq(namespace))
            .filter(i18n_translations::Column::Key.eq(key))
            .filter(i18n_translations::Column::Locale.eq(locale))
            .filter(i18n_translations::Column::TenantId.is_null())
            .one(db)
            .await
    }

    pub async fn find_tenant<C: ConnectionTrait>(
        db: &C,
        namespace: &str,
        key: &str,
        locale: &str,
        tenant_id: Uuid,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(i18n_translations::Column::Namespace.eq(namespace))
            .filter(i18n_translations::Column::Key.eq(key))
            .filter(i18n_translations::Column::Locale.eq(locale))
            .filter(i18n_translations::Column::TenantId.eq(tenant_id))
            .one(db)
            .await
    }

    /// All global rows for a (namespace, locale).
    pub async fn list_global_bundle<C: ConnectionTrait>(
        db: &C,
        namespace: &str,
        locale: &str,
    ) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(i18n_translations::Column::Namespace.eq(namespace))
            .filter(i18n_translations::Column::Locale.eq(locale))
            .filter(i18n_translations::Column::TenantId.is_null())
            .all(db)
            .await
    }

    /// All tenant override rows for a (namespace, locale, tenant).
    pub async fn list_tenant_bundle<C: ConnectionTrait>(
        db: &C,
        namespace: &str,
        locale: &str,
        tenant_id: Uuid,
    ) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(i18n_translations::Column::Namespace.eq(namespace))
            .filter(i18n_translations::Column::Locale.eq(locale))
            .filter(i18n_translations::Column::TenantId.eq(tenant_id))
            .all(db)
            .await
    }
}
