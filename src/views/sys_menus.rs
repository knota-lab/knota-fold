use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SysMenuResponse {
    pub id: String,
    pub parent_id: Option<String>,
    pub code: String,
    pub name: String,
    pub path: Option<String>,
    pub alias: Option<String>,
    pub icon: Option<String>,
    #[serde(rename = "type")]
    pub menu_type: String,
    pub is_cache: bool,
    pub sort_order: i32,
    pub remark: Option<String>,
    pub status: String,
    pub version: i32,
}

impl SysMenuResponse {
    pub fn from_model(m: &crate::models::_entities::sys_menus::Model) -> Self {
        Self {
            id: m.id.to_string(),
            parent_id: m.parent_id.map(|p| p.to_string()),
            code: m.code.clone(),
            name: m.name.clone(),
            path: m.path.clone(),
            alias: m.alias.clone(),
            icon: m.icon.clone(),
            menu_type: m.menu_type.clone(),
            is_cache: m.is_cache,
            sort_order: m.sort_order,
            remark: m.remark.clone(),
            status: m.status.clone(),
            version: m.version,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SysMenuTreeResponse {
    pub id: String,
    pub parent_id: Option<String>,
    pub code: String,
    pub name: String,
    pub path: Option<String>,
    pub alias: Option<String>,
    pub icon: Option<String>,
    #[serde(rename = "type")]
    pub menu_type: String,
    pub is_cache: bool,
    pub sort_order: i32,
    pub remark: Option<String>,
    pub status: String,
    pub version: i32,
    pub children: Vec<SysMenuTreeResponse>,
}

impl SysMenuTreeResponse {
    pub fn from_model(
        m: &crate::models::_entities::sys_menus::Model,
        children: Vec<SysMenuTreeResponse>,
    ) -> Self {
        Self {
            id: m.id.to_string(),
            parent_id: m.parent_id.map(|p| p.to_string()),
            code: m.code.clone(),
            name: m.name.clone(),
            path: m.path.clone(),
            alias: m.alias.clone(),
            icon: m.icon.clone(),
            menu_type: m.menu_type.clone(),
            is_cache: m.is_cache,
            sort_order: m.sort_order,
            remark: m.remark.clone(),
            status: m.status.clone(),
            version: m.version,
            children,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSysMenuRequest {
    pub name: String,
    pub code: String,
    #[serde(rename = "type")]
    pub menu_type: String,
    pub path: Option<String>,
    pub alias: Option<String>,
    pub icon: Option<String>,
    pub parent_id: Option<Uuid>,
    pub is_cache: Option<bool>,
    pub sort_order: Option<i32>,
    pub remark: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSysMenuRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    #[serde(rename = "type")]
    pub menu_type: Option<String>,
    pub path: Option<Option<String>>,
    pub alias: Option<Option<String>>,
    pub icon: Option<Option<String>>,
    pub parent_id: Option<Option<Uuid>>,
    pub is_cache: Option<bool>,
    pub sort_order: Option<i32>,
    pub remark: Option<Option<String>>,
    pub status: Option<String>,
    pub version: i32,
}
