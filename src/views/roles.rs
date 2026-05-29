use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::_entities::roles;
use crate::views::menus::MergedMenuTreeResponse;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleResponse {
    pub id: String,
    pub tenant_code: String,
    pub tenant_name: String,
    pub name: String,
    pub code: String,
    pub parent_id: Option<String>,
    pub is_system: bool,
    pub description: Option<String>,
    pub version: i32,
    pub status: String,
}

impl RoleResponse {
    pub fn from_model(m: &roles::Model, tenant_code: &str, tenant_name: &str) -> Self {
        Self {
            id: m.id.to_string(),
            tenant_code: tenant_code.to_string(),
            tenant_name: tenant_name.to_string(),
            name: m.name.clone(),
            code: m.code.clone(),
            parent_id: m.parent_id.map(|p| p.to_string()),
            is_system: m.is_system,
            description: m.description.clone(),
            version: m.version,
            status: m.status.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoleRequest {
    pub name: String,
    pub code: String,
    pub parent_id: Option<Uuid>,
    pub is_system: Option<bool>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub parent_id: Option<Option<Uuid>>,
    pub is_system: Option<bool>,
    pub description: Option<Option<String>>,
    pub version: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncUserRolesRequest {
    pub role_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RolePermissionIdsResponse {
    pub permission_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRolePermissionsRequest {
    pub permission_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleMenuIdsResponse {
    pub sys_menu_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRoleMenusRequest {
    pub sys_menu_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleRoleStatusRequest {
    pub status: String,
}

/// Query parameters for GET /api/roles with optional tenant_code filter
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleListParams {
    pub page: u64,
    pub page_size: u64,
    pub tenant_code: Option<String>,
    pub name: Option<String>,
    pub status: Option<String>,
}

/// Combined response for role menu assignment dialog
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignableMenusResponse {
    pub menus: Vec<MergedMenuTreeResponse>,
    pub assigned_menu_ids: Vec<String>,
}
