use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use crate::extractors::RequestMeta;
use crate::extractors::TenantContext;
use crate::services::sys_menu_service;
use crate::views::audit_logs::AuditContext;
use crate::views::errors::parse_uuid;
use crate::views::sys_menus::{
    CreateSysMenuRequest, SysMenuResponse, SysMenuTreeResponse, UpdateSysMenuRequest,
};

#[utoipa::path(
    get,
    path = "/api/sys-menus",
    tag = "系统菜单管理",
    description = "查询系统菜单列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let response: Vec<SysMenuResponse> =
        sys_menu_service::list_sys_menus(&ctx.db).await?;
    format::json(response)
}

#[utoipa::path(
    get,
    path = "/api/sys-menus/tree",
    tag = "系统菜单管理",
    description = "查询系统菜单树",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn tree(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let response: Vec<SysMenuTreeResponse> =
        sys_menu_service::get_sys_menu_tree(&ctx.db).await?;
    format::json(response)
}

#[utoipa::path(
    post,
    path = "/api/sys-menus",
    tag = "系统菜单管理",
    description = "创建系统菜单",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn create(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateSysMenuRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let menu =
        sys_menu_service::create_sys_menu(&ctx.db, tc.user_id, &params, &audit_ctx)
            .await?;
    format::json(SysMenuResponse::from_model(&menu))
}

#[utoipa::path(
    put,
    path = "/api/sys-menus/{id}",
    tag = "系统菜单管理",
    description = "更新系统菜单",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn update(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<UpdateSysMenuRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let id_uuid = parse_uuid(id)?;
    let menu = sys_menu_service::update_sys_menu(
        &ctx.db, id_uuid, tc.user_id, &params, &audit_ctx,
    )
    .await?;
    format::json(SysMenuResponse::from_model(&menu))
}

#[utoipa::path(
    delete,
    path = "/api/sys-menus/{id}",
    tag = "系统菜单管理",
    description = "删除系统菜单",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn remove(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let id_uuid = parse_uuid(id)?;
    sys_menu_service::delete_sys_menu(&ctx.db, id_uuid, &audit_ctx).await?;
    format::json(())
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/sys-menus")
        .add("/", openapi(get(list), routes!(list)))
        .add("/tree", openapi(get(tree), routes!(tree)))
        .add("/", openapi(post(create), routes!(create)))
        .add("/{id}", openapi(put(update), routes!(update)))
        .add("/{id}", openapi(delete(remove), routes!(remove)))
}
