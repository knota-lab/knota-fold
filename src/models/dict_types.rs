use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::{ConnectionTrait, PaginatorTrait};
use uuid::Uuid;

pub use super::_entities::dict_types::{self, ActiveModel, Entity, Model};

#[async_trait]
impl ActiveModelBehavior for super::_entities::dict_types::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if insert {
            let mut this = self;
            this.id = ActiveValue::Set(crate::utils::id::generate_id());
            Ok(this)
        } else {
            Ok(self)
        }
    }
}

/// Effective dict type with computed `scope` column.
#[derive(Debug, Clone)]
pub struct EffectiveDictType {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub source_type_id: Option<Uuid>,
    pub code: String,
    pub name: String,
    pub status: String,
    pub description: Option<String>,
    pub version: i32,
    pub updated_by: Option<Uuid>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
    pub deleted_at: Option<DateTimeWithTimeZone>,
    pub scope: String,
}

impl EffectiveDictType {
    fn from_model(m: &Model, scope: &str) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            source_type_id: m.source_type_id,
            code: m.code.clone(),
            name: m.name.clone(),
            status: m.status.clone(),
            description: m.description.clone(),
            version: m.version,
            updated_by: m.updated_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
            deleted_at: m.deleted_at,
            scope: scope.to_string(),
        }
    }
}

