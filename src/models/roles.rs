use async_trait::async_trait;
use loco_rs::prelude::*;
use uuid::Uuid;

pub use super::_entities::roles::{self, ActiveModel, Entity, Model};

#[async_trait]
impl ActiveModelBehavior for super::_entities::roles::ActiveModel {
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
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_by_tenant<C: ConnectionTrait>(
        db: &C,
        tenant_id: Uuid,
    ) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(roles::Column::TenantId.eq(tenant_id))
            .all(db)
            .await?)
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_by_id_and_tenant<C: ConnectionTrait>(
        db: &C,
        id: Uuid,
        tenant_id: Uuid,
    ) -> ModelResult<Self> {
        Entity::find()
            .filter(roles::Column::Id.eq(id))
            .filter(roles::Column::TenantId.eq(tenant_id))
            .filter(roles::Column::Status.eq("active"))
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn update_with_version<C: ConnectionTrait>(
        db: &C,
        id: Uuid,
        tenant_id: Uuid,
        mut active_model: roles::ActiveModel,
        current_user_id: Uuid,
    ) -> ModelResult<Self> {
        let existing = Entity::find()
            .filter(roles::Column::Id.eq(id))
            .filter(roles::Column::TenantId.eq(tenant_id))
            .filter(roles::Column::Status.eq("active"))
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

        active_model.id = ActiveValue::Unchanged(id);
        active_model.tenant_id = ActiveValue::Unchanged(tenant_id);
        active_model.version = ActiveValue::Set(existing.version + 1);
        active_model.updated_by = ActiveValue::Set(Some(current_user_id));
        Ok(active_model.update(db).await?)
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn toggle_status<C: ConnectionTrait>(
        db: &C,
        id: Uuid,
        tenant_id: Uuid,
        new_status: &str,
    ) -> ModelResult<Self> {
        let existing = Entity::find()
            .filter(roles::Column::Id.eq(id))
            .filter(roles::Column::TenantId.eq(tenant_id))
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)?;

        let mut active: ActiveModel = existing.into();
        active.status = ActiveValue::Set(new_status.to_string());
        Ok(active.update(db).await?)
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn create_role<C: ConnectionTrait>(
        db: &C,
        tenant_id: Uuid,
        mut active_model: ActiveModel,
        current_user_id: Uuid,
    ) -> ModelResult<Self> {
        active_model.tenant_id = ActiveValue::Set(tenant_id);
        active_model.version = ActiveValue::Set(1);
        active_model.updated_by = ActiveValue::Set(Some(current_user_id));

        if matches!(&active_model.is_system, ActiveValue::NotSet) {
            active_model.is_system = ActiveValue::Set(false);
        }

        active_model.status = ActiveValue::Set("active".to_string());

        Ok(active_model.insert(db).await?)
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_user_role_codes<C: ConnectionTrait>(
        db: &C,
        user_id: Uuid,
        tenant_id: Uuid,
    ) -> ModelResult<Vec<String>> {
        use super::_entities::user_roles;

        let user_role_records = user_roles::Entity::find()
            .filter(user_roles::Column::UserId.eq(user_id))
            .filter(user_roles::Column::TenantId.eq(tenant_id))
            .all(db)
            .await?;

        let role_ids: Vec<Uuid> = user_role_records.iter().map(|ur| ur.role_id).collect();
        if role_ids.is_empty() {
            return Ok(vec![]);
        }

        let roles = roles::Entity::find()
            .filter(roles::Column::Id.is_in(role_ids))
            .filter(roles::Column::Status.eq("active"))
            .all(db)
            .await?;

        Ok(roles.into_iter().map(|r| r.code).collect())
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn sync_user_roles<C: ConnectionTrait>(
        db: &C,
        tenant_id: Uuid,
        user_id: Uuid,
        role_ids: Vec<Uuid>,
    ) -> ModelResult<()> {
        use super::_entities::user_roles;

        user_roles::Entity::delete_many()
            .filter(user_roles::Column::TenantId.eq(tenant_id))
            .filter(user_roles::Column::UserId.eq(user_id))
            .exec(db)
            .await?;

        for role_id in role_ids {
            user_roles::ActiveModel {
                tenant_id: ActiveValue::Set(tenant_id),
                user_id: ActiveValue::Set(user_id),
                role_id: ActiveValue::Set(role_id),
            }
            .insert(db)
            .await?;
        }

        Ok(())
    }
}
