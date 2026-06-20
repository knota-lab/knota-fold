use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait,
    QueryFilter, QueryOrder,
};
use serde_json::Value;
use uuid::Uuid;

use crate::models::_entities::worker_runs;
use crate::utils::error::IntoAppError;
use crate::views::errors::err_internal;

pub const STATUS_RUNNING: &str = "running";
pub const STATUS_SUCCEEDED: &str = "succeeded";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_CANCELLED: &str = "cancelled";
pub const KNOWLEDGE_BASE_INDEXING_BUSINESS_TYPE: &str = "knowledge_base_document";
pub const KNOWLEDGE_BASE_INDEXING_WORKER_NAME: &str = "IndexingWorker";
pub const KNOWLEDGE_BASE_INDEXING_RUN_DEFINITION: WorkerRunDefinition =
    WorkerRunDefinition {
        stages: &[
            WorkerRunStageDefinition {
                code: "queued",
                label: "等待入库",
                stale_after: Duration::from_mins(2),
                hard_timeout: None,
            },
            WorkerRunStageDefinition {
                code: "load_file",
                label: "读取文件",
                stale_after: Duration::from_mins(2),
                hard_timeout: Some(Duration::from_mins(10)),
            },
            WorkerRunStageDefinition {
                code: "parse",
                label: "文档解析",
                stale_after: Duration::from_mins(30),
                hard_timeout: Some(Duration::from_mins(45)),
            },
            WorkerRunStageDefinition {
                code: "assets",
                label: "资源处理",
                stale_after: Duration::from_mins(5),
                hard_timeout: Some(Duration::from_mins(15)),
            },
            WorkerRunStageDefinition {
                code: "save_parsed",
                label: "保存解析结果",
                stale_after: Duration::from_mins(2),
                hard_timeout: Some(Duration::from_mins(10)),
            },
            WorkerRunStageDefinition {
                code: "chunk",
                label: "文档分块",
                stale_after: Duration::from_mins(2),
                hard_timeout: Some(Duration::from_mins(10)),
            },
            WorkerRunStageDefinition {
                code: "lines",
                label: "行号落库",
                stale_after: Duration::from_mins(2),
                hard_timeout: Some(Duration::from_mins(10)),
            },
            WorkerRunStageDefinition {
                code: "embedding",
                label: "向量生成",
                stale_after: Duration::from_mins(5),
                hard_timeout: None,
            },
            WorkerRunStageDefinition {
                code: "persist",
                label: "索引写入",
                stale_after: Duration::from_mins(5),
                hard_timeout: Some(Duration::from_mins(15)),
            },
            WorkerRunStageDefinition {
                code: "mark_ready",
                label: "完成入库",
                stale_after: Duration::from_mins(2),
                hard_timeout: Some(Duration::from_mins(5)),
            },
        ],
    };

#[derive(Clone, Copy)]
pub struct WorkerRunDefinition {
    pub stages: &'static [WorkerRunStageDefinition],
}

impl WorkerRunDefinition {
    #[must_use]
    pub fn stage(&self, code: &str) -> Option<&'static WorkerRunStageDefinition> {
        self.stages.iter().find(|stage| stage.code == code)
    }
}

#[derive(Clone, Copy)]
pub struct WorkerRunStageDefinition {
    pub code: &'static str,
    pub label: &'static str,
    pub stale_after: Duration,
    pub hard_timeout: Option<Duration>,
}

pub struct WorkerRunStart {
    pub tenant_id: Option<Uuid>,
    pub worker_name: &'static str,
    pub business_type: &'static str,
    pub business_id: String,
    pub definition: WorkerRunDefinition,
    pub trace_id: Option<String>,
    pub metadata: Option<Value>,
}

#[derive(Clone)]
pub struct WorkerRunTracker {
    db: DatabaseConnection,
    run_id: Uuid,
    definition: WorkerRunDefinition,
}

