use loco_openapi::prelude::*;
use loco_rs::prelude::model::query;
use loco_rs::prelude::*;

use crate::extractors::{RequestMeta, TenantContext};
use crate::models::_entities::roles;
use crate::models::_entities::users;
use crate::services::casbin_service::SharedEnforcer;
use crate::services::{
    permission_service, role_service, tenant_menu_service, tenant_service,
};
use crate::utils::error::IntoModelResult;
use crate::utils::error::OptionErrInto;
use crate::views::audit_logs::AuditContext;
use crate::views::errors::parse_uuid;
use crate::views::roles::{
    CreateRoleRequest, RoleListParams, RoleMenuIdsResponse, RolePermissionIdsResponse,
    RoleResponse, SyncRoleMenusRequest, SyncRolePermissionsRequest, SyncUserRolesRequest,
    ToggleRoleStatusRequest, UpdateRoleRequest,
};

#[utoipa::path(
    get,
    path = "/api/roles",
    tag = "角色管理",
    description = "查询角色列表（支持tenant_code过滤）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(params): Query<RoleListParams>,
) -> Result<Response> {
    let tenant_id_filter = if let Some(ref code) = params.tenant_code {
        let target_tenant = tenant_service::find_tenant_by_code(&ctx.db, code).await?;
        if !tc.is_super_admin && target_tenant.id != tc.tenant_id {
            return crate::views::errors::role::cross_tenant("查看角色");
        }
        Some(target_tenant.id)
    } else {
        tc.tenant_filter()
    };

    let pagination = query::PaginationQuery {
        page: params.page,
        page_size: params.page_size,
    };

    let result =
        role_service::list_roles(&ctx.db, tenant_id_filter, &pagination, &params).await?;

    format::json(result)
}

#[utoipa::path(
    post,
    path = "/api/roles",
    tag = "角色管理",
    description = "创建角色",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn create(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateRoleRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let role =
        role_service::create_role(&ctx.db, tc.tenant_id, tc.user_id, &params, &audit_ctx)
            .await?;

    format::json(RoleResponse::from_model(
        &role,
        &tc.tenant_code,
        &tc.tenant_name,
    ))
}

#[utoipa::path(
    put,
    path = "/api/roles/{id}",
    tag = "角色管理",
    description = "更新角色",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn update(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<UpdateRoleRequest>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let role = role_service::update_role(
        &ctx.db,
        id_uuid,
        tc.tenant_id,
        tc.user_id,
        &params,
        &audit_ctx,
    )
    .await?;

    format::json(RoleResponse::from_model(
        &role,
        &tc.tenant_code,
        &tc.tenant_name,
    ))
}

#[utoipa::path(
    put,
    path = "/api/roles/{id}/status",
    tag = "角色管理",
    description = "切换角色状态（启用/禁用）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn toggle_status(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<ToggleRoleStatusRequest>,
) -> Result<Response> {
    if params.status != "active" && params.status != "disabled" {
        return crate::views::errors::bad_request(
            "role.invalid_status",
            "状态必须为 'active' 或 'disabled'",
        );
    }
    let id_uuid = parse_uuid(id)?;
    let Some(enforcer) = ctx.shared_store.get::<SharedEnforcer>() else {
        return crate::views::errors::internal(
            "casbin.not_initialized",
            "Casbin 策略引擎未初始化",
        );
    };

    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let role = role_service::toggle_role_status(
        &ctx.db,
        &enforcer,
        id_uuid,
        tc.tenant_id,
        &params.status,
        &audit_ctx,
    )
    .await?;

    format::json(RoleResponse::from_model(
        &role,
        &tc.tenant_code,
        &tc.tenant_name,
    ))
}

#[utoipa::path(
    put,
    path = "/api/users/{id}/roles",
    tag = "角色管理",
    description = "同步用户角色",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn sync_user_roles(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<SyncUserRolesRequest>,
) -> Result<Response> {
    let target_user_uuid = parse_uuid(id)?;
    let Some(enforcer) = ctx.shared_store.get::<SharedEnforcer>() else {
        return crate::views::errors::internal(
            "casbin.not_initialized",
            "Casbin 策略引擎未初始化",
        );
    };

    let target_user = users::Entity::find()
        .filter(users::Column::Id.eq(target_user_uuid))
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::role::NOT_FOUND)?;

    if !tc.is_super_admin && target_user.tenant_id != tc.tenant_id {
        return crate::views::errors::role::cross_tenant("分配角色");
    }

    let audit_ctx = AuditContext::from_request(&tc, &meta);

    role_service::sync_user_roles(
        &ctx.db,
        &enforcer,
        target_user.tenant_id,
        target_user_uuid,
        params.role_ids.clone(),
        &audit_ctx,
    )
    .await?;

    format::json(())
}

#[utoipa::path(
    put,
    path = "/api/roles/{id}/permissions",
    tag = "角色管理",
    description = "同步角色权限",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn sync_role_permissions(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<SyncRolePermissionsRequest>,
) -> Result<Response> {
    let role_uuid = parse_uuid(id)?;
    let Some(enforcer) = ctx.shared_store.get::<SharedEnforcer>() else {
        return crate::views::errors::internal(
            "casbin.not_initialized",
            "Casbin 策略引擎未初始化",
        );
    };

    let target_role = roles::Entity::find()
        .filter(roles::Column::Id.eq(role_uuid))
        .filter(roles::Column::Status.eq("active"))
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::role::NOT_FOUND)?;

    if !tc.is_super_admin && target_role.tenant_id != tc.tenant_id {
        return crate::views::errors::role::cross_tenant("分配权限");
    }

    let audit_ctx = AuditContext::from_request(&tc, &meta);

    role_service::sync_role_permissions(
        &ctx.db,
        &enforcer,
        target_role.tenant_id,
        role_uuid,
        params.permission_ids.clone(),
        tc.user_id,
        tc.is_super_admin,
        &audit_ctx,
    )
    .await?;

    format::json(())
}

