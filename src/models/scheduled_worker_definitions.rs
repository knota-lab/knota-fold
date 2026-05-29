use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter,
};

pub use super::_entities::scheduled_worker_definitions::{
    self, ActiveModel, Entity, Model,
};

#[async_trait]
impl ActiveModelBehavior for super::_entities::scheduled_worker_definitions::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if insert {
            let mut this = self;
            this.id = ActiveValue::Set(crate::utils::id::generate_id());
            let now = chrono::Utc::now().fixed_offset();
            this.created_at = ActiveValue::Set(now);
            this.updated_at = ActiveValue::Set(now);
            Ok(this)
        } else {
            let mut this = self;
            this.updated_at = ActiveValue::Set(chrono::Utc::now().fixed_offset());
            Ok(this)
        }
    }
}

impl Model {
    pub async fn find_by_code(
        db: &DatabaseConnection,
        code: &str,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_definitions::Column::Code.eq(code))
            .one(db)
            .await
    }

    pub async fn find_active_by_code(
        db: &DatabaseConnection,
        code: &str,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_definitions::Column::Code.eq(code))
            .filter(scheduled_worker_definitions::Column::Status.eq("active"))
            .one(db)
            .await
    }

    pub async fn find_all_active(db: &DatabaseConnection) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_definitions::Column::Status.eq("active"))
            .all(db)
            .await
    }
}