impl WorkerRunTracker {
    /// Start a new tracked worker run.
    ///
    /// # Errors
    ///
    /// Returns an error if the initial run row cannot be inserted.
    pub async fn start(
        db: &DatabaseConnection,
        input: WorkerRunStart,
    ) -> loco_rs::Result<Self> {
        let now = Utc::now().naive_utc();
        let run_id = Uuid::now_v7();
        let attempt =
            next_attempt(db, input.tenant_id, input.business_type, &input.business_id)
                .await?;
        let initial_stage = input.definition.stage("queued");
        let model = worker_runs::ActiveModel {
            id: ActiveValue::Set(run_id),
            tenant_id: ActiveValue::Set(input.tenant_id),
            worker_name: ActiveValue::Set(input.worker_name.to_string()),
            business_type: ActiveValue::Set(input.business_type.to_string()),
            business_id: ActiveValue::Set(input.business_id),
            status: ActiveValue::Set(STATUS_RUNNING.to_string()),
            stage: ActiveValue::Set(initial_stage.map(|stage| stage.code.to_string())),
            stage_label: ActiveValue::Set(
                initial_stage.map(|stage| stage.label.to_string()),
            ),
            attempt: ActiveValue::Set(attempt),
            heartbeat_at: ActiveValue::Set(Some(now)),
            stage_started_at: ActiveValue::Set(Some(now)),
            started_at: ActiveValue::Set(Some(now)),
            trace_id: ActiveValue::Set(input.trace_id),
            metadata: ActiveValue::Set(input.metadata),
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
            ..Default::default()
        };
        model.insert(db).await.db_err()?;
        Ok(Self {
            db: db.clone(),
            run_id,
            definition: input.definition,
        })
    }

    #[must_use]
    pub const fn run_id(&self) -> Uuid {
        self.run_id
    }

    /// Update the current stage and heartbeat.
    ///
    /// # Errors
    ///
    /// Returns an error if the stage is not declared or the row cannot be updated.
    pub async fn stage(
        &self,
        stage_code: &str,
        message: Option<&str>,
    ) -> loco_rs::Result<()> {
        let stage = self.definition.stage(stage_code).ok_or_else(|| {
            err_internal(
                "worker_run.stage_not_defined",
                format!("worker run stage '{stage_code}' is not defined"),
            )
        })?;
        let now = Utc::now().naive_utc();
        let model = worker_runs::ActiveModel {
            id: ActiveValue::Unchanged(self.run_id),
            status: ActiveValue::Set(STATUS_RUNNING.to_string()),
            stage: ActiveValue::Set(Some(stage.code.to_string())),
            stage_label: ActiveValue::Set(Some(stage.label.to_string())),
            message: ActiveValue::Set(message.map(str::to_string)),
            current: ActiveValue::Set(None),
            total: ActiveValue::Set(None),
            heartbeat_at: ActiveValue::Set(Some(now)),
            stage_started_at: ActiveValue::Set(Some(now)),
            updated_at: ActiveValue::Set(now),
            ..Default::default()
        };
        model.update(&self.db).await.db_err()?;
        Ok(())
    }

    /// Update progress and heartbeat for a declared stage.
    ///
    /// # Errors
    ///
    /// Returns an error if the stage is not declared or the row cannot be updated.
    pub async fn progress(
        &self,
        stage_code: &str,
        current: i32,
        total: i32,
        message: Option<&str>,
    ) -> loco_rs::Result<()> {
        let stage = self.definition.stage(stage_code).ok_or_else(|| {
            err_internal(
                "worker_run.stage_not_defined",
                format!("worker run stage '{stage_code}' is not defined"),
            )
        })?;
        let now = Utc::now().naive_utc();
        let model = worker_runs::ActiveModel {
            id: ActiveValue::Unchanged(self.run_id),
            status: ActiveValue::Set(STATUS_RUNNING.to_string()),
            stage: ActiveValue::Set(Some(stage.code.to_string())),
            stage_label: ActiveValue::Set(Some(stage.label.to_string())),
            message: ActiveValue::Set(message.map(str::to_string)),
            current: ActiveValue::Set(Some(current)),
            total: ActiveValue::Set(Some(total)),
            heartbeat_at: ActiveValue::Set(Some(now)),
            updated_at: ActiveValue::Set(now),
            ..Default::default()
        };
        model.update(&self.db).await.db_err()?;
        Ok(())
    }

