use async_trait::async_trait;
use loco_rs::prelude::*;
use uuid::Uuid;

pub use super::_entities::sys_menus::{self, ActiveModel, Entity, Model};

const MAX_TREE_DEPTH: usize = 10;

#[async_trait]
impl ActiveModelBehavior for super::_entities::sys_menus::ActiveModel {
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

impl Model {
    /// All non-deleted `sys_menus` (for super admin listing)
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_all_not_deleted(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(sys_menus::Column::DeletedAt.is_null())
            .all(db)
            .await?)
    }

    /// All active, non-deleted `sys_menus` (for tenant/user queries)
    ///
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_active(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(sys_menus::Column::DeletedAt.is_null())
            .filter(sys_menus::Column::Status.eq("active"))
            .all(db)
            .await?)
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails, or `EntityNotFound` if the menu does not exist.
    pub async fn find_by_id(db: &DatabaseConnection, id: Uuid) -> ModelResult<Self> {
        Entity::find()
            .filter(sys_menus::Column::Id.eq(id))
            .filter(sys_menus::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails, `EntityNotFound` if the menu does not exist,
    /// or a version conflict error.
    pub async fn update_with_version(
        db: &DatabaseConnection,
        id: Uuid,
        mut active_model: sys_menus::ActiveModel,
        current_user_id: Uuid,
    ) -> ModelResult<Self> {
        let existing = Entity::find()
            .filter(sys_menus::Column::Id.eq(id))
            .filter(sys_menus::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)?;

        match active_model.version {
            ActiveValue::Set(v) if v != existing.version => {
                return Err(ModelError::msg(
                    "Version conflict, please refresh and try again",
                ));
            }
            ActiveValue::Set(_) => {}
            _ => return Err(ModelError::msg("version field is required for updates")),
        }

        let next_parent_id = match active_model.parent_id.clone() {
            ActiveValue::Set(parent_id) | ActiveValue::Unchanged(parent_id) => parent_id,
            ActiveValue::NotSet => existing.parent_id,
        };

        Self::validate_tree_depth(db, next_parent_id).await?;
        Self::validate_no_circular_ref(db, id, next_parent_id).await?;

        active_model.id = ActiveValue::Unchanged(id);
        active_model.version = ActiveValue::Set(existing.version + 1);
        active_model.updated_by = ActiveValue::Set(Some(current_user_id));
        Ok(active_model.update(db).await?)
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails, or `EntityNotFound` if the menu does not exist.
    pub async fn soft_delete(db: &DatabaseConnection, id: Uuid) -> ModelResult<Self> {
        let existing = Entity::find()
            .filter(sys_menus::Column::Id.eq(id))
            .filter(sys_menus::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)?;

        let mut active: ActiveModel = existing.into();
        active.deleted_at = ActiveValue::Set(Some(chrono::Utc::now().into()));
        Ok(active.update(db).await?)
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn create_menu(
        db: &DatabaseConnection,
        mut active_model: ActiveModel,
        current_user_id: Uuid,
    ) -> ModelResult<Self> {
        let parent_id = match active_model.parent_id.clone() {
            ActiveValue::Set(parent_id) | ActiveValue::Unchanged(parent_id) => parent_id,
            ActiveValue::NotSet => None,
        };

        Self::validate_tree_depth(db, parent_id).await?;

        active_model.version = ActiveValue::Set(1);
        active_model.updated_by = ActiveValue::Set(Some(current_user_id));

        if matches!(&active_model.is_cache, ActiveValue::NotSet) {
            active_model.is_cache = ActiveValue::Set(false);
        }
        if matches!(&active_model.sort_order, ActiveValue::NotSet) {
            active_model.sort_order = ActiveValue::Set(0);
        }
        if matches!(&active_model.status, ActiveValue::NotSet) {
            active_model.status = ActiveValue::Set("active".to_string());
        }

        Ok(active_model.insert(db).await?)
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails, or an error if maximum tree depth is exceeded.
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

            let menu = Entity::find()
                .filter(sys_menus::Column::Id.eq(pid))
                .one(db)
                .await?;
            current_parent = menu.and_then(|m| m.parent_id);
        }

        Ok(())
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails, or an error if a circular reference is detected.
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

            let menu = Entity::find()
                .filter(sys_menus::Column::Id.eq(pid))
                .one(db)
                .await?;
            current = menu.and_then(|m| m.parent_id);
        }

        Ok(())
    }
}
