use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use crate::extractors::{RequestMeta, TenantContext};
use crate::models::tenants;
use crate::services::sys_config_service;
use crate::utils::error::IntoModelResult;
use crate::views::audit_logs::AuditContext;
use crate::views::sys_configs::{
    CreateGlobalConfigRequest, GlobalConfigListParams, SysConfigListResponse,
    SysConfigResponse, TenantConfigListParams, UpdateGlobalConfigRequest,
    UpsertTenantConfigRequest,
};

// ── Global config management ──────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/api/sys-configs",
    tag = "配置中心",
    description = "查询全局配置列表（支持 category/prefix/分页过滤）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_global(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(params): Query<GlobalConfigListParams>,
) -> Result<Response> {
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(20).clamp(1, 200);

    let (items, total) = sys_config_service::list_global_configs(
        &ctx.db,
        params.category.as_deref(),
        params.prefix.as_deref(),
        page,
        page_size,
    )
    .await?;

    let total_pages = total.div_ceil(page_size);

    format::json(SysConfigListResponse {
        items: items.iter().map(SysConfigResponse::from_model).collect(),
        total_items: total,
        total_pages,
        page,
        page_size,
    })
}

#[utoipa::path(
    post,
    path = "/api/sys-configs",
    tag = "配置中心",
    description = "创建全局配置",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn create_global(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateGlobalConfigRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let model =
        sys_config_service::create_global_config(&ctx, tc.user_id, &params, &audit_ctx)
            .await?;

    format::json(SysConfigResponse::from_model(&model))
}

#[utoipa::path(
    put,
    path = "/api/sys-configs/{key}",
    tag = "配置中心",
    description = "更新全局配置",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn update_global(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(key): Path<String>,
    Json(params): Json<UpdateGlobalConfigRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let model = sys_config_service::update_global_config(
        &ctx, &key, tc.user_id, &params, &audit_ctx,
    )
    .await?;

    format::json(SysConfigResponse::from_model(&model))
}

#[utoipa::path(
    delete,
    path = "/api/sys-configs/{key}",
    tag = "配置中心",
    description = "删除全局配置（级联删除所有租户覆盖）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn delete_global(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(key): Path<String>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    sys_config_service::delete_global_config(&ctx, &key, &audit_ctx).await?;

    format::json(())
}

// ── Tenant override management (current tenant via JWT) ──────────────────────

#[utoipa::path(
    get,
    path = "/api/sys-configs/overrides",
    tag = "配置中心",
    description = "查询当前租户的所有覆盖配置（基于 JWT 隐式作用域）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_current_tenant_overrides(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(params): Query<TenantConfigListParams>,
) -> Result<Response> {
    let items = sys_config_service::list_tenant_configs(
        &ctx.db,
        tc.tenant_id,
        params.category.as_deref(),
        params.prefix.as_deref(),
    )
    .await?;

    format::json(
        items
            .iter()
            .map(SysConfigResponse::from_model)
            .collect::<Vec<_>>(),
    )
}

#[utoipa::path(
    put,
    path = "/api/sys-configs/{key}/override",
    tag = "配置中心",
    description = "Upsert 当前租户的覆盖配置（基于 JWT 隐式作用域）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn upsert_current_tenant_override(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(key): Path<String>,
    Json(params): Json<UpsertTenantConfigRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let response = sys_config_service::upsert_tenant_config(
        &ctx,
        &key,
        tc.tenant_id,
        tc.user_id,
        &params,
        &audit_ctx,
    )
    .await?;

    format::json(response)
}

#[utoipa::path(
    delete,
    path = "/api/sys-configs/{key}/override",
    tag = "配置中心",
    description = "删除当前租户的覆盖配置（基于 JWT 隐式作用域）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn delete_current_tenant_override(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(key): Path<String>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    sys_config_service::delete_tenant_config(&ctx, &key, tc.tenant_id, &audit_ctx)
        .await?;

    format::json(())
}

// ── Super-admin cross-tenant override management (by tenant_code) ────────────

/// Resolve a `tenant_code` to UUID, requiring super-admin privileges.
async fn require_super_admin_and_resolve_tenant(
    tc: &TenantContext,
    ctx: &AppContext,
    tenant_code: &str,
) -> Result<uuid::Uuid> {
    if !tc.is_super_admin {
        return Err(crate::views::errors::sys_config::err_super_admin_required());
    }
    let tenant = tenants::Model::find_by_code(&ctx.db, tenant_code)
        .await
        .model_err()?;
    Ok(tenant.id)
}

#[utoipa::path(
    get,
    path = "/api/sys/tenants/{tenant_code}/sys-configs/overrides",
    tag = "配置中心",
    description = "[超管] 查询指定租户的所有覆盖配置",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_tenant_overrides_super(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(tenant_code): Path<String>,
    Query(params): Query<TenantConfigListParams>,
) -> Result<Response> {
    let tenant_uuid =
        require_super_admin_and_resolve_tenant(&tc, &ctx, &tenant_code).await?;

    let items = sys_config_service::list_tenant_configs(
        &ctx.db,
        tenant_uuid,
        params.category.as_deref(),
        params.prefix.as_deref(),
    )
    .await?;

    format::json(
        items
            .iter()
            .map(SysConfigResponse::from_model)
            .collect::<Vec<_>>(),
    )
}

