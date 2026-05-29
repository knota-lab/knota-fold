use crate::utils::error::IntoModelResult;
use loco_openapi::prelude::*;
use loco_rs::{bgworker::BackgroundWorker, prelude::*};
use sea_orm::{ActiveModelTrait, ActiveValue, EntityTrait};
use uuid::Uuid;

use crate::extractors::TenantContext;
use crate::middleware::tracing::TraceId;
use crate::models::{
    scheduled_worker_definitions, scheduled_worker_executions,
    scheduled_worker_schedules, scheduled_worker_tenant_grants, tenants,
};
use crate::services::task_scheduler_service;
use crate::views::errors::parse_uuid;
use crate::views::worker_scheduler::*;

#[utoipa::path(
    get,
    path = "/api/worker-schedules",
    tag = "任务调度",
    description = "查询当前租户 Worker 调度计划列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let db = &ctx.db;
    let schedules =
        scheduled_worker_schedules::Model::find_by_tenant(db, tc.tenant_id).await?;

    let mut responses = Vec::new();
    for schedule in &schedules {
        let mut resp = WorkerScheduleResponse::from_model(schedule);
        if let Ok(Some(worker_def)) =
            scheduled_worker_definitions::Entity::find_by_id(schedule.worker_def_id)
                .one(db)
                .await
        {
            resp = resp.with_worker_info(&worker_def.name, &worker_def.code);
        }
        responses.push(resp);
    }
    format::json(responses)
}

#[utoipa::path(
    post,
    path = "/api/worker-schedules",
    tag = "任务调度",
    description = "创建 Worker 调度计划",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn create(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateWorkerScheduleRequest>,
) -> Result<Response> {
    let db = &ctx.db;
    let worker_def_id = parse_uuid(params.worker_def_id)?;

    let grant = scheduled_worker_tenant_grants::Model::find_granted(
        db,
        worker_def_id,
        tc.tenant_id,
    )
    .await?;
    if grant.is_none() {
        return crate::views::errors::worker::not_authorized();
    }

    let Some(worker_def) =
        scheduled_worker_definitions::Entity::find_by_id(worker_def_id)
            .one(db)
            .await?
    else {
        return crate::views::errors::not_found(
            "worker_def.not_found",
            "Worker 定义未找到",
        );
    };
    if worker_def.status != "active" {
        return crate::views::errors::bad_request("worker.not_active", "Worker 未激活");
    }

    let next_run =
        task_scheduler_service::compute_next_run(&params.cron_expr, chrono::Utc::now())?;

    let active_model = scheduled_worker_schedules::ActiveModel {
        worker_def_id: ActiveValue::Set(worker_def_id),
        tenant_id: ActiveValue::Set(tc.tenant_id),
        name: ActiveValue::Set(params.name),
        cron_expr: ActiveValue::Set(params.cron_expr),
        params_json: ActiveValue::Set(params.params_json),
        enabled: ActiveValue::Set(true),
        next_run_at: ActiveValue::Set(Some(next_run.fixed_offset())),
        created_by: ActiveValue::Set(Some(tc.user_id)),
        ..Default::default()
    };

    let result = active_model.insert(db).await.model_err()?;

    let resp = WorkerScheduleResponse::from_model(&result)
        .with_worker_info(&worker_def.name, &worker_def.code);
    format::json(resp)
}

