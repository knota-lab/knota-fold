use loco_openapi::prelude::*;
use loco_rs::prelude::model::query;
use loco_rs::prelude::*;

use crate::extractors::{RequestMeta, TenantContext};
use crate::models::roles as roles_model;
use crate::services::casbin_service::SharedEnforcer;
use crate::services::{tenant_service, user_service};
use crate::views::audit_logs::AuditContext;
use crate::views::errors::parse_uuid;
use crate::views::roles::RoleResponse;
use crate::views::tenants::{
    CreateTenantRequest, TenantListParams, TenantResponse, UpdateTenantRequest,
};
use crate::views::users::{CreateUserRequest, UserResponse};

#[utoipa::path(
    get,
    path = "/api/tenants",
    tag = "租户管理",
    description = "查询租户列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(params): Query<TenantListParams>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }
    let pagination = query::PaginationQuery {
        page: params.page,
        page_size: params.page_size,
    };
    let result = tenant_service::list_tenants(&ctx.db, &pagination, &params).await?;

    format::json(result)
}

#[utoipa::path(
    post,
    path = "/api/tenants",
    tag = "租户管理",
    description = "创建租户",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn create(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateTenantRequest>,
) -> Result<Response> {
    let Some(enforcer) = ctx.shared_store.get::<SharedEnforcer>() else {
        return crate::views::errors::internal(
            "casbin.not_initialized",
            "Casbin 策略引擎未初始化",
        );
    };

    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let tenant = tenant_service::create_tenant_with_init(
        &ctx.db, &enforcer, &params, tc.user_id, &audit_ctx,
    )
    .await?;

    format::json(TenantResponse::from_model(&tenant))
}

#[utoipa::path(
    post,
    path = "/api/sys/tenants/{tenant_code}/admins",
    tag = "租户管理",
    description = "为指定租户创建管理员（超管专用）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn create_admin(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(tenant_code): Path<String>,
    Json(params): Json<CreateUserRequest>,
) -> Result<Response> {
    let Some(enforcer) = ctx.shared_store.get::<SharedEnforcer>() else {
        return crate::views::errors::internal(
            "casbin.not_initialized",
            "Casbin 策略引擎未初始化",
        );
    };

    let audit_ctx = AuditContext::from_request(&tc, &meta);

    let tenant = tenant_service::find_tenant_by_code(&ctx.db, &tenant_code).await?;
    if tenant.status != "active" {
        return crate::views::errors::forbidden("common.tenant_inactive", "租户已停用");
    }
    let user = user_service::create_tenant_admin(
        &ctx.db, &enforcer, tenant.id, &params, &audit_ctx,
    )
    .await?;

    format::json(UserResponse::from_model(&user, &tenant_code, &tenant.name))
}

#[utoipa::path(
    get,
    path = "/api/sys/tenants/{tenant_code}/roles",
    tag = "租户管理",
    description = "查询指定租户的角色列表（超管专用）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_tenant_roles(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(tenant_code): Path<String>,
) -> Result<Response> {
    let tenant = tenant_service::find_tenant_by_code(&ctx.db, &tenant_code).await?;
    let roles = roles_model::Model::find_by_tenant(&ctx.db, tenant.id).await?;
    let responses: Vec<RoleResponse> = roles
        .iter()
        .map(|r| RoleResponse::from_model(r, &tenant_code, &tenant.name))
        .collect();
    format::json(responses)
}

#[utoipa::path(
    put,
    path = "/api/tenants/{id}",
    tag = "租户管理",
    description = "更新租户",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn update(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<UpdateTenantRequest>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let tenant =
        tenant_service::update_tenant(&ctx.db, id_uuid, &params, &audit_ctx).await?;

    format::json(TenantResponse::from_model(&tenant))
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/tenants")
        .add("/", openapi(get(list), routes!(list)))
        .add("/", openapi(post(create), routes!(create)))
        .add("/{id}", openapi(put(update), routes!(update)))
}

pub fn sys_routes() -> Routes {
    Routes::new()
        .prefix("/api/sys/tenants")
        .add(
            "/{tenant_code}/admins",
            openapi(post(create_admin), routes!(create_admin)),
        )
        .add(
            "/{tenant_code}/roles",
            openapi(get(get_tenant_roles), routes!(get_tenant_roles)),
        )
}
