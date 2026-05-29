use crate::utils::error::IntoLocoResult;
use async_trait::async_trait;
use loco_rs::prelude::*;
use uuid::Uuid;

pub use super::_entities::sys_role_templates::{self, ActiveModel, Entity, Model};

#[async_trait]
impl ActiveModelBehavior for super::_entities::sys_role_templates::ActiveModel {
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
        Ok(Entity::find().all(db).await?)
    }

    pub async fn find_by_id<C: ConnectionTrait>(db: &C, id: Uuid) -> ModelResult<Self> {
        Entity::find()
            .filter(sys_role_templates::Column::Id.eq(id))
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    pub async fn create<C: ConnectionTrait>(
        db: &C,
        active_model: ActiveModel,
    ) -> ModelResult<Self> {
        Ok(active_model.insert(db).await?)
    }

    pub async fn update_template<C: ConnectionTrait>(
        db: &C,
        id: Uuid,
        mut active_model: ActiveModel,
    ) -> ModelResult<Self> {
        Self::find_by_id(db, id).await?;
        active_model.id = ActiveValue::Unchanged(id);
        Ok(active_model.update(db).await?)
    }

    pub async fn delete_template<C: ConnectionTrait>(
        db: &C,
        id: Uuid,
    ) -> loco_rs::Result<()> {
        use super::_entities::sys_role_template_menus;
        use super::_entities::sys_role_template_permissions;

        // Delete associated menus and permissions first
        sys_role_template_menus::Entity::delete_many()
            .filter(sys_role_template_menus::Column::TemplateId.eq(id))
            .exec(db)
            .await
            .loco_err()?;

        sys_role_template_permissions::Entity::delete_many()
            .filter(sys_role_template_permissions::Column::TemplateId.eq(id))
            .exec(db)
            .await
            .loco_err()?;

        let template = Self::find_by_id(db, id).await.loco_err()?;

        use sea_orm::ModelTrait;
        template.delete(db).await.loco_err()?;

        Ok(())
    }
}
