use async_trait::async_trait;
use loco_rs::prelude::*;
use uuid::Uuid;

pub use super::_entities::tenants::{self, ActiveModel, Entity, Model};

#[async_trait]
impl ActiveModelBehavior for super::_entities::tenants::ActiveModel {
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
    /// Returns a database error if the query fails, or `EntityNotFound` if the tenant does not exist.
    pub async fn find_by_id<C: ConnectionTrait>(db: &C, id: Uuid) -> ModelResult<Self> {
        Entity::find()
            .filter(tenants::Column::Id.eq(id))
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails, or `EntityNotFound` if the tenant does not exist.
    pub async fn find_by_code<C: ConnectionTrait>(
        db: &C,
        code: &str,
    ) -> ModelResult<Self> {
        Entity::find()
            .filter(tenants::Column::Code.eq(code))
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// # Errors
    ///
    /// Returns a database error if the insert fails.
    pub async fn create<C: ConnectionTrait>(
        db: &C,
        active_model: ActiveModel,
    ) -> ModelResult<Self> {
        Ok(active_model.insert(db).await?)
    }

    /// # Errors
    ///
    /// Returns a database error if the query or update fails, or `EntityNotFound` if the tenant does not exist.
    pub async fn update<C: ConnectionTrait>(
        db: &C,
        id: Uuid,
        mut active_model: ActiveModel,
    ) -> ModelResult<Self> {
        Self::find_by_id(db, id).await?;
        active_model.id = ActiveValue::Unchanged(id);
        Ok(active_model.update(db).await?)
    }
}