impl Model {
    /// Find a dict type by ID (no tenant filter — used for looking up any row).
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_by_id(db: &DatabaseConnection, id: Uuid) -> ModelResult<Self> {
        Entity::find()
            .filter(dict_types::Column::Id.eq(id))
            .filter(dict_types::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Find system dict types (`tenant_id` IS NULL, not deleted).
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_system_types(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(dict_types::Column::TenantId.is_null())
            .filter(dict_types::Column::DeletedAt.is_null())
            .all(db)
            .await?)
    }

    /// Find a system dict type by code.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_system_type_by_code(
        db: &DatabaseConnection,
        code: &str,
    ) -> ModelResult<Self> {
        Entity::find()
            .filter(dict_types::Column::TenantId.is_null())
            .filter(dict_types::Column::Code.eq(code))
            .filter(dict_types::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Find the override row a tenant has for a specific system type.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_override_by_tenant_and_source(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        source_type_id: Uuid,
    ) -> ModelResult<Self> {
        Entity::find()
            .filter(dict_types::Column::TenantId.eq(tenant_id))
            .filter(dict_types::Column::SourceTypeId.eq(source_type_id))
            .filter(dict_types::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Find dict type by id, scoped to a tenant (owns or system).
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_by_id_and_tenant(
        db: &DatabaseConnection,
        id: Uuid,
        tenant_id: Uuid,
    ) -> ModelResult<Self> {
        // Try tenant-owned first
        let result = Entity::find()
            .filter(dict_types::Column::Id.eq(id))
            .filter(dict_types::Column::TenantId.eq(tenant_id))
            .filter(dict_types::Column::DeletedAt.is_null())
            .one(db)
            .await?;

        if let Some(m) = result {
            return Ok(m);
        }

        // Fall back to system row
        Entity::find()
            .filter(dict_types::Column::Id.eq(id))
            .filter(dict_types::Column::TenantId.is_null())
            .filter(dict_types::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Effective dict types for a tenant — 3 ORM queries merged in memory.
    /// Returns tenant overrides + tenant-only + system rows not overridden.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    #[allow(clippy::cast_possible_truncation)] // DB pagination: page/page_size are bounded by app config
    pub async fn find_effective_types(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        page: u64,
        page_size: u64,
    ) -> ModelResult<(Vec<EffectiveDictType>, u64)> {
        // 1. Tenant override rows
        let overrides = Entity::find()
            .filter(dict_types::Column::TenantId.eq(tenant_id))
            .filter(dict_types::Column::SourceTypeId.is_not_null())
            .filter(dict_types::Column::DeletedAt.is_null())
            .all(db)
            .await?;

        // 2. Tenant-only rows
        let tenant_only = Entity::find()
            .filter(dict_types::Column::TenantId.eq(tenant_id))
            .filter(dict_types::Column::SourceTypeId.is_null())
            .filter(dict_types::Column::DeletedAt.is_null())
            .all(db)
            .await?;

        // 3. System rows (active, not overridden by this tenant)
        let overridden_ids: Vec<Uuid> =
            overrides.iter().filter_map(|o| o.source_type_id).collect();

        let mut system_query = Entity::find()
            .filter(dict_types::Column::TenantId.is_null())
            .filter(dict_types::Column::DeletedAt.is_null())
            .filter(dict_types::Column::Status.eq("active"));

        if !overridden_ids.is_empty() {
            system_query =
                system_query.filter(dict_types::Column::Id.is_not_in(overridden_ids));
        }

        let system = system_query.all(db).await?;

        // Merge
        let mut results: Vec<EffectiveDictType> = Vec::new();
        for m in &overrides {
            results.push(EffectiveDictType::from_model(m, "override"));
        }
        for m in &tenant_only {
            results.push(EffectiveDictType::from_model(m, "tenantOnly"));
        }
        for m in &system {
            results.push(EffectiveDictType::from_model(m, "system"));
        }

        // Sort by code
        results.sort_by(|a, b| a.code.cmp(&b.code));

        // Paginate in memory
        let total = results.len() as u64;
        let offset = ((page - 1) * page_size) as usize;
        let items: Vec<EffectiveDictType> = results
            .into_iter()
            .skip(offset)
            .take(page_size as usize)
            .collect();

        Ok((items, total))
    }

    /// System dict types list (for super admin) with pagination.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    #[tracing::instrument(skip_all)]
    pub async fn find_system_types_paginated(
        db: &DatabaseConnection,
        page: u64,
        page_size: u64,
    ) -> ModelResult<(Vec<Self>, u64)> {
        let base_query = Entity::find()
            .filter(dict_types::Column::TenantId.is_null())
            .filter(dict_types::Column::DeletedAt.is_null());

        let paginator = base_query.paginate(db, page_size);
        let total = paginator.num_items().await?;
        let rows = paginator.fetch_page(page - 1).await?;

        Ok((rows, total))
    }

    /// Update with optimistic locking. `tenant_id` can be None for system rows.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn update_with_version(
        db: &DatabaseConnection,
        id: Uuid,
        mut active_model: dict_types::ActiveModel,
        current_user_id: Uuid,
    ) -> ModelResult<Self> {
        let existing = Self::find_by_id(db, id).await?;

        match active_model.version {
            ActiveValue::Set(v) if v != existing.version => {
                return Err(ModelError::msg(
                    "Version conflict, please refresh and try again",
                ));
            }
            ActiveValue::Set(_) => {}
            _ => return Err(ModelError::msg("version field is required for updates")),
        }

        active_model.id = ActiveValue::Unchanged(id);
        active_model.version = ActiveValue::Set(existing.version + 1);
        active_model.updated_by = ActiveValue::Set(Some(current_user_id));
        Ok(active_model.update(db).await?)
    }

    /// Soft delete a dict type.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn soft_delete(db: &DatabaseConnection, id: Uuid) -> ModelResult<Self> {
        let existing = Self::find_by_id(db, id).await?;
        let mut active: ActiveModel = existing.into();
        active.deleted_at = ActiveValue::Set(Some(chrono::Utc::now().into()));
        Ok(active.update(db).await?)
    }

    /// Create a dict type.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn create_dict_type(
        db: &DatabaseConnection,
        tenant_id: Option<Uuid>,
        mut active_model: ActiveModel,
        current_user_id: Uuid,
    ) -> ModelResult<Self> {
        active_model.tenant_id = ActiveValue::Set(tenant_id);
        active_model.version = ActiveValue::Set(1);
        active_model.status = ActiveValue::Set("active".to_string());
        active_model.updated_by = ActiveValue::Set(Some(current_user_id));

        Ok(active_model.insert(db).await?)
    }
}
