use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::_entities::sys_role_templates;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleTemplateResponse {
    pub id: String,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

impl RoleTemplateResponse {
    pub fn from_model(m: &sys_role_templates::Model) -> Self {
        Self {
            id: m.id.to_string(),
            code: m.code.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            is_default: m.is_default,
            sort_order: m.sort_order,
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoleTemplateRequest {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: Option<bool>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRoleTemplateRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub is_default: Option<bool>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateMenuIdsResponse {
    pub sys_menu_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncTemplateMenusRequest {
    pub sys_menu_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplatePermissionResponse {
    pub obj: String,
    pub act: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncTemplatePermissionsRequest {
    pub permissions: Vec<TemplatePermissionInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplatePermissionInput {
    pub obj: String,
    pub act: String,
}
