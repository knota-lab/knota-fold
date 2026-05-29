use async_trait::async_trait;
use loco_rs::prelude::*;
use uuid::Uuid;

pub use super::_entities::tenant_menu_overrides::{self, ActiveModel, Entity, Model};

#[async_trait]
impl ActiveModelBehavior for super::_entities::tenant_menu_overrides::ActiveModel {
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
    pub async fn find_by_tenant(
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(tenant_menu_overrides::Column::TenantId.eq(tenant_id))
            .all(db)
            .await?)
    }

    pub async fn find_by_tenant_and_menu(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        sys_menu_id: Uuid,
    ) -> ModelResult<Option<Self>> {
        Ok(Entity::find()
            .filter(tenant_menu_overrides::Column::TenantId.eq(tenant_id))
            .filter(tenant_menu_overrides::Column::SysMenuId.eq(sys_menu_id))
            .one(db)
            .await?)
    }

    pub async fn upsert(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        sys_menu_id: Uuid,
        mut active_model: tenant_menu_overrides::ActiveModel,
        current_user_id: Uuid,
    ) -> ModelResult<Self> {
        let existing = Self::find_by_tenant_and_menu(db, tenant_id, sys_menu_id).await?;

        if let Some(record) = existing {
            active_model.id = ActiveValue::Unchanged(record.id);
            active_model.tenant_id = ActiveValue::Unchanged(tenant_id);
            active_model.sys_menu_id = ActiveValue::Unchanged(sys_menu_id);
            active_model.version = ActiveValue::Set(record.version + 1);
            active_model.updated_by = ActiveValue::Set(Some(current_user_id));
            Ok(active_model.update(db).await?)
        } else {
            active_model.tenant_id = ActiveValue::Set(tenant_id);
            active_model.sys_menu_id = ActiveValue::Set(sys_menu_id);
            active_model.version = ActiveValue::Set(1);
            active_model.updated_by = ActiveValue::Set(Some(current_user_id));

            if matches!(&active_model.is_hidden, ActiveValue::NotSet) {
                active_model.is_hidden = ActiveValue::Set(false);
            }

            Ok(active_model.insert(db).await?)
        }
    }

    pub async fn delete_override(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        sys_menu_id: Uuid,
    ) -> ModelResult<()> {
        let result = Entity::delete_many()
            .filter(tenant_menu_overrides::Column::TenantId.eq(tenant_id))
            .filter(tenant_menu_overrides::Column::SysMenuId.eq(sys_menu_id))
            .exec(db)
            .await?;

        if result.rows_affected == 0 {
            return Err(ModelError::EntityNotFound);
        }
        Ok(())
    }
}
