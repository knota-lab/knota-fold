use axum::http::StatusCode;
use loco_rs::controller::ErrorDetail;
use loco_rs::prelude::model::query;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, TransactionTrait,
};
use std::collections::HashMap;
use uuid::Uuid;

use crate::models::_entities::roles;
use crate::models::_entities::sys_role_templates;
use crate::models::_entities::tenants;
use crate::models::_entities::user_roles;
use crate::models::_entities::users;
use crate::models::roles as roles_model;
use crate::models::users as users_model;
use crate::models::users::RegisterParams;
use crate::services::audit_service;
use crate::services::casbin_service::{self, SharedEnforcer};
use crate::services::login_guard;
use crate::utils::error::{IntoAppError, IntoModelResult, OptionErrInto};
use crate::views::audit_logs::{AuditAction, AuditContext, UserAuditSnapshot};
use crate::views::errors::err_bad_request;
use crate::views::pagination::PaginatedResponse;
use crate::views::users::{
    CreateSuperAdminRequest, CreateUserRequest, UpdateUserRequest, UserListParams,
    UserResponse,
};

#[tracing::instrument(skip_all)]
pub async fn list_users(
    ctx: &loco_rs::app::AppContext,
    tenant_id: Option<Uuid>,
    pagination: &query::PaginationQuery,
    search: &UserListParams,
) -> loco_rs::Result<PaginatedResponse<UserResponse>> {
    let db = &ctx.db;
    let mut base_query = users::Entity::find();
    if let Some(tid) = tenant_id {
        base_query = base_query.filter(users::Column::TenantId.eq(tid));
    }
    if let Some(ref name) = search.name {
        base_query = base_query.filter(users::Column::Name.contains(name));
    }
    if let Some(ref email) = search.email {
        base_query = base_query.filter(users::Column::Email.contains(email));
    }
    if let Some(ref status) = search.status {
        base_query = base_query.filter(users::Column::Status.eq(status));
    }
    let page_response = query::paginate(db, base_query, None, pagination).await?;
    let tenant_info = load_tenant_info(
        db,
        page_response
            .page
            .iter()
            .map(|user| user.tenant_id)
            .collect(),
    )
    .await?;

    // Resolve lock status for the current page only (cache hit ≈ O(1) per
    // entry; Redis backend can be optimised later with a pipeline if N
    // grows). Lookups are best-effort: a cache miss / error is treated as
    // "not locked" so the list endpoint never fails on transient cache
    // issues.
    let mut lock_map: std::collections::HashMap<String, i64> =
        std::collections::HashMap::with_capacity(page_response.page.len());
    for user in &page_response.page {
        if let Some(until) = login_guard::get_lock_until(&ctx.cache, &user.email).await {
            lock_map.insert(user.email.to_lowercase(), until);
        }
    }

    Ok(PaginatedResponse::from_page_response(
        &page_response,
        pagination,
        |m| {
            let (code, name) = tenant_info
                .get(&m.tenant_id)
                .map_or(("", ""), |(c, n)| (c.as_str(), n.as_str()));
            let unlock_at = lock_map.get(&m.email.to_lowercase()).copied();
            UserResponse::from_model_with_lock(m, code, name, unlock_at)
        },
    ))
}

#[tracing::instrument(skip_all)]
pub async fn create_user(
    db: &DatabaseConnection,
    caller_tenant_id: Uuid,
    is_super_admin: bool,
    params: &CreateUserRequest,
    audit_ctx: &AuditContext,
) -> Result<users::Model> {
    let effective_tenant_id = if let Some(code) = &params.tenant_code {
        if !is_super_admin {
            return Err(Error::CustomError(
                StatusCode::BAD_REQUEST,
                ErrorDetail::new(
                    "common.bad_request",
                    "仅超级管理员可在其他租户下创建用户",
                ),
            ));
        }
        let tenant = crate::models::tenants::Model::find_by_code(db, code)
            .await
            .model_err()
            .map_err(|_| {
                Error::CustomError(
                    StatusCode::BAD_REQUEST,
                    ErrorDetail::new("common.bad_request", "租户未找到"),
                )
            })?;
        tenant.id
    } else {
        caller_tenant_id
    };

    let user = users_model::Model::create_with_password(
        db,
        &RegisterParams {
            email: params.email.clone(),
            password: params.password.clone(),
            name: params.name.clone(),
            tenant_id: Some(effective_tenant_id),
        },
    )
    .await
    .model_err()?;

    let snapshot = UserAuditSnapshot::from(&user);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Create,
        "user",
        &user.id.to_string(),
        None::<&UserAuditSnapshot>,
        Some(&snapshot),
    )
    .await?;

    Ok(user)
}

