use axum::http::StatusCode;
use loco_rs::controller::ErrorDetail;
use loco_rs::prelude::model::query;
use loco_rs::prelude::*;
use sea_orm::PaginatorTrait;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter, TransactionTrait,
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::models::_entities::permissions;
use crate::models::_entities::role_menus;
use crate::models::_entities::roles;
use crate::models::_entities::sys_menus;
use crate::models::_entities::tenants;
use crate::models::_entities::user_roles;
use crate::models::permissions as permissions_model;
use crate::models::roles as roles_model;
use crate::services::audit_service;
use crate::services::casbin_service::{self, SharedEnforcer};
use crate::utils::error::{IntoAppError, IntoModelResult, OptionErrInto};
use crate::views::audit_logs::{AuditAction, AuditContext, RoleAuditSnapshot};
use crate::views::pagination::PaginatedResponse;
use crate::views::roles::{
    CreateRoleRequest, RoleListParams, RoleResponse, UpdateRoleRequest,
};

#[tracing::instrument(skip_all)]
pub async fn list_roles(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    pagination: &query::PaginationQuery,
    search: &RoleListParams,
) -> loco_rs::Result<PaginatedResponse<RoleResponse>> {
    let mut base_query = roles::Entity::find();
    if let Some(tid) = tenant_id {
        base_query = base_query.filter(roles::Column::TenantId.eq(tid));
    }
    if let Some(ref name) = search.name {
        base_query = base_query.filter(roles::Column::Name.contains(name));
    }
    if let Some(ref status) = search.status {
        base_query = base_query.filter(roles::Column::Status.eq(status));
    }
    let page_response = query::paginate(db, base_query, None, pagination).await?;
    let tenant_info = load_tenant_info(
        db,
        page_response
            .page
            .iter()
            .map(|role| role.tenant_id)
            .collect(),
    )
    .await?;

    Ok(PaginatedResponse::from_page_response(
        page_response,
        pagination,
        |m| {
            let (code, name) = tenant_info
                .get(&m.tenant_id)
                .map_or(("", ""), |(c, n)| (c.as_str(), n.as_str()));
            RoleResponse::from_model(m, code, name)
        },
    ))
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

#[tracing::instrument(skip_all)]
pub async fn create_role(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    params: &CreateRoleRequest,
    audit_ctx: &AuditContext,
) -> Result<roles::Model> {
    if RESERVED_ROLE_CODES
        .iter()
        .any(|&c| c.eq_ignore_ascii_case(&params.code))
    {
        return Err(Error::CustomError(
            StatusCode::BAD_REQUEST,
            ErrorDetail::new("common.bad_request", "角色编码为系统保留编码，不可使用"),
        ));
    }

    let mut am = roles_model::ActiveModel {
        name: ActiveValue::Set(params.name.clone()),
        code: ActiveValue::Set(params.code.clone()),
        ..Default::default()
    };

    if let Some(parent_id) = params.parent_id {
        am.parent_id = ActiveValue::Set(Some(parent_id));
    }
    if let Some(description) = &params.description {
        am.description = ActiveValue::Set(Some(description.clone()));
    }

    let role = roles_model::Model::create_role(db, tenant_id, am, user_id)
        .await
        .model_err()?;

    let snapshot = RoleAuditSnapshot::from(&role);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Create,
        "role",
        &role.id.to_string(),
        None::<&RoleAuditSnapshot>,
        Some(&snapshot),
    )
    .await?;

    Ok(role)
}

#[tracing::instrument(skip_all)]
pub async fn update_role(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
    params: &UpdateRoleRequest,
    audit_ctx: &AuditContext,
) -> Result<roles::Model> {
    if let Some(code) = &params.code {
        if RESERVED_ROLE_CODES
            .iter()
            .any(|&c| c.eq_ignore_ascii_case(code))
        {
            return Err(Error::CustomError(
                StatusCode::BAD_REQUEST,
                ErrorDetail::new(
                    "common.bad_request",
                    "角色编码为系统保留编码，不可使用",
                ),
            ));
        }
    }

    // Load before state for audit
    let existing = roles_model::Model::find_by_id_and_tenant(db, id, tenant_id)
        .await
        .model_err()?;
    let before = RoleAuditSnapshot::from(&existing);

    let mut am = roles_model::ActiveModel {
        version: ActiveValue::Set(params.version),
        ..Default::default()
    };

    if let Some(name) = &params.name {
        am.name = ActiveValue::Set(name.clone());
    }
    if let Some(code) = &params.code {
        am.code = ActiveValue::Set(code.clone());
    }
    if let Some(parent_id) = params.parent_id {
        am.parent_id = ActiveValue::Set(parent_id);
    }
    if let Some(description) = &params.description {
        am.description = ActiveValue::Set(description.clone());
    }

    let role = roles_model::Model::update_with_version(db, id, tenant_id, am, user_id)
        .await
        .model_err()?;

    let after = RoleAuditSnapshot::from(&role);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "role",
        &id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await?;

    Ok(role)
}

