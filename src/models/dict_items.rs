use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::ConnectionTrait;
use uuid::Uuid;

pub use super::_entities::dict_items::{self, ActiveModel, Entity, Model};

const MAX_TREE_DEPTH: usize = 10;

#[async_trait]
impl ActiveModelBehavior for super::_entities::dict_items::ActiveModel {
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

/// Effective dict item with computed `scope` and `base_item_id`.
#[derive(Debug, Clone)]
pub struct EffectiveDictItem {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub dict_type_id: Uuid,
    pub source_item_id: Option<Uuid>,
    pub code: String,
    pub name: String,
    pub value: String,
    pub parent_id: Option<Uuid>,
    pub sort_order: i32,
    pub status: String,
    pub description: Option<String>,
    pub version: i32,
    pub updated_by: Option<Uuid>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
    pub deleted_at: Option<DateTimeWithTimeZone>,
    pub scope: String,
    pub base_item_id: Uuid,
}

impl EffectiveDictItem {
    fn from_model(m: &Model, scope: &str) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            dict_type_id: m.dict_type_id,
            source_item_id: m.source_item_id,
            code: m.code.clone(),
            name: m.name.clone(),
            value: m.value.clone(),
            parent_id: m.parent_id,
            sort_order: m.sort_order,
            status: m.status.clone(),
            description: m.description.clone(),
            version: m.version,
            updated_by: m.updated_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
            deleted_at: m.deleted_at,
            scope: scope.to_string(),
            base_item_id: m.source_item_id.unwrap_or(m.id),
        }
    }
}