#[tracing::instrument(skip_all)]
pub async fn update_user(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Uuid,
    params: &UpdateUserRequest,
    audit_ctx: &AuditContext,
) -> Result<users::Model> {
    let user = users::Entity::find()
        .filter(users::Column::Id.eq(id))
        .filter(users::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .model_err()?
        .ok_or_else(|| {
            Error::CustomError(
                StatusCode::NOT_FOUND,
                ErrorDetail::new("common.not_found", "用户未找到"),
            )
        })?;

    let before = UserAuditSnapshot::from(&user);

    let mut am: users_model::ActiveModel = user.into();

    if let Some(name) = &params.name {
        am.name = ActiveValue::Set(name.clone());
    }

    am.id = ActiveValue::Unchanged(id);
    let updated = am.update(db).await.model_err()?;

    let after = UserAuditSnapshot::from(&updated);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "user",
        &id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await?;

    Ok(updated)
}

#[tracing::instrument(skip_all)]
pub async fn toggle_user_status(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Uuid,
    caller_user_id: Uuid,
    new_status: &str,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<users::Model> {
    let user = users::Entity::find()
        .filter(users::Column::Id.eq(id))
        .filter(users::Column::TenantId.eq(tenant_id))
        .one(db)
        .await?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    if new_status == "disabled" {
        // Guard 1: Admins cannot disable themselves
        if caller_user_id == id {
            let caller_roles =
                roles_model::Model::find_user_role_codes(db, caller_user_id, tenant_id)
                    .await
                    .model_err()?;
            if caller_roles
                .iter()
                .any(|c| c == "SUPER_ADMIN" || c == "TENANT_ADMIN")
            {
                return Err(err_bad_request(
                    "user.cannot_disable_self",
                    "管理员不能禁用自己的帐户",
                ));
            }
        }

        // Guard 2: Cannot disable the last active super admin
        let target_roles = roles_model::Model::find_user_role_codes(db, id, tenant_id)
            .await
            .model_err()?;
        if target_roles.iter().any(|c| c == "SUPER_ADMIN") {
            let active_super_admin_count = count_active_super_admins(db).await?;
            if active_super_admin_count <= 1 {
                return Err(err_bad_request(
                    "user.last_super_admin",
                    "系统中仅剩一个超级管理员，不可禁用",
                ));
            }
        }
    }

    let before = UserAuditSnapshot::from(&user);

    let mut am: users_model::ActiveModel = user.into();
    am.status = ActiveValue::Set(new_status.to_string());
    am.id = ActiveValue::Unchanged(id);
    let updated = am.update(db).await.db_err()?;

    let after = UserAuditSnapshot::from(&updated);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "user",
        &id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await?;

    Ok(updated)
}

#[tracing::instrument(skip_all)]
pub async fn reset_password(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Uuid,
    password: &str,
    audit_ctx: &AuditContext,
) -> Result<users::Model> {
    let user = users::Entity::find()
        .filter(users::Column::Id.eq(id))
        .filter(users::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .model_err()?
        .ok_or_else(|| {
            Error::CustomError(
                StatusCode::NOT_FOUND,
                ErrorDetail::new("common.not_found", "用户未找到"),
            )
        })?;

    let before = UserAuditSnapshot::from(&user);

    let am: users_model::ActiveModel = user.into();
    let updated = am.reset_password(db, password).await.model_err()?;

    let after = UserAuditSnapshot::from(&updated);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::ResetPassword,
        "user",
        &id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await?;

    Ok(updated)
}

/// Create a tenant admin user with automatic default role binding.
#[tracing::instrument(skip_all)]
pub async fn create_tenant_admin(
    db: &DatabaseConnection,
    enforcer: &SharedEnforcer,
    tenant_id: Uuid,
    params: &CreateUserRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<users::Model> {
    let txn = db.begin().await.db_err()?;

    // Step 1: Create user bound to the specified tenant
    let user = users_model::Model::create_with_password(
        &txn,
        &RegisterParams {
            email: params.email.clone(),
            password: params.password.clone(),
            name: params.name.clone(),
            tenant_id: Some(tenant_id),
        },
    )
    .await
    .model_err()?;

    let snapshot = UserAuditSnapshot::from(&user);
    audit_service::log(
        &txn,
        audit_ctx,
        AuditAction::Create,
        "user",
        &user.id.to_string(),
        None::<&UserAuditSnapshot>,
        Some(&snapshot),
    )
    .await?;

    // Step 2: Find default template codes
    let default_template_codes: Vec<String> = sys_role_templates::Entity::find()
        .filter(sys_role_templates::Column::IsDefault.eq(true))
        .all(&txn)
        .await
        .db_err()?
        .into_iter()
        .map(|t| t.code)
        .collect();

    // Step 3: Find matching roles in this tenant
    let role_ids: Vec<Uuid> = roles::Entity::find()
        .filter(roles::Column::TenantId.eq(tenant_id))
        .filter(roles::Column::Code.is_in(default_template_codes))
        .filter(roles::Column::Status.eq("active"))
        .all(&txn)
        .await
        .db_err()?
        .into_iter()
        .map(|r| r.id)
        .collect();

    if role_ids.is_empty() {
        return Err(Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("tenant.no_default_roles", "No default roles found for this tenant. Ensure the tenant was created with template initialization."),
        ));
    }

    // Step 4: Sync user roles in DB only (no Casbin yet — avoids pool deadlock)
    roles_model::Model::sync_user_roles(&txn, tenant_id, user.id, role_ids).await?;
    let role_codes =
        roles_model::Model::find_user_role_codes(&txn, user.id, tenant_id).await?;

    txn.commit().await.db_err()?;

    // Step 5: Sync Casbin policies AFTER commit (connection is released)
    casbin_service::sync_user_roles(
        enforcer,
        &user.id.to_string(),
        &tenant_id.to_string(),
        &role_codes,
    )
    .await
    .map_err(|e| {
        let desc = format!("Failed to sync casbin: {e}");
        Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("casbin.sync_failed", &desc),
        )
    })?;

    Ok(user)
}

/// Count active users who hold the `SUPER_ADMIN` role (across all tenants).
async fn count_active_super_admins(db: &DatabaseConnection) -> loco_rs::Result<u64> {
    // Find all role IDs with code SUPER_ADMIN
    let super_admin_roles = roles::Entity::find()
        .filter(roles::Column::Code.eq("SUPER_ADMIN"))
        .filter(roles::Column::Status.eq("active"))
        .all(db)
        .await?;
    let sa_role_ids: Vec<Uuid> = super_admin_roles.into_iter().map(|r| r.id).collect();
    if sa_role_ids.is_empty() {
        return Ok(0);
    }

    // Find distinct user IDs bound to those roles
    let bindings = user_roles::Entity::find()
        .filter(user_roles::Column::RoleId.is_in(sa_role_ids))
        .all(db)
        .await?;
    let user_ids: Vec<Uuid> = bindings
        .into_iter()
        .map(|ur| ur.user_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    if user_ids.is_empty() {
        return Ok(0);
    }

    // Count those users who are currently active
    let count = users::Entity::find()
        .filter(users::Column::Id.is_in(user_ids))
        .filter(users::Column::Status.eq("active"))
        .count(db)
        .await?;

    Ok(count)
}

async fn load_tenant_info(
    db: &DatabaseConnection,
    tenant_ids: Vec<Uuid>,
) -> loco_rs::Result<HashMap<Uuid, (String, String)>> {
    if tenant_ids.is_empty() {
        return Ok(HashMap::new());
    }

    Ok(tenants::Entity::find()
        .filter(tenants::Column::Id.is_in(tenant_ids))
        .all(db)
        .await?
        .into_iter()
        .map(|tenant| (tenant.id, (tenant.code, tenant.name)))
        .collect())
}

/// Create a super admin user in the DEFAULT tenant with `SUPER_ADMIN` role.
#[tracing::instrument(skip_all)]
pub async fn create_super_admin(
    db: &DatabaseConnection,
    enforcer: &SharedEnforcer,
    params: &CreateSuperAdminRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<users::Model> {
    let default_tenant = tenants::Entity::find()
        .filter(tenants::Column::Code.eq("DEFAULT"))
        .one(db)
        .await?
        .ok_or(Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("tenant.default_not_found", "Default tenant not found"),
        ))?;

    let txn = db.begin().await.db_err()?;

    let user = users_model::Model::create_with_password(
        &txn,
        &RegisterParams {
            email: params.email.clone(),
            password: params.password.clone(),
            name: params.name.clone(),
            tenant_id: Some(default_tenant.id),
        },
    )
    .await
    .model_err()?;

    let snapshot = UserAuditSnapshot::from(&user);
    audit_service::log(
        &txn,
        audit_ctx,
        AuditAction::Create,
        "user",
        &user.id.to_string(),
        None::<&UserAuditSnapshot>,
        Some(&snapshot),
    )
    .await?;

    let super_admin_role = roles::Entity::find()
        .filter(roles::Column::TenantId.eq(default_tenant.id))
        .filter(roles::Column::Code.eq("SUPER_ADMIN"))
        .filter(roles::Column::Status.eq("active"))
        .one(&txn)
        .await?
        .ok_or(Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new(
                "role.super_admin_not_found",
                "SUPER_ADMIN role not found in default tenant",
            ),
        ))?;

    roles_model::Model::sync_user_roles(
        &txn,
        default_tenant.id,
        user.id,
        vec![super_admin_role.id],
    )
    .await
    .model_err()?;

    txn.commit().await.db_err()?;

    casbin_service::sync_user_roles(
        enforcer,
        &user.id.to_string(),
        &default_tenant.id.to_string(),
        &["SUPER_ADMIN".to_string()],
    )
    .await
    .map_err(|e| {
        let desc = format!("Failed to sync casbin: {e}");
        Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("casbin.sync_failed", &desc),
        )
    })?;

    Ok(user)
}

/// Get role IDs for a user within a tenant.
#[tracing::instrument(skip_all)]
pub async fn get_user_role_ids(
    db: &DatabaseConnection,
    user_id: Uuid,
    tenant_id: Uuid,
) -> loco_rs::Result<Vec<Uuid>> {
    let bindings = user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(user_roles::Column::TenantId.eq(tenant_id))
        .all(db)
        .await?;
    Ok(bindings.into_iter().map(|ur| ur.role_id).collect())
}