/// Role codes that must not be disabled to prevent system lockout.
const PROTECTED_ROLE_CODES: &[&str] = &["SUPER_ADMIN", "TENANT_ADMIN"];

/// Role codes reserved by the system — cannot be created or renamed to.
const RESERVED_ROLE_CODES: &[&str] = &["SUPER_ADMIN", "TENANT_ADMIN", "MEMBER"];

#[tracing::instrument(skip_all)]
pub async fn toggle_role_status(
    db: &DatabaseConnection,
    enforcer: &SharedEnforcer,
    id: Uuid,
    tenant_id: Uuid,
    new_status: &str,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<roles::Model> {
    if new_status == "disabled" {
        let target = roles_model::Model::find_by_id_and_tenant(db, id, tenant_id)
            .await
            .model_err()?;
        if PROTECTED_ROLE_CODES.contains(&target.code.as_str()) {
            let desc = format!("角色 {} 为系统保护角色，不可禁用", target.code);
            return Err(Error::CustomError(
                StatusCode::FORBIDDEN,
                ErrorDetail::new("role.protected", &desc),
            ));
        }
    }

    // Load before state for audit — use unfiltered query since role may be disabled
    let existing = roles::Entity::find()
        .filter(roles::Column::Id.eq(id))
        .filter(roles::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .db_err()?
        .or_err(crate::error_info::role::NOT_FOUND)?;
    let before = RoleAuditSnapshot::from(&existing);

    let role = roles_model::Model::toggle_status(db, id, tenant_id, new_status)
        .await
        .model_err()?;

    let after = RoleAuditSnapshot::from(&role);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "role",
        &id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await?;

    if new_status == "disabled" {
        casbin_service::remove_role_policies(
            enforcer,
            &role.code,
            &tenant_id.to_string(),
        )
        .await
        .map_err(|e| {
            let desc = format!("Failed to sync casbin: {e}");
            Error::CustomError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetail::new("casbin.sync_failed", &desc),
            )
        })?;
    } else {
        let obj_acts =
            permissions_model::Model::find_role_permission_obj_acts(db, id, tenant_id)
                .await?;
        casbin_service::sync_role_permissions(
            enforcer,
            &role.code,
            &tenant_id.to_string(),
            &obj_acts,
        )
        .await
        .map_err(|e| {
            let desc = format!("Failed to sync casbin: {e}");
            Error::CustomError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetail::new("casbin.sync_failed", &desc),
            )
        })?;

        use crate::models::_entities::user_roles;
        let user_role_records = user_roles::Entity::find()
            .filter(user_roles::Column::RoleId.eq(id))
            .filter(user_roles::Column::TenantId.eq(tenant_id))
            .all(db)
            .await
            .model_err()?;
        for ur in user_role_records {
            let role_codes =
                roles_model::Model::find_user_role_codes(db, ur.user_id, tenant_id)
                    .await
                    .model_err()?;
            casbin_service::sync_user_roles(
                enforcer,
                &ur.user_id.to_string(),
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
        }
    }

    Ok(role)
}

#[tracing::instrument(skip_all)]
pub async fn sync_user_roles<C: ConnectionTrait>(
    db: &C,
    enforcer: &SharedEnforcer,
    tenant_id: Uuid,
    target_user_id: Uuid,
    role_ids: Vec<Uuid>,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    if !role_ids.is_empty() {
        let valid_count = roles::Entity::find()
            .filter(roles::Column::Id.is_in(role_ids.clone()))
            .filter(roles::Column::TenantId.eq(tenant_id))
            .filter(roles::Column::Status.eq("active"))
            .count(db)
            .await
            .model_err()?;

        if valid_count as usize != role_ids.len() {
            return Err(loco_rs::Error::CustomError(
                StatusCode::BAD_REQUEST,
                ErrorDetail::new(
                    "role.invalid_role_ids",
                    "One or more role_ids do not belong to the target tenant",
                ),
            ));
        }
    }

    let old_role_ids: Vec<String> = user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(target_user_id))
        .filter(user_roles::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .model_err()?
        .into_iter()
        .map(|ur| ur.role_id.to_string())
        .collect();

    roles_model::Model::sync_user_roles(db, tenant_id, target_user_id, role_ids.clone())
        .await?;

    let role_codes =
        roles_model::Model::find_user_role_codes(db, target_user_id, tenant_id).await?;

    casbin_service::sync_user_roles(
        enforcer,
        &target_user_id.to_string(),
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

    let new_role_ids: Vec<String> = role_ids.iter().map(|id| id.to_string()).collect();
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "user_roles",
        &target_user_id.to_string(),
        Some(&serde_json::json!({"roleIds": old_role_ids})),
        Some(&serde_json::json!({"roleIds": new_role_ids})),
    )
    .await?;

    Ok(())
}

#[tracing::instrument(skip_all)]
pub async fn get_role_permission_ids<C: ConnectionTrait>(
    db: &C,
    role_id: Uuid,
    tenant_id: Uuid,
) -> loco_rs::Result<Vec<String>> {
    let ids = permissions_model::Model::find_role_permission_ids(db, role_id, tenant_id)
        .await?;
    Ok(ids.into_iter().map(|id| id.to_string()).collect())
}

#[tracing::instrument(skip_all)]
#[allow(clippy::too_many_arguments)]
pub async fn sync_role_permissions<C: ConnectionTrait>(
    db: &C,
    enforcer: &SharedEnforcer,
    tenant_id: Uuid,
    role_id: Uuid,
    permission_ids: Vec<Uuid>,
    actor_user_id: Uuid,
    is_super_admin: bool,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    // Deduplicate permission IDs
    let permission_ids: Vec<Uuid> = {
        let mut seen = HashSet::new();
        permission_ids
            .into_iter()
            .filter(|id| seen.insert(*id))
            .collect()
    };

    if !permission_ids.is_empty() {
        let valid_count = permissions::Entity::find()
            .filter(permissions::Column::Id.is_in(permission_ids.clone()))
            .filter(permissions::Column::DeletedAt.is_null())
            .count(db)
            .await
            .model_err()?;

        if valid_count as usize != permission_ids.len() {
            return Err(loco_rs::Error::CustomError(
                StatusCode::BAD_REQUEST,
                ErrorDetail::new(
                    "role.invalid_permission_ids",
                    "One or more permission_ids do not exist",
                ),
            ));
        }
    }

    // Delegation check: non-super-admin can only assign permissions they hold
    if !is_super_admin && !permission_ids.is_empty() {
        let actor_scope =
            crate::services::permission_service::get_user_effective_permission_ids(
                db,
                actor_user_id,
                tenant_id,
            )
            .await?;
        let out_of_scope: Vec<&Uuid> = permission_ids
            .iter()
            .filter(|id| !actor_scope.contains(id))
            .collect();
        if !out_of_scope.is_empty() {
            return Err(crate::views::errors::role::err_out_of_scope_permissions());
        }
    }

    let role = roles_model::Model::find_by_id_and_tenant(db, role_id, tenant_id).await?;
    let old_permission_ids: Vec<String> =
        permissions_model::Model::find_role_permission_ids(db, role_id, tenant_id)
            .await
            .model_err()?
            .into_iter()
            .map(|id| id.to_string())
            .collect();
    permissions_model::Model::sync_role_permissions(
        db,
        tenant_id,
        role_id,
        permission_ids.clone(),
    )
    .await?;

    let obj_acts =
        permissions_model::Model::find_role_permission_obj_acts(db, role_id, tenant_id)
            .await?;

    casbin_service::sync_role_permissions(
        enforcer,
        &role.code,
        &tenant_id.to_string(),
        &obj_acts,
    )
    .await
    .map_err(|e| {
        let desc = format!("Failed to sync casbin: {e}");
        Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("casbin.sync_failed", &desc),
        )
    })?;

    let new_permission_ids: Vec<String> =
        permission_ids.iter().map(|id| id.to_string()).collect();
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "role_permissions",
        &role_id.to_string(),
        Some(&serde_json::json!({"permissionIds": old_permission_ids})),
        Some(&serde_json::json!({"permissionIds": new_permission_ids})),
    )
    .await?;

    Ok(())
}