#[utoipa::path(
    get,
    path = "/api/roles/{id}/permissions",
    tag = "角色管理",
    description = "查询角色权限",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_role_permissions(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let role_uuid = parse_uuid(id)?;

    let target_role = roles::Entity::find()
        .filter(roles::Column::Id.eq(role_uuid))
        .filter(roles::Column::Status.eq("active"))
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::role::NOT_FOUND)?;

    if !tc.is_super_admin && target_role.tenant_id != tc.tenant_id {
        return crate::views::errors::role::cross_tenant("查看权限");
    }

    let permission_ids =
        role_service::get_role_permission_ids(&ctx.db, role_uuid, target_role.tenant_id)
            .await?;

    format::json(RolePermissionIdsResponse { permission_ids })
}

#[utoipa::path(
    get,
    path = "/api/roles/{id}/assignable-permissions",
    tag = "角色管理",
    description = "查询可分配权限（含元数据）及角色已分配权限ID",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn assignable_permissions(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let role_uuid = parse_uuid(id)?;

    let target_role = roles::Entity::find()
        .filter(roles::Column::Id.eq(role_uuid))
        .filter(roles::Column::Status.eq("active"))
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::role::NOT_FOUND)?;

    if !tc.is_super_admin && target_role.tenant_id != tc.tenant_id {
        return crate::views::errors::role::cross_tenant("查看权限");
    }

    let openapi = crate::controllers::permissions::get_openapi_spec_ref();
    let result = permission_service::get_assignable_permissions(
        &ctx.db,
        openapi,
        role_uuid,
        target_role.tenant_id,
        tc.user_id,
        tc.is_super_admin,
    )
    .await?;

    format::json(result)
}

#[utoipa::path(
    get,
    path = "/api/roles/{id}/menus",
    tag = "角色管理",
    description = "查询角色菜单",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_role_menus(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let role_uuid = parse_uuid(id)?;

    let target_role = roles::Entity::find()
        .filter(roles::Column::Id.eq(role_uuid))
        .filter(roles::Column::Status.eq("active"))
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::role::NOT_FOUND)?;

    if !tc.is_super_admin && target_role.tenant_id != tc.tenant_id {
        return crate::views::errors::role::cross_tenant("查看菜单");
    }

    let ids = role_service::get_role_menu_ids(&ctx.db, role_uuid, target_role.tenant_id)
        .await?;
    format::json(RoleMenuIdsResponse { sys_menu_ids: ids })
}

#[utoipa::path(
    put,
    path = "/api/roles/{id}/menus",
    tag = "角色管理",
    description = "同步角色菜单",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn sync_role_menus(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<SyncRoleMenusRequest>,
) -> Result<Response> {
    let role_uuid = parse_uuid(id)?;

    let target_role = roles::Entity::find()
        .filter(roles::Column::Id.eq(role_uuid))
        .filter(roles::Column::Status.eq("active"))
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::role::NOT_FOUND)?;

    if !tc.is_super_admin && target_role.tenant_id != tc.tenant_id {
        return crate::views::errors::role::cross_tenant("同步菜单");
    }

    let audit_ctx = AuditContext::from_request(&tc, &meta);

    role_service::sync_role_menus(
        &ctx.db,
        target_role.tenant_id,
        role_uuid,
        params.sys_menu_ids.clone(),
        tc.user_id,
        tc.is_super_admin,
        &audit_ctx,
    )
    .await?;
    format::json(())
}

#[utoipa::path(
    get,
    path = "/api/roles/{id}/assignable-menus",
    tag = "角色管理",
    description = "查询可分配菜单树及角色已分配菜单ID",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn assignable_menus(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let role_uuid = parse_uuid(id)?;

    let target_role = roles::Entity::find()
        .filter(roles::Column::Id.eq(role_uuid))
        .filter(roles::Column::Status.eq("active"))
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::role::NOT_FOUND)?;

    if !tc.is_super_admin && target_role.tenant_id != tc.tenant_id {
        return crate::views::errors::role::cross_tenant("查看菜单");
    }

    let result = tenant_menu_service::get_assignable_menus(
        &ctx.db,
        role_uuid,
        target_role.tenant_id,
        tc.user_id,
        tc.is_super_admin,
    )
    .await?;

    format::json(result)
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/roles")
        .add("/", openapi(get(list), routes!(list)))
        .add("/", openapi(post(create), routes!(create)))
        .add("/{id}", openapi(put(update), routes!(update)))
        .add(
            "/{id}/status",
            openapi(put(toggle_status), routes!(toggle_status)),
        )
        .add(
            "/{id}/menus",
            openapi(get(get_role_menus), routes!(get_role_menus)),
        )
        .add(
            "/{id}/menus",
            openapi(put(sync_role_menus), routes!(sync_role_menus)),
        )
        .add(
            "/{id}/permissions",
            openapi(get(get_role_permissions), routes!(get_role_permissions)),
        )
        .add(
            "/{id}/permissions",
            openapi(put(sync_role_permissions), routes!(sync_role_permissions)),
        )
        .add(
            "/{id}/assignable-permissions",
            openapi(get(assignable_permissions), routes!(assignable_permissions)),
        )
        .add(
            "/{id}/assignable-menus",
            openapi(get(assignable_menus), routes!(assignable_menus)),
        )
}

pub fn user_role_routes() -> Routes {
    Routes::new().prefix("/api/users").add(
        "/{id}/roles",
        openapi(put(sync_user_roles), routes!(sync_user_roles)),
    )
}
