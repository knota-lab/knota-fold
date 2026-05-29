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

pub struct CreateExecutionParams {
    pub schedule_id: Uuid,
    pub worker_def_id: Uuid,
    pub tenant_id: Uuid,
    pub trigger_type: String,
    pub params_json: Option<String>,
    pub triggered_by: Option<Uuid>,
    pub traceparent: Option<String>,
    pub parent_span_id: Option<String>,
}

pub struct UpdateStatusParams {
    pub status: String,
    pub started_at: Option<DateTimeWithTimeZone>,
    pub finished_at: Option<DateTimeWithTimeZone>,
    pub duration_ms: Option<i32>,
    pub output: Option<String>,
    pub error_message: Option<String>,
    pub traceparent: Option<String>,
}

impl Model {
    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn create_pending(
        db: &DatabaseConnection,
        params: &CreateExecutionParams,
    ) -> Result<Self, DbErr> {
        let active_model = ActiveModel {
            schedule_id: ActiveValue::Set(params.schedule_id),
            worker_def_id: ActiveValue::Set(params.worker_def_id),
            tenant_id: ActiveValue::Set(params.tenant_id),
            trigger_type: ActiveValue::Set(params.trigger_type.clone()),
            triggered_by: ActiveValue::Set(params.triggered_by),
            params_json: ActiveValue::Set(params.params_json.clone()),
            status: ActiveValue::Set("pending".to_string()),
            retry_count: ActiveValue::Set(0),
            started_at: ActiveValue::Set(None),
            finished_at: ActiveValue::Set(None),
            duration_ms: ActiveValue::Set(None),
            output: ActiveValue::Set(None),
            error_message: ActiveValue::Set(None),
            traceparent: ActiveValue::Set(params.traceparent.clone()),
            parent_span_id: ActiveValue::Set(params.parent_span_id.clone()),
            ..Default::default()
        };

        active_model.insert(db).await
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails.
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

    /// # Errors
    ///
    /// Returns a database error if the query fails.
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

    /// # Errors
    ///
    /// Returns a database error if the query fails.
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

    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn update_status(
        db: &DatabaseConnection,
        id: Uuid,
        params: &UpdateStatusParams,
    ) -> Result<Option<Self>, DbErr> {
        let existing = Self::find_by_id(db, id).await?;

        match existing {
            Some(execution) => {
                let mut active_model: ActiveModel = execution.into();
                active_model.status = ActiveValue::Set(params.status.clone());
                if let Some(ts) = params.started_at {
                    active_model.started_at = ActiveValue::Set(Some(ts));
                }
                active_model.finished_at = ActiveValue::Set(params.finished_at);
                active_model.duration_ms = ActiveValue::Set(params.duration_ms);
                active_model.output = ActiveValue::Set(params.output.clone());
                active_model.error_message =
                    ActiveValue::Set(params.error_message.clone());
                if let Some(ref tp) = params.traceparent {
                    active_model.traceparent = ActiveValue::Set(Some(tp.clone()));
                }
                active_model.update(db).await.map(Some)
            }
            None => Ok(None),
        }
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails.
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

    /// # Errors
    ///
    /// Returns a database error if the query fails.
    pub async fn find_by_id(
        db: &DatabaseConnection,
        id: Uuid,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(scheduled_worker_executions::Column::Id.eq(id))
            .one(db)
            .await
    }

    /// # Errors
    ///
    /// Returns a database error if the query fails.
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
