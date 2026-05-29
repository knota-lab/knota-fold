use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use crate::extractors::RequestMeta;
use crate::extractors::TenantContext;
use crate::services::tenant_menu_service;
use crate::views::audit_logs::AuditContext;
use crate::views::errors::parse_uuid;
use crate::views::menus::{MergedMenuTreeResponse, UpdateOverrideRequest};

#[utoipa::path(
    get,
    path = "/api/menus/tree",
    tag = "菜单管理",
    description = "查询租户菜单树",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn tree(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    if !tc.is_super_admin && !tc.is_tenant_admin {
        return crate::views::errors::authz::admin_required();
    }
    let response: Vec<MergedMenuTreeResponse> =
        tenant_menu_service::get_merged_menu_tree(&ctx.db, tc.tenant_id).await?;

    format::json(response)
}

#[utoipa::path(
    put,
    path = "/api/menus/{sys_menu_id}/override",
    tag = "菜单管理",
    description = "更新菜单覆盖",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn upsert_override(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(sys_menu_id): Path<String>,
    Json(params): Json<UpdateOverrideRequest>,
) -> Result<Response> {
    if !tc.is_super_admin && !tc.is_tenant_admin {
        return crate::views::errors::authz::admin_required();
    }
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let sys_menu_id = parse_uuid(sys_menu_id)?;
    tenant_menu_service::upsert_override(
        &ctx.db,
        tc.tenant_id,
        sys_menu_id,
        tc.user_id,
        &params,
        &audit_ctx,
    )
    .await?;

    format::json(())
}

#[utoipa::path(
    delete,
    path = "/api/menus/{sys_menu_id}/override",
    tag = "菜单管理",
    description = "删除菜单覆盖",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn remove_override(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(sys_menu_id): Path<String>,
) -> Result<Response> {
    if !tc.is_super_admin && !tc.is_tenant_admin {
        return crate::views::errors::authz::admin_required();
    }
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let sys_menu_id = parse_uuid(sys_menu_id)?;
    tenant_menu_service::delete_override(&ctx.db, tc.tenant_id, sys_menu_id, &audit_ctx)
        .await?;

    format::json(())
}

#[utoipa::path(
    get,
    path = "/api/users/me/menus",
    tag = "菜单管理",
    description = "获取当前用户菜单",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn my_menus(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let response: Vec<MergedMenuTreeResponse> = tenant_menu_service::get_user_menus(
        &ctx.db,
        tc.user_id,
        tc.tenant_id,
        tc.is_super_admin,
    )
    .await?;

    format::json(response)
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/menus")
        .add("/tree", openapi(get(tree), routes!(tree)))
        .add(
            "/{sys_menu_id}/override",
            openapi(put(upsert_override), routes!(upsert_override)),
        )
        .add(
            "/{sys_menu_id}/override",
            openapi(delete(remove_override), routes!(remove_override)),
        )
}

pub fn user_menu_routes() -> Routes {
    Routes::new()
        .prefix("/api/users")
        .add("/me/menus", openapi(get(my_menus), routes!(my_menus)))
}