impl Model {
    /// Find a dict item by ID (no tenant filter).
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_by_id(db: &DatabaseConnection, id: Uuid) -> ModelResult<Self> {
        Entity::find()
            .filter(dict_items::Column::Id.eq(id))
            .filter(dict_items::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Find dict item by id, scoped to a tenant (owns or system).
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_by_id_and_tenant(
        db: &DatabaseConnection,
        id: Uuid,
        tenant_id: Uuid,
    ) -> ModelResult<Self> {
        let result = Entity::find()
            .filter(dict_items::Column::Id.eq(id))
            .filter(dict_items::Column::TenantId.eq(tenant_id))
            .filter(dict_items::Column::DeletedAt.is_null())
            .one(db)
            .await?;

        if let Some(m) = result {
            return Ok(m);
        }

        Entity::find()
            .filter(dict_items::Column::Id.eq(id))
            .filter(dict_items::Column::TenantId.is_null())
            .filter(dict_items::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Find the override row a tenant has for a specific system item.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_override_by_tenant_and_source(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        source_item_id: Uuid,
    ) -> ModelResult<Self> {
        Entity::find()
            .filter(dict_items::Column::TenantId.eq(tenant_id))
            .filter(dict_items::Column::SourceItemId.eq(source_item_id))
            .filter(dict_items::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Find system items for a given dict type.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_system_items_by_type(
        db: &DatabaseConnection,
        dict_type_id: Uuid,
    ) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(dict_items::Column::TenantId.is_null())
            .filter(dict_items::Column::DictTypeId.eq(dict_type_id))
            .filter(dict_items::Column::DeletedAt.is_null())
            .all(db)
            .await?)
    }

    /// Effective dict items for a tenant under a given `dict_type_id` — 3 ORM queries merged.
    /// Returns all effective items (no pagination — tree built in memory).
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_effective_items(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        dict_type_id: Uuid,
    ) -> ModelResult<Vec<EffectiveDictItem>> {
        // 1. Override items
        let overrides = Entity::find()
            .filter(dict_items::Column::TenantId.eq(tenant_id))
            .filter(dict_items::Column::SourceItemId.is_not_null())
            .filter(dict_items::Column::DictTypeId.eq(dict_type_id))
            .filter(dict_items::Column::DeletedAt.is_null())
            .all(db)
            .await?;

        // 2. Tenant-only items
        let tenant_only = Entity::find()
            .filter(dict_items::Column::TenantId.eq(tenant_id))
            .filter(dict_items::Column::SourceItemId.is_null())
            .filter(dict_items::Column::DictTypeId.eq(dict_type_id))
            .filter(dict_items::Column::DeletedAt.is_null())
            .all(db)
            .await?;

        // 3. System items (active, not overridden by this tenant)
        let overridden_ids: Vec<Uuid> =
            overrides.iter().filter_map(|o| o.source_item_id).collect();

        let mut system_query = Entity::find()
            .filter(dict_items::Column::TenantId.is_null())
            .filter(dict_items::Column::DictTypeId.eq(dict_type_id))
            .filter(dict_items::Column::DeletedAt.is_null())
            .filter(dict_items::Column::Status.eq("active"));

        if !overridden_ids.is_empty() {
            system_query =
                system_query.filter(dict_items::Column::Id.is_not_in(overridden_ids));
        }

        let system = system_query.all(db).await?;

        // Merge
        let mut results: Vec<EffectiveDictItem> = Vec::new();
        for m in &overrides {
            results.push(EffectiveDictItem::from_model(m, "override"));
        }
        for m in &tenant_only {
            results.push(EffectiveDictItem::from_model(m, "tenantOnly"));
        }
        for m in &system {
            results.push(EffectiveDictItem::from_model(m, "system"));
        }

        Ok(results)
    }

    /// System items for a dict type (for super admin — no merge needed).
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_items_by_type(
        db: &DatabaseConnection,
        dict_type_id: Uuid,
    ) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(dict_items::Column::DictTypeId.eq(dict_type_id))
            .filter(dict_items::Column::DeletedAt.is_null())
            .all(db)
            .await?)
    }

    /// Find all override items a tenant has for a given source dict type.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_tenant_overrides_for_type(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        dict_type_id: Uuid,
    ) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(dict_items::Column::TenantId.eq(tenant_id))
            .filter(dict_items::Column::DictTypeId.eq(dict_type_id))
            .filter(dict_items::Column::SourceItemId.is_not_null())
            .filter(dict_items::Column::DeletedAt.is_null())
            .all(db)
            .await?)
    }

    /// Update with optimistic locking.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn update_with_version(
        db: &DatabaseConnection,
        id: Uuid,
        mut active_model: dict_items::ActiveModel,
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

    /// Soft delete.
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

    /// Create a dict item.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn create_dict_item(
        db: &DatabaseConnection,
        tenant_id: Option<Uuid>,
        mut active_model: ActiveModel,
        current_user_id: Uuid,
    ) -> ModelResult<Self> {
        active_model.tenant_id = ActiveValue::Set(tenant_id);
        active_model.version = ActiveValue::Set(1);
        active_model.status = ActiveValue::Set("active".to_string());
        active_model.updated_by = ActiveValue::Set(Some(current_user_id));

        if matches!(&active_model.sort_order, ActiveValue::NotSet) {
            active_model.sort_order = ActiveValue::Set(0);
        }

        Ok(active_model.insert(db).await?)
    }

    /// Validate tree depth by walking parent chain.
    /// Uses the effective item set (system + overrides) to resolve parents.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn validate_tree_depth(
        db: &DatabaseConnection,
        parent_id: Option<Uuid>,
    ) -> ModelResult<()> {
        let mut depth = 0;
        let mut current_parent = parent_id;

        while let Some(pid) = current_parent {
            depth += 1;
            if depth > MAX_TREE_DEPTH {
                return Err(ModelError::msg("Maximum tree depth exceeded"));
            }

            let item = Entity::find()
                .filter(dict_items::Column::Id.eq(pid))
                .filter(dict_items::Column::DeletedAt.is_null())
                .one(db)
                .await?;
            current_parent = item.and_then(|m| m.parent_id);
        }

        Ok(())
    }

    /// Validate no circular reference.
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn validate_no_circular_ref(
        db: &DatabaseConnection,
        id: Uuid,
        parent_id: Option<Uuid>,
    ) -> ModelResult<()> {
        let mut current = parent_id;

        while let Some(pid) = current {
            if pid == id {
                return Err(ModelError::msg("Circular reference detected"));
            }

            let item = Entity::find()
                .filter(dict_items::Column::Id.eq(pid))
                .filter(dict_items::Column::DeletedAt.is_null())
                .one(db)
                .await?;
            current = item.and_then(|m| m.parent_id);
        }

        Ok(())
    }
}
