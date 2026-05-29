use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter,
};
use uuid::Uuid;

pub use super::_entities::scheduled_worker_tenant_grants::{
    self, ActiveModel, Entity, Model,
};

#[async_trait]
impl ActiveModelBehavior
    for super::_entities::scheduled_worker_tenant_grants::ActiveModel
{
    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if insert {
            let mut this = self;
            this.id = ActiveValue::Set(crate::utils::id::generate_id());
            this.created_at = ActiveValue::Set(chrono::Utc::now().fixed_offset());
            Ok(this)
        } else {
            Ok(self)
        }
    }
}

impl Model {
    pub async fn find_granted(
        db: &DatabaseConnection,
        worker_def_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_tenant_grants::Column::WorkerDefId.eq(worker_def_id))
            .filter(scheduled_worker_tenant_grants::Column::TenantId.eq(tenant_id))
            .one(db)
            .await
    }

    pub async fn find_grants_for_worker(
        db: &DatabaseConnection,
        worker_def_id: Uuid,
    ) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_tenant_grants::Column::WorkerDefId.eq(worker_def_id))
            .all(db)
            .await
    }

    pub async fn find_granted_worker_ids_for_tenant(
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> Result<Vec<Uuid>, DbErr> {
        let grants = Entity::find()
            .filter(scheduled_worker_tenant_grants::Column::TenantId.eq(tenant_id))
            .all(db)
            .await?;

        Ok(grants
            .into_iter()
            .map(|grant| grant.worker_def_id)
            .collect())
    }

    pub async fn delete_for_worker(
        db: &DatabaseConnection,
        worker_def_id: Uuid,
    ) -> Result<u64, DbErr> {
        let res = Entity::delete_many()
            .filter(scheduled_worker_tenant_grants::Column::WorkerDefId.eq(worker_def_id))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }
}
