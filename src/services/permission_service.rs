use axum::http::StatusCode;
use std::collections::{HashMap, HashSet};

use loco_rs::controller::ErrorDetail;
use loco_rs::prelude::model::query;
use loco_rs::prelude::*;
use sea_orm::{ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::models::_entities::{permissions, role_permissions, roles, user_roles};
use crate::models::permissions as permissions_model;
use crate::services::casbin_service::{self, SharedEnforcer};
use crate::utils::error::IntoModelResult;
use crate::views::pagination::PaginatedResponse;
use crate::views::permissions::{
    AssignablePermissionsResponse, CreatePermissionRequest, PermissionResponse,
    PermissionWithMetadataResponse, PermissionsWithMetadataResponse, RouteMetadataItem,
    SyncPermissionItem, UpdatePermissionRequest,
};

pub async fn list_permissions(
    db: &DatabaseConnection,
    pagination: &query::PaginationQuery,
) -> loco_rs::Result<PaginatedResponse<PermissionResponse>> {
    let base_query =
        permissions::Entity::find().filter(permissions::Column::DeletedAt.is_null());
    let page_response = query::paginate(db, base_query, None, pagination).await?;

    Ok(PaginatedResponse::from_page_response(
        &page_response,
        pagination,
        PermissionResponse::from_model,
    ))
}

pub async fn create_permission(
    db: &DatabaseConnection,
    user_id: Uuid,
    params: &CreatePermissionRequest,
) -> Result<permissions::Model> {
    let mut am = permissions_model::ActiveModel {
        name: ActiveValue::Set(params.name.clone()),
        code: ActiveValue::Set(params.code.clone()),
        obj: ActiveValue::Set(params.obj.clone()),
        act: ActiveValue::Set(params.act.clone()),
        permission_type: ActiveValue::Set(params.permission_type.clone()),
        ..Default::default()
    };

    if let Some(is_system) = params.is_system {
        am.is_system = ActiveValue::Set(is_system);
    }

    permissions_model::Model::create_permission(db, am, user_id)
        .await
        .model_err()
}

pub async fn update_permission(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
    params: &UpdatePermissionRequest,
) -> Result<permissions::Model> {
    let mut am = permissions_model::ActiveModel {
        version: ActiveValue::Set(params.version),
        ..Default::default()
    };

    if let Some(name) = &params.name {
        am.name = ActiveValue::Set(name.clone());
    }
    if let Some(code) = &params.code {
        am.code = ActiveValue::Set(code.clone());
    }
    if let Some(obj) = &params.obj {
        am.obj = ActiveValue::Set(obj.clone());
    }
    if let Some(act) = &params.act {
        am.act = ActiveValue::Set(act.clone());
    }
    if let Some(permission_type) = &params.permission_type {
        am.permission_type = ActiveValue::Set(permission_type.clone());
    }
    if let Some(is_system) = params.is_system {
        am.is_system = ActiveValue::Set(is_system);
    }

    permissions_model::Model::update_with_version(db, id, am, user_id)
        .await
        .model_err()
}

pub async fn delete_permission(
    db: &DatabaseConnection,
    enforcer: &SharedEnforcer,
    id: Uuid,
) -> loco_rs::Result<()> {
    let permission = permissions_model::Model::find_by_id(db, id).await?;
    permissions_model::Model::soft_delete(db, id).await?;

    casbin_service::remove_permission_policies(
        enforcer,
        &permission.obj,
        &permission.act,
    )
    .await
    .map_err(|e| {
        let desc = format!("Failed to sync casbin: {e}");
        Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("casbin.sync_failed", &desc),
        )
    })?;

    Ok(())
}

