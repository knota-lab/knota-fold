use async_trait::async_trait;
use loco_rs::prelude::*;
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder,
};
use uuid::Uuid;

pub use super::_entities::scheduled_worker_executions::{
    self, ActiveModel, Entity, Model,
};

#[async_trait]
impl ActiveModelBehavior for super::_entities::scheduled_worker_executions::ActiveModel {
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
    pub async fn create_pending(
        db: &DatabaseConnection,
        schedule_id: Uuid,
        worker_def_id: Uuid,
        tenant_id: Uuid,
        trigger_type: &str,
        params_json: Option<String>,
        triggered_by: Option<Uuid>,
        traceparent: Option<String>,
        parent_span_id: Option<String>,
    ) -> Result<Self, DbErr> {
        let active_model = ActiveModel {
            schedule_id: ActiveValue::Set(schedule_id),
            worker_def_id: ActiveValue::Set(worker_def_id),
            tenant_id: ActiveValue::Set(tenant_id),
            trigger_type: ActiveValue::Set(trigger_type.to_string()),
            triggered_by: ActiveValue::Set(triggered_by),
            params_json: ActiveValue::Set(params_json),
            status: ActiveValue::Set("pending".to_string()),
            retry_count: ActiveValue::Set(0),
            started_at: ActiveValue::Set(None),
            finished_at: ActiveValue::Set(None),
            duration_ms: ActiveValue::Set(None),
            output: ActiveValue::Set(None),
            error_message: ActiveValue::Set(None),
            traceparent: ActiveValue::Set(traceparent),
            parent_span_id: ActiveValue::Set(parent_span_id),
            ..Default::default()
        };

        active_model.insert(db).await
    }

    pub async fn find_running_for_schedule(
        db: &DatabaseConnection,
        schedule_id: Uuid,
    ) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_executions::Column::ScheduleId.eq(schedule_id))
            .filter(scheduled_worker_executions::Column::Status.eq("running"))
            .order_by_desc(scheduled_worker_executions::Column::CreatedAt)
            .all(db)
            .await
    }

    pub async fn count_running_for_tenant(
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> Result<u64, DbErr> {
        Entity::find()
            .filter(scheduled_worker_executions::Column::TenantId.eq(tenant_id))
            .filter(scheduled_worker_executions::Column::Status.eq("running"))
            .count(db)
            .await
    }

    pub async fn find_zombies(
        db: &DatabaseConnection,
        cutoff: DateTimeWithTimeZone,
    ) -> Result<Vec<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_executions::Column::Status.eq("running"))
            .filter(scheduled_worker_executions::Column::StartedAt.lte(cutoff))
            .filter(scheduled_worker_executions::Column::FinishedAt.is_null())
            .order_by_asc(scheduled_worker_executions::Column::StartedAt)
            .all(db)
            .await
    }

    pub async fn update_status(
        db: &DatabaseConnection,
        id: Uuid,
        status: &str,
        started_at: Option<DateTimeWithTimeZone>,
        finished_at: Option<DateTimeWithTimeZone>,
        duration_ms: Option<i32>,
        output: Option<String>,
        error_message: Option<String>,
        traceparent: Option<String>,
    ) -> Result<Option<Self>, DbErr> {
        let existing = Self::find_by_id(db, id).await?;

        match existing {
            Some(execution) => {
                let mut active_model: ActiveModel = execution.into();
                active_model.status = ActiveValue::Set(status.to_string());
                // Only overwrite started_at when the caller provides an explicit value.
                // Passing None (e.g. from zombie recovery / enqueue failure) means
                // "don't touch the existing timestamp" — use NotSet to preserve it.
                if let Some(ts) = started_at {
                    active_model.started_at = ActiveValue::Set(Some(ts));
                }
                active_model.finished_at = ActiveValue::Set(finished_at);
                active_model.duration_ms = ActiveValue::Set(duration_ms);
                active_model.output = ActiveValue::Set(output);
                active_model.error_message = ActiveValue::Set(error_message);
                if let Some(tp) = traceparent {
                    active_model.traceparent = ActiveValue::Set(Some(tp));
                }
                active_model.update(db).await.map(Some)
            }
            None => Ok(None),
        }
    }

    pub async fn find_by_tenant(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        page: u64,
        page_size: u64,
    ) -> Result<(Vec<Self>, u64), DbErr> {
        let paginator = Entity::find()
            .filter(scheduled_worker_executions::Column::TenantId.eq(tenant_id))
            .order_by_desc(scheduled_worker_executions::Column::CreatedAt)
            .paginate(db, page_size);

        let total = paginator.num_items().await?;
        let rows = paginator.fetch_page(page.saturating_sub(1)).await?;

        Ok((rows, total))
    }

    pub async fn find_by_id(
        db: &DatabaseConnection,
        id: Uuid,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_executions::Column::Id.eq(id))
            .one(db)
            .await
    }

    pub async fn set_retry_count(
        db: &DatabaseConnection,
        id: Uuid,
        retry_count: i32,
    ) -> Result<Option<Self>, DbErr> {
        let existing = Self::find_by_id(db, id).await?;

        match existing {
            Some(execution) => {
                let mut active_model: ActiveModel = execution.into();
                active_model.retry_count = ActiveValue::Set(retry_count);
                active_model.update(db).await.map(Some)
            }
            None => Ok(None),
        }
    }
}