#[utoipa::path(
    put,
    path = "/api/worker-schedules/{id}",
    tag = "任务调度",
    description = "更新 Worker 调度计划",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn update(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<UpdateWorkerScheduleRequest>,
) -> Result<Response> {
    let db = &ctx.db;
    let schedule_id = parse_uuid(id)?;

    let Some(schedule) =
        scheduled_worker_schedules::Model::find_by_id(db, schedule_id).await?
    else {
        return crate::views::errors::not_found(
            "worker_schedule.not_found",
            "定时任务未找到",
        );
    };

    if schedule.tenant_id != tc.tenant_id {
        return crate::views::errors::worker::schedule_not_yours();
    }

    let mut active_model: scheduled_worker_schedules::ActiveModel =
        schedule.clone().into();

    if let Some(name) = params.name {
        active_model.name = ActiveValue::Set(name);
    }
    if let Some(new_cron_expr) = params.cron_expr {
        let next_run =
            task_scheduler_service::compute_next_run(&new_cron_expr, chrono::Utc::now())?;
        active_model.cron_expr = ActiveValue::Set(new_cron_expr);
        active_model.next_run_at = ActiveValue::Set(Some(next_run.fixed_offset()));
    }
    if let Some(params_json) = params.params_json {
        active_model.params_json = ActiveValue::Set(params_json);
    }
    active_model.updated_by = ActiveValue::Set(Some(tc.user_id));

    let result = active_model.update(db).await.model_err()?;
    let mut resp = WorkerScheduleResponse::from_model(&result);
    if let Ok(Some(worker_def)) =
        scheduled_worker_definitions::Entity::find_by_id(result.worker_def_id)
            .one(db)
            .await
    {
        resp = resp.with_worker_info(&worker_def.name, &worker_def.code);
    }
    format::json(resp)
}

#[utoipa::path(
    patch,
    path = "/api/worker-schedules/{id}/status",
    tag = "任务调度",
    description = "启用或禁用 Worker 调度计划",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn patch_status(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<PatchEnabledRequest>,
) -> Result<Response> {
    let db = &ctx.db;
    let schedule_id = parse_uuid(id)?;

    let Some(schedule) =
        scheduled_worker_schedules::Model::find_by_id(db, schedule_id).await?
    else {
        return crate::views::errors::not_found(
            "worker_schedule.not_found",
            "定时任务未找到",
        );
    };

    if schedule.tenant_id != tc.tenant_id {
        return crate::views::errors::worker::schedule_not_yours();
    }

    let mut active_model: scheduled_worker_schedules::ActiveModel =
        schedule.clone().into();
    active_model.enabled = ActiveValue::Set(params.enabled);
    active_model.updated_by = ActiveValue::Set(Some(tc.user_id));

    if params.enabled {
        let next_run = task_scheduler_service::compute_next_run(
            &schedule.cron_expr,
            chrono::Utc::now(),
        )?;
        active_model.next_run_at = ActiveValue::Set(Some(next_run.fixed_offset()));
    }

    let result = active_model.update(db).await.model_err()?;
    let mut resp = WorkerScheduleResponse::from_model(&result);
    if let Ok(Some(worker_def)) =
        scheduled_worker_definitions::Entity::find_by_id(result.worker_def_id)
            .one(db)
            .await
    {
        resp = resp.with_worker_info(&worker_def.name, &worker_def.code);
    }
    format::json(resp)
}

#[utoipa::path(
    delete,
    path = "/api/worker-schedules/{id}",
    tag = "任务调度",
    description = "删除 Worker 调度计划",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn delete_schedule(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let db = &ctx.db;
    let schedule_id = parse_uuid(id)?;

    let Some(schedule) =
        scheduled_worker_schedules::Model::find_by_id(db, schedule_id).await?
    else {
        return crate::views::errors::not_found(
            "worker_schedule.not_found",
            "定时任务未找到",
        );
    };

    if schedule.tenant_id != tc.tenant_id {
        return crate::views::errors::worker::schedule_not_yours();
    }

    scheduled_worker_schedules::Entity::delete_by_id(schedule_id)
        .exec(db)
        .await?;
    format::json(())
}

