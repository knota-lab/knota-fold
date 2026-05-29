use loco_rs::prelude::*;

pub use super::_entities::audit_logs::{self, ActiveModel, Entity, Model};

#[async_trait::async_trait]
impl ActiveModelBehavior for super::_entities::audit_logs::ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if insert && self.id.is_not_set() {
            let mut this = self;
            this.id = ActiveValue::Set(crate::utils::id::generate_id());
            Ok(this)
        } else {
            Ok(self)
        }
    }
}
