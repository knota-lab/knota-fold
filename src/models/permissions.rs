use async_trait::async_trait;
use loco_rs::prelude::*;
use uuid::Uuid;

pub use super::_entities::permissions::{self, ActiveModel, Entity, Model};

#[async_trait]
impl ActiveModelBehavior for super::_entities::permissions::ActiveModel {
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
    pub async fn find_all<C: ConnectionTrait>(db: &C) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(permissions::Column::DeletedAt.is_null())
            .all(db)
            .await?)
    }

    /// Find all permissions including soft-deleted ones.
    /// Used by sync_permissions to detect (obj, act) collisions with deleted records.
    pub async fn find_all_including_deleted<C: ConnectionTrait>(
        db: &C,
    ) -> ModelResult<Vec<Self>> {
        Ok(Entity::find().all(db).await?)
    }

    /// Restore a soft-deleted permission by clearing deleted_at and updating fields.
    pub async fn restore<C: ConnectionTrait>(
        db: &C,
        id: Uuid,
        code: String,
        current_user_id: Uuid,
    ) -> ModelResult<Self> {
        let existing = Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or(ModelError::EntityNotFound)?;

        let mut active: ActiveModel = existing.into();
        active.deleted_at = ActiveValue::Set(None);
        active.code = ActiveValue::Set(code);
        active.version = ActiveValue::Set(1);
        active.updated_by = ActiveValue::Set(Some(current_user_id));
        Ok(active.update(db).await?)
    }

    pub async fn find_by_id<C: ConnectionTrait>(db: &C, id: Uuid) -> ModelResult<Self> {
        Entity::find()
            .filter(permissions::Column::Id.eq(id))
            .filter(permissions::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    pub async fn update_with_version<C: ConnectionTrait>(
        db: &C,
        id: Uuid,
        mut active_model: permissions::ActiveModel,
        current_user_id: Uuid,
    ) -> ModelResult<Self> {
        let existing = Entity::find()
            .filter(permissions::Column::Id.eq(id))
            .filter(permissions::Column::DeletedAt.is_null())
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
        active_model.version = ActiveValue::Set(existing.version + 1);
        active_model.updated_by = ActiveValue::Set(Some(current_user_id));
        Ok(active_model.update(db).await?)
    }

    pub async fn soft_delete<C: ConnectionTrait>(db: &C, id: Uuid) -> ModelResult<Self> {
        let existing = Entity::find()
            .filter(permissions::Column::Id.eq(id))
            .filter(permissions::Column::DeletedAt.is_null())
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)?;

        let mut active: ActiveModel = existing.into();
        active.deleted_at = ActiveValue::Set(Some(chrono::Utc::now().into()));
        Ok(active.update(db).await?)
    }

    pub async fn create_permission<C: ConnectionTrait>(
        db: &C,
        mut active_model: ActiveModel,
        current_user_id: Uuid,
    ) -> ModelResult<Self> {
        active_model.version = ActiveValue::Set(1);
        active_model.updated_by = ActiveValue::Set(Some(current_user_id));

        if matches!(&active_model.is_system, ActiveValue::NotSet) {
            active_model.is_system = ActiveValue::Set(false);
        }

        Ok(active_model.insert(db).await?)
    }

    pub async fn sync_role_permissions<C: ConnectionTrait>(
        db: &C,
        tenant_id: Uuid,
        role_id: Uuid,
        permission_ids: Vec<Uuid>,
    ) -> ModelResult<()> {
        use super::_entities::role_permissions;

        role_permissions::Entity::delete_many()
            .filter(role_permissions::Column::TenantId.eq(tenant_id))
            .filter(role_permissions::Column::RoleId.eq(role_id))
            .exec(db)
            .await?;

        for permission_id in permission_ids {
            role_permissions::ActiveModel {
                tenant_id: ActiveValue::Set(tenant_id),
                role_id: ActiveValue::Set(role_id),
                permission_id: ActiveValue::Set(permission_id),
            }
            .insert(db)
            .await?;
        }

        Ok(())
    }

    pub async fn find_role_permission_ids<C: ConnectionTrait>(
        db: &C,
        role_id: Uuid,
        tenant_id: Uuid,
    ) -> ModelResult<Vec<Uuid>> {
        use super::_entities::role_permissions;
        let records = role_permissions::Entity::find()
            .filter(role_permissions::Column::RoleId.eq(role_id))
            .filter(role_permissions::Column::TenantId.eq(tenant_id))
            .all(db)
            .await?;
        Ok(records.iter().map(|rp| rp.permission_id).collect())
    }

    pub async fn find_role_permission_obj_acts<C: ConnectionTrait>(
        db: &C,
        role_id: Uuid,
        tenant_id: Uuid,
    ) -> ModelResult<Vec<(String, String)>> {
        use super::_entities::role_permissions;

        let rp_records = role_permissions::Entity::find()
            .filter(role_permissions::Column::RoleId.eq(role_id))
            .filter(role_permissions::Column::TenantId.eq(tenant_id))
            .all(db)
            .await?;

        let perm_ids: Vec<Uuid> = rp_records.iter().map(|rp| rp.permission_id).collect();
        if perm_ids.is_empty() {
            return Ok(vec![]);
        }

        let perms = permissions::Entity::find()
            .filter(permissions::Column::Id.is_in(perm_ids))
            .filter(permissions::Column::DeletedAt.is_null())
            .all(db)
            .await?;

        Ok(perms.into_iter().map(|p| (p.obj, p.act)).collect())
    }

    /// Find an existing permission by (obj, act), or create one if not found.
    /// If a soft-deleted record exists with the same (obj, act), restore it.
    /// Permissions are global — no tenant isolation.
    /// Used during tenant initialization from role templates.
    pub async fn find_or_create_by_obj_act<C: ConnectionTrait>(
        db: &C,
        obj: &str,
        act: &str,
    ) -> ModelResult<Self> {
        // Query including soft-deleted to avoid unique constraint violations
        let existing = permissions::Entity::find()
            .filter(permissions::Column::Obj.eq(obj))
            .filter(permissions::Column::Act.eq(act))
            .one(db)
            .await?;

        match existing {
            Some(perm) if perm.deleted_at.is_none() => Ok(perm),
            Some(perm) => {
                // Restore soft-deleted record
                let name_code = format!("{obj}:{act}");
                Self::restore(db, perm.id, name_code, perm.updated_by.unwrap_or(perm.id))
                    .await
            }
            None => {
                let name_code = format!("{obj}:{act}");
                let am = ActiveModel {
                    name: ActiveValue::Set(name_code.clone()),
                    code: ActiveValue::Set(name_code),
                    obj: ActiveValue::Set(obj.to_string()),
                    act: ActiveValue::Set(act.to_string()),
                    permission_type: ActiveValue::Set("api".to_string()),
                    is_system: ActiveValue::Set(true),
                    version: ActiveValue::Set(1),
                    ..Default::default()
                };

                Ok(am.insert(db).await?)
            }
        }
    }
}
