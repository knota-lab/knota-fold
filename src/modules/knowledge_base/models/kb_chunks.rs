use chrono::Utc;
use loco_rs::prelude::*;
use sea_orm::ActiveValue;
use uuid::Uuid;

pub use crate::models::_entities::kb_chunks::{self, ActiveModel, Entity, Model};

#[async_trait::async_trait]
impl ActiveModelBehavior for crate::models::_entities::kb_chunks::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        let mut this = self;
        if insert {
            if this.id.is_not_set() {
                this.id = ActiveValue::Set(Uuid::now_v7());
            }
            if this.created_at.is_not_set() {
                this.created_at = ActiveValue::Set(Utc::now().naive_utc());
            }
        }
        Ok(this)
    }
}
