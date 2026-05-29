use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::{ActiveValue, ConnectionTrait};

pub use super::_entities::i18n_entry_locations::{self, ActiveModel, Entity, Model};

#[async_trait]
impl ActiveModelBehavior for super::_entities::i18n_entry_locations::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        let mut this = self;
        if insert {
            if matches!(this.id, ActiveValue::NotSet) {
                this.id = ActiveValue::Set(crate::utils::id::generate_id());
            }
            if matches!(this.created_at, ActiveValue::NotSet) {
                this.created_at = ActiveValue::Set(chrono::Utc::now().fixed_offset());
            }
        }
        Ok(this)
    }
}
