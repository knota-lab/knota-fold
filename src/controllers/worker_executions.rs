use loco_openapi::prelude::*;
use loco_rs::prelude::model::query;
use loco_rs::prelude::*;
use sea_orm::EntityTrait;

use crate::extractors::TenantContext;
use crate::models::{
    scheduled_worker_definitions, scheduled_worker_executions, scheduled_worker_schedules,
};
use crate::views::errors::parse_uuid;
use crate::views::pagination::PaginatedResponse;
use crate::views::worker_scheduler::*;

#[utoipa::path(
    get,
    path = "/api/worker-executions",
    tag = "任务调度",
    description = "分页查询当前租户 Worker 执行日志",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(pagination): Query<query::PaginationQuery>,
) -> Result<Response> {
    let db = &ctx.db;
    let (rows, total) = scheduled_worker_executions::Model::find_by_tenant(
        db,
        tc.tenant_id,
        pagination.page,
        pagination.page_size,
    )
    .await?;

    let mut responses = Vec::new();
    for execution in &rows {
        let mut resp = WorkerExecutionResponse::from_model(execution);
        if let Ok(Some(worker_def)) =
            scheduled_worker_definitions::Entity::find_by_id(execution.worker_def_id)
                .one(db)
                .await
        {
            resp = resp.with_worker_info(&worker_def.name, &worker_def.code);
        }
        if let Ok(Some(schedule)) =
            scheduled_worker_schedules::Model::find_by_id(db, execution.schedule_id).await
        {
            resp = resp.with_schedule_name(&schedule.name);
        }
        responses.push(resp);
    }

    let total_pages = if pagination.page_size == 0 {
        0
    } else {
        total.div_ceil(pagination.page_size)
    };

    format::json(PaginatedResponse {
        items: responses,
        total_pages,
        total_items: total,
        page: pagination.page,
        page_size: pagination.page_size,
    })
}

#[utoipa::path(
    get,
    path = "/api/worker-executions/{id}",
    tag = "任务调度",
    description = "查询 Worker 执行日志详情",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_detail(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let db = &ctx.db;
    let execution_id = parse_uuid(id)?;

    let execution =
        match scheduled_worker_executions::Model::find_by_id(db, execution_id).await? {
            Some(e) => e,
            None => {
                return crate::views::errors::not_found(
                    "worker_execution.not_found",
                    "执行记录未找到",
                )
            }
        };

    if execution.tenant_id != tc.tenant_id && !tc.is_super_admin {
        return crate::views::errors::worker::execution_not_yours();
    }

    let mut resp = WorkerExecutionResponse::from_model(&execution);
    if let Ok(Some(worker_def)) =
        scheduled_worker_definitions::Entity::find_by_id(execution.worker_def_id)
            .one(db)
            .await
    {
        resp = resp.with_worker_info(&worker_def.name, &worker_def.code);
    }
    if let Ok(Some(schedule)) =
        scheduled_worker_schedules::Model::find_by_id(db, execution.schedule_id).await
    {
        resp = resp.with_schedule_name(&schedule.name);
    }

    format::json(resp)
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/worker-executions")
        .add("/", openapi(get(list), routes!(list)))
        .add("/{id}", openapi(get(get_detail), routes!(get_detail)))
}
