use crate::utils::error::IntoModelResult;
use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use sea_orm::{ActiveModelTrait, ActiveValue, TransactionTrait};
use uuid::Uuid;

use crate::extractors::TenantContext;
use crate::models::{
    scheduled_worker_definitions, scheduled_worker_schedules,
    scheduled_worker_tenant_grants, tenants,
};
use crate::views::worker_scheduler::*;

#[utoipa::path(
    get,
    path = "/api/worker-definitions",
    tag = "任务调度",
    description = "查询 Worker 定义列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let db = &ctx.db;

    if tc.is_super_admin {
        let defs = scheduled_worker_definitions::Model::find_all_active(db).await?;
        let responses: Vec<WorkerDefinitionResponse> = defs
            .iter()
            .map(WorkerDefinitionResponse::from_model)
            .collect();
        return format::json(responses);
    }

    let granted_ids =
        scheduled_worker_tenant_grants::Model::find_granted_worker_ids_for_tenant(
            db,
            tc.tenant_id,
        )
        .await?;
    let all_active = scheduled_worker_definitions::Model::find_all_active(db).await?;
    let filtered: Vec<WorkerDefinitionResponse> = all_active
        .iter()
        .filter(|d| granted_ids.contains(&d.id))
        .map(WorkerDefinitionResponse::from_model)
        .collect();
    format::json(filtered)
}

#[utoipa::path(
    get,
    path = "/api/worker-definitions/{code}",
    tag = "任务调度",
    description = "查询 Worker 定义详情",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_detail(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(code): Path<String>,
) -> Result<Response> {
    let db = &ctx.db;
    let Some(def) = scheduled_worker_definitions::Model::find_by_code(db, &code).await?
    else {
        return crate::views::errors::not_found(
            "worker_def.not_found",
            "Worker 定义未找到",
        );
    };

    if !tc.is_super_admin {
        let granted =
            scheduled_worker_tenant_grants::Model::find_granted(db, def.id, tc.tenant_id)
                .await?;
        if granted.is_none() {
            return crate::views::errors::worker::not_authorized();
        }
    }

    format::json(WorkerDefinitionResponse::from_model(&def))
}

#[utoipa::path(
    post,
    path = "/api/worker-definitions",
    tag = "任务调度",
    description = "创建 Worker 定义",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn create(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateWorkerDefinitionRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let db = &ctx.db;
    let existing =
        scheduled_worker_definitions::Model::find_by_code(db, &params.code).await?;
    if existing.is_some() {
        return crate::views::errors::conflict(
            "worker_def.already_exists",
            "Worker 编码已存在",
        );
    }

    let active_model = scheduled_worker_definitions::ActiveModel {
        code: ActiveValue::Set(params.code),
        name: ActiveValue::Set(params.name),
        description: ActiveValue::Set(params.description),
        category: ActiveValue::Set(params.category),
        params_schema: ActiveValue::Set(params.params_schema),
        timeout_secs: ActiveValue::Set(params.timeout_secs.unwrap_or(300)),
        max_retries: ActiveValue::Set(params.max_retries.unwrap_or(3)),
        allow_concurrent: ActiveValue::Set(params.allow_concurrent.unwrap_or(false)),
        is_system: ActiveValue::Set(false),
        status: ActiveValue::Set("active".to_string()),
        version: ActiveValue::Set(1),
        updated_by: ActiveValue::Set(None),
        ..Default::default()
    };

    let result = active_model.insert(db).await.model_err()?;
    format::json(WorkerDefinitionResponse::from_model(&result))
}

#[utoipa::path(
    put,
    path = "/api/worker-definitions/{code}",
    tag = "任务调度",
    description = "更新 Worker 定义",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn update(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(code): Path<String>,
    Json(params): Json<UpdateWorkerDefinitionRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let db = &ctx.db;
    let Some(def) = scheduled_worker_definitions::Model::find_by_code(db, &code).await?
    else {
        return crate::views::errors::not_found(
            "worker_def.not_found",
            "Worker 定义未找到",
        );
    };

    let mut active_model: scheduled_worker_definitions::ActiveModel = def.into();

    if let Some(name) = params.name {
        active_model.name = ActiveValue::Set(name);
    }
    if let Some(description) = params.description {
        active_model.description = ActiveValue::Set(description);
    }
    if let Some(category) = params.category {
        active_model.category = ActiveValue::Set(category);
    }
    if let Some(params_schema) = params.params_schema {
        active_model.params_schema = ActiveValue::Set(params_schema);
    }
    if let Some(timeout_secs) = params.timeout_secs {
        active_model.timeout_secs = ActiveValue::Set(timeout_secs);
    }
    if let Some(max_retries) = params.max_retries {
        active_model.max_retries = ActiveValue::Set(max_retries);
    }
    if let Some(allow_concurrent) = params.allow_concurrent {
        active_model.allow_concurrent = ActiveValue::Set(allow_concurrent);
    }

    active_model.updated_by = ActiveValue::Set(Some(tc.user_id));
    let result = active_model.update(db).await.model_err()?;

    format::json(WorkerDefinitionResponse::from_model(&result))
}

