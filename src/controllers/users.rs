use loco_openapi::prelude::*;
use loco_rs::prelude::model::query;
use loco_rs::prelude::*;

use crate::extractors::{RequestMeta, TenantContext};
use crate::models::_entities::users;
use crate::services::auth_cache;
use crate::services::casbin_service::SharedEnforcer;
use crate::services::{tenant_service, user_service};
use crate::utils::error::{IntoModelResult, OptionErrInto};
use crate::views::audit_logs::AuditContext;
use crate::views::errors::parse_uuid;
use crate::views::users::{
    CreateSuperAdminRequest, CreateUserRequest, ResetPasswordRequest,
    ToggleStatusRequest, UpdateUserRequest, UserListParams, UserResponse,
    UserRolesResponse,
};

#[utoipa::path(
    get,
    path = "/api/users",
    tag = "用户管理",
    description = "查询用户列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(params): Query<UserListParams>,
) -> Result<Response> {
    let pagination = query::PaginationQuery {
        page: params.page,
        page_size: params.page_size,
    };
    let result =
        user_service::list_users(&ctx, tc.tenant_filter(), &pagination, &params).await?;

    format::json(result)
}

#[utoipa::path(
    post,
    path = "/api/users",
    tag = "用户管理",
    description = "创建用户",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn create(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateUserRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let user = user_service::create_user(
        &ctx.db,
        tc.tenant_id,
        tc.is_super_admin,
        &params,
        &audit_ctx,
    )
    .await
    .model_err()?;
    let (response_tenant_code, response_tenant_name) =
        if let Some(ref code) = params.tenant_code {
            if code != &tc.tenant_code {
                let t = tenant_service::find_tenant_by_code(&ctx.db, code).await?;
                (t.code.clone(), t.name)
            } else {
                (tc.tenant_code.clone(), tc.tenant_name.clone())
            }
        } else {
            (tc.tenant_code.clone(), tc.tenant_name.clone())
        };

    format::json(UserResponse::from_model(
        &user,
        &response_tenant_code,
        &response_tenant_name,
    ))
}

#[utoipa::path(
    put,
    path = "/api/users/{id}",
    tag = "用户管理",
    description = "更新用户",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn update(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<UpdateUserRequest>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;

    let target_user = users::Entity::find()
        .filter(users::Column::Id.eq(id_uuid))
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    if !tc.is_super_admin && target_user.tenant_id != tc.tenant_id {
        return crate::views::errors::user::cross_tenant("修改用户");
    }

    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let user = user_service::update_user(
        &ctx.db,
        id_uuid,
        target_user.tenant_id,
        &params,
        &audit_ctx,
    )
    .await?;

    format::json(UserResponse::from_model(
        &user,
        &tc.tenant_code,
        &tc.tenant_name,
    ))
}

#[utoipa::path(
    put,
    path = "/api/users/{id}/status",
    tag = "用户管理",
    description = "切换用户状态（启用/禁用）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn toggle_status(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<ToggleStatusRequest>,
) -> Result<Response> {
    if params.status != "active" && params.status != "disabled" {
        return crate::views::errors::bad_request(
            "user.invalid_status",
            "状态必须为 'active' 或 'disabled'",
        );
    }
    let id_uuid = parse_uuid(id)?;

    let target_user = users::Entity::find()
        .filter(users::Column::Id.eq(id_uuid))
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    if !tc.is_super_admin && target_user.tenant_id != tc.tenant_id {
        return crate::views::errors::user::cross_tenant("修改用户");
    }

    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let user = user_service::toggle_user_status(
        &ctx.db,
        id_uuid,
        target_user.tenant_id,
        tc.user_id,
        &params.status,
        &audit_ctx,
    )
    .await?;
    format::json(UserResponse::from_model(
        &user,
        &tc.tenant_code,
        &tc.tenant_name,
    ))
}

#[utoipa::path(
    put,
    path = "/api/users/{id}/reset-password",
    tag = "用户管理",
    description = "重置用户密码",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn reset_password(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<ResetPasswordRequest>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;

    let target_user = users::Entity::find()
        .filter(users::Column::Id.eq(id_uuid))
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    if !tc.is_super_admin && target_user.tenant_id != tc.tenant_id {
        return crate::views::errors::user::cross_tenant("重置密码");
    }

    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let user = user_service::reset_password(
        &ctx.db,
        id_uuid,
        target_user.tenant_id,
        &params.password,
        &audit_ctx,
    )
    .await?;

    // Invalidate auth cache so the target user's old JWT becomes invalid
    // (password_changed_at bumped in DB, cached value must be evicted).
    auth_cache::invalidate_user(&ctx.cache, id_uuid).await;

    format::json(UserResponse::from_model(
        &user,
        &tc.tenant_code,
        &tc.tenant_name,
    ))
}

#[utoipa::path(
    post,
    path = "/api/users/super-admin",
    tag = "用户管理",
    description = "创建超级管理员（仅超级管理员可调用）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn create_super_admin(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateSuperAdminRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::forbidden(
            "user.super_admin_create",
            "仅超级管理员可创建超级管理员",
        );
    }

    let Some(enforcer) = ctx.shared_store.get::<SharedEnforcer>() else {
        return crate::views::errors::internal(
            "casbin.not_initialized",
            "Casbin 策略引擎未初始化",
        );
    };

    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let user =
        user_service::create_super_admin(&ctx.db, &enforcer, &params, &audit_ctx).await?;

    format::json(UserResponse::from_model(&user, "DEFAULT", "默认租户"))
}

#[utoipa::path(
    get,
    path = "/api/users/{id}/roles",
    tag = "用户管理",
    description = "查询用户当前角色列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_user_roles(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;

    let target_user = users::Entity::find()
        .filter(users::Column::Id.eq(id_uuid))
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    if !tc.is_super_admin && target_user.tenant_id != tc.tenant_id {
        return crate::views::errors::user::cross_tenant("查看角色");
    }

    let role_ids =
        user_service::get_user_role_ids(&ctx.db, id_uuid, target_user.tenant_id).await?;

    format::json(UserRolesResponse {
        role_ids: role_ids.iter().map(|id| id.to_string()).collect(),
    })
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/users")
        .add("/", openapi(get(list), routes!(list)))
        .add("/", openapi(post(create), routes!(create)))
        .add(
            "/super-admin",
            openapi(post(create_super_admin), routes!(create_super_admin)),
        )
        .add("/{id}", openapi(put(update), routes!(update)))
        .add(
            "/{id}/roles",
            openapi(get(get_user_roles), routes!(get_user_roles)),
        )
        .add(
            "/{id}/status",
            openapi(put(toggle_status), routes!(toggle_status)),
        )
        .add(
            "/{id}/reset-password",
            openapi(put(reset_password), routes!(reset_password)),
        )
}
