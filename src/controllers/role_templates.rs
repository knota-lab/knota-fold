use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use crate::extractors::{RequestMeta, TenantContext};
use crate::services::role_template_service;
use crate::views::audit_logs::AuditContext;
use crate::views::errors::parse_uuid;
use crate::views::role_templates::{
    CreateRoleTemplateRequest, SyncTemplateMenusRequest, SyncTemplatePermissionsRequest,
    TemplateMenuIdsResponse, UpdateRoleTemplateRequest,
};

#[utoipa::path(
    get,
    path = "/api/sys/role-templates",
    tag = "角色模板管理",
    description = "查询所有角色模板",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let templates = role_template_service::list_templates(&ctx.db).await?;
    format::json(templates)
}

#[utoipa::path(
    post,
    path = "/api/sys/role-templates",
    tag = "角色模板管理",
    description = "新建角色模板",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn create(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    meta: RequestMeta,
    Json(params): Json<CreateRoleTemplateRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let template =
        role_template_service::create_template(&ctx.db, &params, &audit_ctx).await?;
    format::json(template)
}

#[utoipa::path(
    put,
    path = "/api/sys/role-templates/{id}",
    tag = "角色模板管理",
    description = "更新角色模板",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn update(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    meta: RequestMeta,
    Path(id): Path<String>,
    Json(params): Json<UpdateRoleTemplateRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let id_uuid = parse_uuid(id)?;
    let template =
        role_template_service::update_template(&ctx.db, id_uuid, &params, &audit_ctx)
            .await?;
    format::json(template)
}

#[utoipa::path(
    delete,
    path = "/api/sys/role-templates/{id}",
    tag = "角色模板管理",
    description = "删除角色模板",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn remove(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    meta: RequestMeta,
    Path(id): Path<String>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let id_uuid = parse_uuid(id)?;
    role_template_service::delete_template(&ctx.db, id_uuid, &audit_ctx).await?;
    format::json(())
}

#[utoipa::path(
    get,
    path = "/api/sys/role-templates/{id}/menus",
    tag = "角色模板管理",
    description = "查询模板关联的菜单",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_template_menus(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;
    let ids = role_template_service::get_template_menu_ids(&ctx.db, id_uuid).await?;
    format::json(TemplateMenuIdsResponse { sys_menu_ids: ids })
}

#[utoipa::path(
    put,
    path = "/api/sys/role-templates/{id}/menus",
    tag = "角色模板管理",
    description = "同步模板菜单关联",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn sync_template_menus(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    meta: RequestMeta,
    Path(id): Path<String>,
    Json(params): Json<SyncTemplateMenusRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let id_uuid = parse_uuid(id)?;
    role_template_service::sync_template_menus(
        &ctx.db,
        id_uuid,
        params.sys_menu_ids,
        &audit_ctx,
    )
    .await?;
    format::json(())
}

#[utoipa::path(
    get,
    path = "/api/sys/role-templates/{id}/permissions",
    tag = "角色模板管理",
    description = "查询模板关联的权限",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_template_permissions(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;
    let permissions =
        role_template_service::get_template_permissions(&ctx.db, id_uuid).await?;
    format::json(permissions)
}

#[utoipa::path(
    put,
    path = "/api/sys/role-templates/{id}/permissions",
    tag = "角色模板管理",
    description = "同步模板权限关联",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn sync_template_permissions(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    meta: RequestMeta,
    Path(id): Path<String>,
    Json(params): Json<SyncTemplatePermissionsRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let id_uuid = parse_uuid(id)?;
    let permissions: Vec<(String, String)> = params
        .permissions
        .into_iter()
        .map(|p| (p.obj, p.act))
        .collect();
    role_template_service::sync_template_permissions(
        &ctx.db,
        id_uuid,
        permissions,
        &audit_ctx,
    )
    .await?;
    format::json(())
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/sys/role-templates")
        .add("/", openapi(get(list), routes!(list)))
        .add("/", openapi(post(create), routes!(create)))
        .add("/{id}", openapi(put(update), routes!(update)))
        .add("/{id}", openapi(delete(remove), routes!(remove)))
        .add(
            "/{id}/menus",
            openapi(get(get_template_menus), routes!(get_template_menus)),
        )
        .add(
            "/{id}/menus",
            openapi(put(sync_template_menus), routes!(sync_template_menus)),
        )
        .add(
            "/{id}/permissions",
            openapi(
                get(get_template_permissions),
                routes!(get_template_permissions),
            ),
        )
        .add(
            "/{id}/permissions",
            openapi(
                put(sync_template_permissions),
                routes!(sync_template_permissions),
            ),
        )
}
