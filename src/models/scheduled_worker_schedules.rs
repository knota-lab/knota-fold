use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter, QueryOrder,
};
use uuid::Uuid;

pub use super::_entities::scheduled_worker_schedules::{
    self, ActiveModel, Entity, Model,
};

#[async_trait]
impl ActiveModelBehavior for super::_entities::scheduled_worker_schedules::ActiveModel {
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
    pub async fn find_due(
        db: &DatabaseConnection,
        now: DateTimeWithTimeZone,
    ) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_schedules::Column::Enabled.eq(true))
            .filter(scheduled_worker_schedules::Column::NextRunAt.lte(now))
            .order_by_asc(scheduled_worker_schedules::Column::NextRunAt)
            .all(db)
            .await
    }

    pub async fn find_by_tenant(
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_schedules::Column::TenantId.eq(tenant_id))
            .order_by_asc(scheduled_worker_schedules::Column::Name)
            .all(db)
            .await
    }

    pub async fn find_by_id(
        db: &DatabaseConnection,
        id: Uuid,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_schedules::Column::Id.eq(id))
            .one(db)
            .await
    }

    pub async fn update_next_run_at(
        db: &DatabaseConnection,
        id: Uuid,
        next_run_at: Option<DateTimeWithTimeZone>,
    ) -> Result<Option<Self>, DbErr> {
        let existing = Self::find_by_id(db, id).await?;

        match existing {
            Some(schedule) => {
                let mut active_model: ActiveModel = schedule.into();
                active_model.next_run_at = ActiveValue::Set(next_run_at);
                active_model.update(db).await.map(Some)
            }
            None => Ok(None),
        }
    }

    pub async fn disable_for_worker_and_tenant(
        db: &DatabaseConnection,
        worker_def_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<u64, DbErr> {
        let schedules = Entity::find()
            .filter(scheduled_worker_schedules::Column::WorkerDefId.eq(worker_def_id))
            .filter(scheduled_worker_schedules::Column::TenantId.eq(tenant_id))
            .all(db)
            .await?;

        let mut affected = 0;
        for schedule in schedules {
            let mut active_model: ActiveModel = schedule.into();
            active_model.enabled = ActiveValue::Set(false);
            active_model.update(db).await?;
            affected += 1;
        }

        Ok(affected)
    }
}