    /// Mark the run as succeeded.
    ///
    /// # Errors
    ///
    /// Returns an error if the row cannot be updated.
    pub async fn succeed(&self) -> loco_rs::Result<()> {
        self.finish(STATUS_SUCCEEDED, None).await
    }

    /// Mark the run as failed.
    ///
    /// # Errors
    ///
    /// Returns an error if the row cannot be updated.
    pub async fn fail(&self, error_message: &str) -> loco_rs::Result<()> {
        self.finish(STATUS_FAILED, Some(error_message)).await
    }

    /// Mark the run as cancelled.
    ///
    /// # Errors
    ///
    /// Returns an error if the row cannot be updated.
    pub async fn cancel(&self, reason: &str) -> loco_rs::Result<()> {
        self.finish(STATUS_CANCELLED, Some(reason)).await
    }

    async fn finish(
        &self,
        status: &str,
        error_message: Option<&str>,
    ) -> loco_rs::Result<()> {
        let now = Utc::now().naive_utc();
        let existing = worker_runs::Entity::find_by_id(self.run_id)
            .one(&self.db)
            .await
            .db_err()?
            .ok_or_else(|| {
                err_internal("worker_run.not_found", "worker run not found")
            })?;
        let duration_ms = existing
            .started_at
            .map(|started_at| (now - started_at).num_milliseconds());
        let model = worker_runs::ActiveModel {
            id: ActiveValue::Unchanged(self.run_id),
            status: ActiveValue::Set(status.to_string()),
            heartbeat_at: ActiveValue::Set(Some(now)),
            finished_at: ActiveValue::Set(Some(now)),
            duration_ms: ActiveValue::Set(duration_ms),
            error_message: ActiveValue::Set(error_message.map(str::to_string)),
            updated_at: ActiveValue::Set(now),
            ..Default::default()
        };
        model.update(&self.db).await.db_err()?;
        Ok(())
    }
}

/// Find the latest run for each business id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn latest_runs_by_business_ids(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    business_type: &str,
    business_ids: &[String],
) -> loco_rs::Result<HashMap<String, worker_runs::Model>> {
    if business_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: HashSet<&str> = business_ids.iter().map(String::as_str).collect();
    let mut query = worker_runs::Entity::find()
        .filter(worker_runs::Column::BusinessType.eq(business_type))
        .filter(worker_runs::Column::BusinessId.is_in(ids))
        .order_by_desc(worker_runs::Column::CreatedAt);
    query = match tenant_id {
        Some(id) => query.filter(worker_runs::Column::TenantId.eq(id)),
        None => query.filter(worker_runs::Column::TenantId.is_null()),
    };
    let runs = query.all(db).await.db_err()?;
    let mut latest = HashMap::new();
    for run in runs {
        latest.entry(run.business_id.clone()).or_insert(run);
    }
    Ok(latest)
}

/// Find the latest run for one business id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn latest_run_by_business_id(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    business_type: &str,
    business_id: &str,
) -> loco_rs::Result<Option<worker_runs::Model>> {
    let mut query = worker_runs::Entity::find()
        .filter(worker_runs::Column::BusinessType.eq(business_type))
        .filter(worker_runs::Column::BusinessId.eq(business_id))
        .order_by_desc(worker_runs::Column::CreatedAt);
    query = match tenant_id {
        Some(id) => query.filter(worker_runs::Column::TenantId.eq(id)),
        None => query.filter(worker_runs::Column::TenantId.is_null()),
    };
    query.one(db).await.db_err()
}

async fn next_attempt(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    business_type: &str,
    business_id: &str,
) -> loco_rs::Result<i32> {
    let latest =
        latest_run_by_business_id(db, tenant_id, business_type, business_id).await?;
    Ok(latest.map_or(1, |run| run.attempt.saturating_add(1)))
}