/// Batch sync permissions from route metadata.
/// For each (path, method) pair:
///   - if an active permission exists → skip
///   - if a soft-deleted permission exists → restore it (upsert)
///   - otherwise → create a new one
///
/// Permissions are global — no tenant isolation.
pub async fn sync_permissions(
    db: &DatabaseConnection,
    user_id: Uuid,
    items: &[SyncPermissionItem],
) -> loco_rs::Result<Vec<permissions::Model>> {
    let all = permissions_model::Model::find_all_including_deleted(db).await?;

    let mut active_keys: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    let mut deleted_map: std::collections::HashMap<(String, String), Uuid> =
        std::collections::HashMap::new();

    for p in &all {
        let key = (p.obj.clone(), p.act.clone());
        if p.deleted_at.is_none() {
            active_keys.insert(key);
        } else {
            deleted_map.insert(key, p.id);
        }
    }

    let mut created = Vec::new();
    for item in items {
        let method_upper = item.method.to_uppercase();
        // Strip trailing slash for consistent matching
        let path = if item.path.len() > 1 && item.path.ends_with('/') {
            item.path[..item.path.len() - 1].to_string()
        } else {
            item.path.clone()
        };

        let key = (path.clone(), method_upper.clone());
        if active_keys.contains(&key) {
            continue;
        }

        let code = format!("{method_upper}:{path}");

        if let Some(&id) = deleted_map.get(&key) {
            // Restore the soft-deleted record
            let model = permissions_model::Model::restore(db, id, code, user_id).await?;
            created.push(model);
        } else {
            // Create new
            let am = permissions_model::ActiveModel {
                name: ActiveValue::Set(String::new()),
                code: ActiveValue::Set(code),
                obj: ActiveValue::Set(path),
                act: ActiveValue::Set(method_upper),
                permission_type: ActiveValue::Set("api".to_string()),
                is_system: ActiveValue::Set(false),
                ..Default::default()
            };

            let model =
                permissions_model::Model::create_permission(db, am, user_id).await?;
            created.push(model);
        }
    }

    Ok(created)
}

/// Build a lookup map from `OpenAPI` spec: (path, METHOD) → (tag, description)
#[must_use]
pub fn build_route_metadata_map(
    openapi: &utoipa::openapi::OpenApi,
) -> HashMap<(String, String), (String, String)> {
    let mut map = HashMap::new();

    for (path_str, path_item) in &openapi.paths.paths {
        let methods: Vec<(&str, Option<&utoipa::openapi::path::Operation>)> = vec![
            ("GET", path_item.get.as_ref()),
            ("POST", path_item.post.as_ref()),
            ("PUT", path_item.put.as_ref()),
            ("DELETE", path_item.delete.as_ref()),
            ("PATCH", path_item.patch.as_ref()),
            ("HEAD", path_item.head.as_ref()),
            ("OPTIONS", path_item.options.as_ref()),
            ("TRACE", path_item.trace.as_ref()),
        ];

        for (http_method, operation) in methods {
            if let Some(op) = operation {
                let tag = op
                    .tags
                    .as_ref()
                    .and_then(|t| t.first())
                    .cloned()
                    .unwrap_or_default();
                let description = op.description.clone().unwrap_or_default();
                map.insert(
                    (path_str.clone(), http_method.to_string()),
                    (tag, description),
                );
            }
        }
    }

    map
}

/// Get all active permissions with `OpenAPI` metadata merged.
pub async fn get_permissions_with_metadata(
    db: &DatabaseConnection,
    openapi: &utoipa::openapi::OpenApi,
) -> loco_rs::Result<Vec<PermissionWithMetadataResponse>> {
    let all_permissions = permissions_model::Model::find_all(db).await?;
    let metadata_map = build_route_metadata_map(openapi);

    let results = all_permissions
        .iter()
        .map(|p| {
            let (tag, description) = metadata_map
                .get(&(p.obj.clone(), p.act.clone()))
                .cloned()
                .unwrap_or_default();
            PermissionWithMetadataResponse::from_model(p, tag, description)
        })
        .collect();

    Ok(results)
}