#[utoipa::path(
    put,
    path = "/api/sys/tenants/{tenant_code}/sys-configs/{key}/override",
    tag = "配置中心",
    description = "[超管] Upsert 指定租户的覆盖配置",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn upsert_tenant_override_super(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path((tenant_code, key)): Path<(String, String)>,
    Json(params): Json<UpsertTenantConfigRequest>,
) -> Result<Response> {
    let tenant_uuid =
        require_super_admin_and_resolve_tenant(&tc, &ctx, &tenant_code).await?;

    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let response = sys_config_service::upsert_tenant_config(
        &ctx,
        &key,
        tenant_uuid,
        tc.user_id,
        &params,
        &audit_ctx,
    )
    .await?;

    format::json(response)
}

#[utoipa::path(
    delete,
    path = "/api/sys/tenants/{tenant_code}/sys-configs/{key}/override",
    tag = "配置中心",
    description = "[超管] 删除指定租户的覆盖配置",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn delete_tenant_override_super(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path((tenant_code, key)): Path<(String, String)>,
) -> Result<Response> {
    let tenant_uuid =
        require_super_admin_and_resolve_tenant(&tc, &ctx, &tenant_code).await?;

    let audit_ctx = AuditContext::from_request(&tc, &meta);
    sys_config_service::delete_tenant_config(&ctx, &key, tenant_uuid, &audit_ctx).await?;

    format::json(())
}

// ── Resolved (frontend-facing) ────────────────────────────────────────────────

/// Frontend bulk-init endpoint — all authenticated users can access.
/// Returns slim resolved config map for the caller's tenant context.
#[utoipa::path(
    get,
    path = "/api/sys-configs/resolved",
    tag = "配置中心",
    description = "获取所有已解析配置（前端初始化用，登录用户均可访问）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_all_resolved(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let tenant_id = if tc.is_super_admin {
        None
    } else {
        Some(tc.tenant_id)
    };

    let response = sys_config_service::get_all_resolved(&ctx, tenant_id).await?;

    format::json(response)
}

/// Admin-only debug endpoint — returns resolved value with all layer details.
#[utoipa::path(
    get,
    path = "/api/sys-configs/resolved/{key}",
    tag = "配置中心",
    description = "获取指定配置的解析详情（含 layers，仅管理员可用）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_resolved_detail(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(key): Path<String>,
) -> Result<Response> {
    let tenant_id = if tc.is_super_admin {
        None
    } else {
        Some(tc.tenant_id)
    };

    let detail = sys_config_service::get_resolved_detail(&ctx, &key, tenant_id).await?;

    detail.map_or_else(
        || {
            Err(crate::views::errors::err_not_found(
                "sys_config.not_found",
                "配置项不存在",
            ))
        },
        format::json,
    )
}

// ── Route registration ────────────────────────────────────────────────────────

/// Routes under authz (admin-only operations + resolved detail).
pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/sys-configs")
        .add("/", openapi(get(list_global), routes!(list_global)))
        .add("/", openapi(post(create_global), routes!(create_global)))
        .add(
            "/{key}",
            openapi(put(update_global), routes!(update_global)),
        )
        .add(
            "/{key}",
            openapi(delete(delete_global), routes!(delete_global)),
        )
        .add(
            "/resolved/{key}",
            openapi(get(get_resolved_detail), routes!(get_resolved_detail)),
        )
}

/// Tenant override routes (current tenant via JWT).
pub fn tenant_routes() -> Routes {
    Routes::new()
        .prefix("/api/sys-configs")
        .add(
            "/overrides",
            openapi(
                get(list_current_tenant_overrides),
                routes!(list_current_tenant_overrides),
            ),
        )
        .add(
            "/{key}/override",
            openapi(
                put(upsert_current_tenant_override),
                routes!(upsert_current_tenant_override),
            ),
        )
        .add(
            "/{key}/override",
            openapi(
                delete(delete_current_tenant_override),
                routes!(delete_current_tenant_override),
            ),
        )
}

/// Super-admin cross-tenant override routes (target tenant via path `tenant_code`).
pub fn super_admin_routes() -> Routes {
    Routes::new()
        .prefix("/api/sys/tenants")
        .add(
            "/{tenant_code}/sys-configs/overrides",
            openapi(
                get(list_tenant_overrides_super),
                routes!(list_tenant_overrides_super),
            ),
        )
        .add(
            "/{tenant_code}/sys-configs/{key}/override",
            openapi(
                put(upsert_tenant_override_super),
                routes!(upsert_tenant_override_super),
            ),
        )
        .add(
            "/{tenant_code}/sys-configs/{key}/override",
            openapi(
                delete(delete_tenant_override_super),
                routes!(delete_tenant_override_super),
            ),
        )
}

/// Resolved bulk endpoint — requires JWT but no extra Casbin permission.
pub fn resolved_routes() -> Routes {
    Routes::new().prefix("/api/sys-configs").add(
        "/resolved",
        openapi(get(get_all_resolved), routes!(get_all_resolved)),
    )
}
