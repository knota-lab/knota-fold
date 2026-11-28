use serde::{Deserialize, Serialize};

use crate::models::_entities::permissions;

/// Single route metadata entry from `OpenAPI`
#[derive(Debug, Serialize, Clone)]
pub struct RouteMetadataItem {
    pub path: String,
    pub method: String,
    pub tag: String,
    pub description: String,
}

/// Request body for POST /api/permissions/sync
#[derive(Debug, Deserialize)]
pub struct SyncPermissionsRequest {
    pub items: Vec<SyncPermissionItem>,
}

#[derive(Debug, Deserialize)]
pub struct SyncPermissionItem {
    pub path: String,
    pub method: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionResponse {
    pub id: String,
    pub name: String,
    pub code: String,
    pub obj: String,
    pub act: String,
    #[serde(rename = "type")]
    pub permission_type: String,
    pub is_system: bool,
    pub version: i32,
}

impl PermissionResponse {
    #[must_use]
    pub fn from_model(m: &permissions::Model) -> Self {
        Self {
            id: m.id.to_string(),
            name: m.name.clone(),
            code: m.code.clone(),
            obj: m.obj.clone(),
            act: m.act.clone(),
            permission_type: m.permission_type.clone(),
            is_system: m.is_system,
            version: m.version,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePermissionRequest {
    pub name: String,
    pub code: String,
    pub obj: String,
    pub act: String,
    #[serde(rename = "type")]
    pub permission_type: String,
    pub is_system: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePermissionRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub obj: Option<String>,
    pub act: Option<String>,
    #[serde(rename = "type")]
    pub permission_type: Option<String>,
    pub is_system: Option<bool>,
    pub version: i32,
}

/// Permission with route metadata (tag + description from `OpenAPI` spec)
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionWithMetadataResponse {
    pub id: String,
    pub name: String,
    pub code: String,
    pub obj: String,
    pub act: String,
    #[serde(rename = "type")]
    pub permission_type: String,
    pub is_system: bool,
    pub version: i32,
    pub tag: String,
    pub description: String,
}

impl PermissionWithMetadataResponse {
    #[must_use]
    pub fn from_model(m: &permissions::Model, tag: String, description: String) -> Self {
        Self {
            id: m.id.to_string(),
            name: m.name.clone(),
            code: m.code.clone(),
            obj: m.obj.clone(),
            act: m.act.clone(),
            permission_type: m.permission_type.clone(),
            is_system: m.is_system,
            version: m.version,
            tag,
            description,
        }
    }
}

/// Combined response for role permission assignment dialog
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignablePermissionsResponse {
    pub permissions: Vec<PermissionWithMetadataResponse>,
    pub assigned_permission_ids: Vec<String>,
}

/// Response for GET /api/permissions/with-metadata
/// Returns all permissions enriched with `OpenAPI` metadata,
/// plus any `OpenAPI` routes that have no matching permission yet.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsWithMetadataResponse {
    pub permissions: Vec<PermissionWithMetadataResponse>,
    pub unmatched_routes: Vec<RouteMetadataItem>,
}