/// Get all permissions with metadata PLUS any `OpenAPI` routes that have no
/// matching permission in the database.  Used by the `ApiScope` management page.
pub async fn get_permissions_with_metadata_and_unmatched(
    db: &DatabaseConnection,
    openapi: &utoipa::openapi::OpenApi,
) -> loco_rs::Result<PermissionsWithMetadataResponse> {
    let all_permissions = permissions_model::Model::find_all(db).await?;
    let metadata_map = build_route_metadata_map(openapi);

    let mut matched_routes = std::collections::HashSet::new();

    let permissions: Vec<PermissionWithMetadataResponse> = all_permissions
        .iter()
        .map(|p| {
            let key = (p.obj.clone(), p.act.clone());
            let (tag, description) = metadata_map.get(&key).cloned().unwrap_or_default();
            if metadata_map.contains_key(&key) {
                matched_routes.insert(key);
            }
            PermissionWithMetadataResponse::from_model(p, tag, description)
        })
        .collect();

    let unmatched_routes: Vec<RouteMetadataItem> = metadata_map
        .iter()
        .filter(|(key, _)| !matched_routes.contains(key))
        .map(|((path, method), (tag, description))| RouteMetadataItem {
            path: path.clone(),
            method: method.clone(),
            tag: tag.clone(),
            description: description.clone(),
        })
        .collect();

    Ok(PermissionsWithMetadataResponse {
        permissions,
        unmatched_routes,
    })
}

/// Get all permissions with metadata + the specified role's currently assigned
/// permission IDs.
///
/// Used by the role permission assignment dialog. When `is_super_admin` is
/// false, filters permissions to only those the actor holds.
pub async fn get_assignable_permissions(
    db: &DatabaseConnection,
    openapi: &utoipa::openapi::OpenApi,
    role_id: Uuid,
    tenant_id: Uuid,
    actor_user_id: Uuid,
    is_super_admin: bool,
) -> loco_rs::Result<AssignablePermissionsResponse> {
    let mut permissions = get_permissions_with_metadata(db, openapi).await?;

    if !is_super_admin {
        let actor_scope =
            get_user_effective_permission_ids(db, actor_user_id, tenant_id).await?;
        permissions.retain(|p| {
            p.id.parse::<Uuid>()
                .map(|id| actor_scope.contains(&id))
                .unwrap_or(false)
        });
    }

    let assigned_ids =
        permissions_model::Model::find_role_permission_ids(db, role_id, tenant_id)
            .await?;

    Ok(AssignablePermissionsResponse {
        permissions,
        assigned_permission_ids: assigned_ids
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
    })
}

/// Compute the union of all permission IDs across all the user's roles in a given tenant.
/// Used for delegation checks: a user can only assign permissions they themselves hold.
pub async fn get_user_effective_permission_ids<C: sea_orm::ConnectionTrait>(
    db: &C,
    user_id: Uuid,
    tenant_id: Uuid,
) -> loco_rs::Result<HashSet<Uuid>> {
    let role_ids: Vec<Uuid> = user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(user_roles::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .model_err()?
        .into_iter()
        .map(|ur| ur.role_id)
        .collect();

    if role_ids.is_empty() {
        return Ok(HashSet::new());
    }

    // Filter to only active roles
    let role_ids: Vec<Uuid> = roles::Entity::find()
        .filter(roles::Column::Id.is_in(role_ids))
        .filter(roles::Column::Status.eq("active"))
        .all(db)
        .await
        .model_err()?
        .into_iter()
        .map(|r| r.id)
        .collect();

    if role_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let raw_perm_ids: Vec<Uuid> = role_permissions::Entity::find()
        .filter(role_permissions::Column::RoleId.is_in(role_ids))
        .filter(role_permissions::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .model_err()?
        .into_iter()
        .map(|rp| rp.permission_id)
        .collect();

    if raw_perm_ids.is_empty() {
        return Ok(HashSet::new());
    }

    // Filter out soft-deleted permissions
    let perm_ids: HashSet<Uuid> = permissions::Entity::find()
        .filter(permissions::Column::Id.is_in(raw_perm_ids))
        .filter(permissions::Column::DeletedAt.is_null())
        .all(db)
        .await
        .model_err()?
        .into_iter()
        .map(|p| p.id)
        .collect();

    Ok(perm_ids)
}