#[tracing::instrument(skip_all)]
pub async fn get_role_menu_ids<C: ConnectionTrait>(
    db: &C,
    role_id: Uuid,
    tenant_id: Uuid,
) -> loco_rs::Result<Vec<String>> {
    let records = role_menus::Entity::find()
        .filter(role_menus::Column::RoleId.eq(role_id))
        .filter(role_menus::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .model_err()?;

    Ok(records
        .into_iter()
        .map(|record| record.sys_menu_id.to_string())
        .collect())
}

#[tracing::instrument(skip_all)]
pub async fn sync_role_menus(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    role_id: Uuid,
    sys_menu_ids: Vec<Uuid>,
    actor_user_id: Uuid,
    is_super_admin: bool,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    // Deduplicate menu IDs
    let sys_menu_ids: Vec<Uuid> = {
        let mut seen = HashSet::new();
        sys_menu_ids
            .into_iter()
            .filter(|id| seen.insert(*id))
            .collect()
    };

    // Delegation check: non-super-admin can only assign menus they hold
    if !is_super_admin && !sys_menu_ids.is_empty() {
        let mut actor_scope =
            crate::services::tenant_menu_service::get_user_effective_menu_ids(
                db,
                actor_user_id,
                tenant_id,
            )
            .await?;

        // Expand actor_scope to include ancestor directories.
        // get_assignable_menus shows parent directories in the tree so the
        // structure stays valid.  With checkStrictly=false, the frontend
        // auto-checks parents when all children are checked, so the PUT
        // payload can contain parent IDs that aren't in the raw role_menus.
        let all_menus = sys_menus::Model::find_active(db).await?;
        let menu_map: HashMap<Uuid, &sys_menus::Model> =
            all_menus.iter().map(|m| (m.id, m)).collect();
        let base_ids: Vec<Uuid> = actor_scope.iter().copied().collect();
        for id in base_ids {
            let mut current = menu_map.get(&id).and_then(|m| m.parent_id);
            while let Some(pid) = current {
                if !actor_scope.insert(pid) {
                    break;
                }
                current = menu_map.get(&pid).and_then(|m| m.parent_id);
            }
        }

        let out_of_scope: Vec<&Uuid> = sys_menu_ids
            .iter()
            .filter(|id| !actor_scope.contains(id))
            .collect();
        if !out_of_scope.is_empty() {
            return Err(crate::views::errors::role::err_out_of_scope_menus());
        }
    }

    let old_menu_ids: Vec<String> = role_menus::Entity::find()
        .filter(role_menus::Column::RoleId.eq(role_id))
        .filter(role_menus::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .model_err()?
        .into_iter()
        .map(|rm| rm.sys_menu_id.to_string())
        .collect();

    let txn = db.begin().await.model_err()?;

    role_menus::Entity::delete_many()
        .filter(role_menus::Column::TenantId.eq(tenant_id))
        .filter(role_menus::Column::RoleId.eq(role_id))
        .exec(&txn)
        .await
        .model_err()?;

    for sys_menu_id in &sys_menu_ids {
        role_menus::ActiveModel {
            tenant_id: ActiveValue::Set(tenant_id),
            role_id: ActiveValue::Set(role_id),
            sys_menu_id: ActiveValue::Set(*sys_menu_id),
        }
        .insert(&txn)
        .await
        .model_err()?;
    }

    txn.commit().await.model_err()?;

    let new_menu_ids: Vec<String> =
        sys_menu_ids.iter().map(|id| id.to_string()).collect();
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "role_menus",
        &role_id.to_string(),
        Some(&serde_json::json!({"sysMenuIds": old_menu_ids})),
        Some(&serde_json::json!({"sysMenuIds": new_menu_ids})),
    )
    .await?;

    Ok(())
}