#[utoipa::path(
    post,
    path = "/api/worker-schedules/{id}/trigger",
    tag = "任务调度",
    description = "立即执行一次 Worker 调度计划",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn trigger(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Response> {
    let db = &ctx.db;
    let schedule_id = parse_uuid(id)?;

    let Some(schedule) =
        scheduled_worker_schedules::Model::find_by_id(db, schedule_id).await?
    else {
        return crate::views::errors::not_found(
            "worker_schedule.not_found",
            "定时任务未找到",
        );
    };

    if schedule.tenant_id != tc.tenant_id {
        return crate::views::errors::worker::schedule_not_yours();
    }

    let worker_code = get_worker_code(db, schedule.worker_def_id).await?;
    let Some(worker_def) =
        scheduled_worker_definitions::Model::find_active_by_code(db, &worker_code)
            .await?
    else {
        return crate::views::errors::not_found(
            "worker.not_found_or_inactive",
            "Worker 未找到或未激活",
        );
    };

    let grant = scheduled_worker_tenant_grants::Model::find_granted(
        db,
        worker_def.id,
        tc.tenant_id,
    )
    .await?;
    if grant.is_none() {
        return crate::views::errors::worker::not_authorized();
    }

    let Ok(tenant) = tenants::Model::find_by_id(db, tc.tenant_id).await else {
        return crate::views::errors::not_found("common.tenant_not_found", "租户未找到");
    };
    if tenant.status != "active" {
        return crate::views::errors::forbidden("common.tenant_inactive", "租户已停用");
    }

    if !worker_def.allow_concurrent {
        let running = scheduled_worker_executions::Model::find_running_for_schedule(
            db,
            schedule_id,
        )
        .await?;
        if !running.is_empty() {
            return crate::views::errors::conflict(
                "worker_schedule.concurrent_execution",
                "已有正在执行的并发任务",
            );
        }
    }

    let trace_id = req.extensions().get::<TraceId>().map(|t| t.0.clone());

    // Extract the current span ID so the worker can set it as parent.
    // The controller runs inside TracingLayer's "http.request" span.
    let parent_span_id: Option<String> =
        tracing::Span::current().with_subscriber(|(id, _)| id.into_u64().to_string());

    let execution = scheduled_worker_executions::Model::create_pending(
        db,
        &scheduled_worker_executions::CreateExecutionParams {
            schedule_id: schedule.id,
            worker_def_id: worker_def.id,
            tenant_id: tc.tenant_id,
            trigger_type: "manual".to_string(),
            params_json: schedule.params_json.clone(),
            triggered_by: Some(tc.user_id),
            traceparent: trace_id.clone(),
            parent_span_id: parent_span_id.clone(),
        },
    )
    .await?;

    let result = if worker_code.as_str() == "test_job" {
        use crate::workers::test_job_worker::{TestJobWorker, TestJobWorkerArgs};

        TestJobWorker::perform_later(
            &ctx,
            TestJobWorkerArgs {
                execution_id: execution.id,
                worker_code: worker_def.code.clone(),
                tenant_id: tc.tenant_id,
                params_json: schedule.params_json.clone(),
                retry_count: 0,
                trace_id,
                parent_span_id,
            },
        )
        .await
    } else {
        tracing::error!(worker_code = %worker_code, "unknown worker code");
        scheduled_worker_executions::Model::update_status(
            db,
            execution.id,
            &scheduled_worker_executions::UpdateStatusParams {
                status: "skipped".to_string(),
                started_at: None,
                finished_at: None,
                duration_ms: None,
                output: None,
                error_message: Some(format!("unknown worker code: {worker_code}")),
                traceparent: None,
            },
        )
        .await?;
        return crate::views::errors::bad_request(
            "worker_schedule.create_failed",
            format!("unknown worker code: {worker_code}"),
        );
    };

    if let Err(e) = result {
        return crate::views::errors::bad_request(
            "worker_schedule.trigger_failed",
            e.to_string(),
        );
    }

    format::json(TriggerResponse {
        execution_id: execution.id.to_string(),
    })
}

async fn get_worker_code(
    db: &sea_orm::DatabaseConnection,
    worker_def_id: Uuid,
) -> Result<String> {
    let Some(worker_def) =
        scheduled_worker_definitions::Entity::find_by_id(worker_def_id)
            .one(db)
            .await?
    else {
        return Err(loco_rs::Error::CustomError(
            axum::http::StatusCode::NOT_FOUND,
            loco_rs::controller::ErrorDetail::new(
                "worker_def.not_found",
                "Worker 定义未找到",
            ),
        ));
    };
    Ok(worker_def.code)
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/worker-schedules")
        .add("/", openapi(get(list), routes!(list)))
        .add("/", openapi(post(create), routes!(create)))
        .add("/{id}", openapi(put(update), routes!(update)))
        .add(
            "/{id}/status",
            openapi(patch(patch_status), routes!(patch_status)),
        )
        .add(
            "/{id}",
            openapi(delete(delete_schedule), routes!(delete_schedule)),
        )
        .add("/{id}/trigger", openapi(post(trigger), routes!(trigger)))
}