#[utoipa::path(
    patch,
    path = "/api/worker-definitions/{code}/status",
    tag = "任务调度",
    description = "启用或禁用 Worker 定义",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn patch_status(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(code): Path<String>,
    Json(params): Json<PatchStatusRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let db = &ctx.db;
    let Some(def) = scheduled_worker_definitions::Model::find_by_code(db, &code).await?
    else {
        return crate::views::errors::not_found(
            "worker_def.not_found",
            "Worker 定义未找到",
        );
    };

    if params.status != "active" && params.status != "disabled" {
        return crate::views::errors::bad_request(
            "worker_def.invalid_status",
            "状态必须是 active 或 disabled",
        );
    }

    let mut active_model: scheduled_worker_definitions::ActiveModel = def.into();
    active_model.status = ActiveValue::Set(params.status);
    active_model.updated_by = ActiveValue::Set(Some(tc.user_id));
    let result = active_model.update(db).await.model_err()?;

    format::json(WorkerDefinitionResponse::from_model(&result))
}

#[utoipa::path(
    get,
    path = "/api/worker-definitions/{code}/grants",
    tag = "任务调度",
    description = "查询 Worker 授权列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_grants(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(code): Path<String>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let db = &ctx.db;
    let Some(def) = scheduled_worker_definitions::Model::find_by_code(db, &code).await?
    else {
        return crate::views::errors::not_found(
            "worker_def.not_found",
            "Worker 定义未找到",
        );
    };

    let grants =
        scheduled_worker_tenant_grants::Model::find_grants_for_worker(db, def.id).await?;
    let responses: Vec<WorkerGrantResponse> =
        grants.iter().map(WorkerGrantResponse::from_model).collect();
    format::json(responses)
}

#[utoipa::path(
    put,
    path = "/api/worker-definitions/{code}/grants",
    tag = "任务调度",
    description = "批量替换 Worker 授权",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn batch_set_grants(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(code): Path<String>,
    Json(params): Json<BatchGrantsRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let db = &ctx.db;
    let Some(def) = scheduled_worker_definitions::Model::find_by_code(db, &code).await?
    else {
        return crate::views::errors::not_found(
            "worker_def.not_found",
            "Worker 定义未找到",
        );
    };

    let tenant_ids: Vec<Uuid> = params
        .tenant_ids
        .iter()
        .filter_map(|s| s.parse::<Uuid>().ok())
        .collect();

    // Wrap the entire grant replacement in a transaction to prevent
    // data loss if the process crashes between delete and re-insert.
    let txn = db.begin().await.model_err()?;

    let existing_grants =
        scheduled_worker_tenant_grants::Model::find_grants_for_worker(db, def.id).await?;
    let existing_tenant_ids: Vec<Uuid> =
        existing_grants.iter().map(|g| g.tenant_id).collect();
    let removed_tenant_ids: Vec<&Uuid> = existing_tenant_ids
        .iter()
        .filter(|id| !tenant_ids.contains(id))
        .collect();

    for &tenant_id in &removed_tenant_ids {
        scheduled_worker_schedules::Model::disable_for_worker_and_tenant(
            db, def.id, *tenant_id,
        )
        .await?;
    }

    scheduled_worker_tenant_grants::Model::delete_for_worker(db, def.id).await?;

    for tenant_id in &tenant_ids {
        if tenants::Model::find_by_id(db, *tenant_id).await.is_err() {
            continue;
        }

        let active_model = scheduled_worker_tenant_grants::ActiveModel {
            worker_def_id: ActiveValue::Set(def.id),
            tenant_id: ActiveValue::Set(*tenant_id),
            granted_by: ActiveValue::Set(Some(tc.user_id)),
            ..Default::default()
        };
        active_model.insert(db).await.model_err()?;
    }

    txn.commit().await.model_err()?;

    let grants =
        scheduled_worker_tenant_grants::Model::find_grants_for_worker(db, def.id).await?;
    let responses: Vec<WorkerGrantResponse> =
        grants.iter().map(WorkerGrantResponse::from_model).collect();
    format::json(responses)
}

#[utoipa::path(
    get,
    path = "/api/worker-definitions/{code}/grants/tenants",
    tag = "任务调度",
    description = "查询已授权租户",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_granted_tenants(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(code): Path<String>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let db = &ctx.db;
    let Some(def) = scheduled_worker_definitions::Model::find_by_code(db, &code).await?
    else {
        return crate::views::errors::not_found(
            "worker_def.not_found",
            "Worker 定义未找到",
        );
    };

    let grants =
        scheduled_worker_tenant_grants::Model::find_grants_for_worker(db, def.id).await?;
    let mut result = Vec::new();
    for grant in &grants {
        if let Ok(tenant) = tenants::Model::find_by_id(db, grant.tenant_id).await {
            result.push(GrantedTenantResponse {
                id: tenant.id.to_string(),
                name: tenant.name.clone(),
                code: tenant.code.clone(),
            });
        }
    }
    format::json(result)
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/worker-definitions")
        .add("/", openapi(get(list), routes!(list)))
        .add("/{code}", openapi(get(get_detail), routes!(get_detail)))
        .add("/", openapi(post(create), routes!(create)))
        .add("/{code}", openapi(put(update), routes!(update)))
        .add(
            "/{code}/status",
            openapi(patch(patch_status), routes!(patch_status)),
        )
        .add(
            "/{code}/grants",
            openapi(get(list_grants), routes!(list_grants)),
        )
        .add(
            "/{code}/grants",
            openapi(put(batch_set_grants), routes!(batch_set_grants)),
        )
        .add(
            "/{code}/grants/tenants",
            openapi(get(list_granted_tenants), routes!(list_granted_tenants)),
        )
}
