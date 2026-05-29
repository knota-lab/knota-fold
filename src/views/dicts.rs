use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::_entities::{dict_items, dict_types};
use crate::models::dict_items::EffectiveDictItem;
use crate::models::dict_types::EffectiveDictType;

// ── Query params ──

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictItemsQuery {
    pub type_code: String,
}

// ── Toggle status request ──

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleStatusRequest {
    pub version: i32,
}

// ── Dict Type DTOs ──

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictTypeResponse {
    pub id: String,
    pub code: String,
    pub name: String,
    pub status: String,
    pub scope: String,
    pub source_type_id: Option<String>,
    pub is_override: bool,
    pub description: Option<String>,
    pub version: i32,
}

impl DictTypeResponse {
    pub fn from_model(m: &dict_types::Model) -> Self {
        let scope = compute_type_scope(m.tenant_id, m.source_type_id);
        Self {
            id: m.id.to_string(),
            code: m.code.clone(),
            name: m.name.clone(),
            status: m.status.clone(),
            scope: scope.clone(),
            source_type_id: m.source_type_id.map(|id| id.to_string()),
            is_override: scope == "override",
            description: m.description.clone(),
            version: m.version,
        }
    }

    pub fn from_effective(e: &EffectiveDictType) -> Self {
        Self {
            id: e.id.to_string(),
            code: e.code.clone(),
            name: e.name.clone(),
            status: e.status.clone(),
            scope: e.scope.clone(),
            source_type_id: e.source_type_id.map(|id| id.to_string()),
            is_override: e.scope == "override",
            description: e.description.clone(),
            version: e.version,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictItemResponse {
    pub id: String,
    pub dict_type_id: String,
    pub code: String,
    pub name: String,
    pub value: String,
    pub parent_id: Option<String>,
    pub sort_order: i32,
    pub status: String,
    pub scope: String,
    pub source_item_id: Option<String>,
    pub is_override: bool,
    pub description: Option<String>,
    pub version: i32,
}

impl DictItemResponse {
    pub fn from_model(m: &dict_items::Model) -> Self {
        let scope = compute_item_scope(m.tenant_id, m.source_item_id);
        Self {
            id: m.id.to_string(),
            dict_type_id: m.dict_type_id.to_string(),
            code: m.code.clone(),
            name: m.name.clone(),
            value: m.value.clone(),
            parent_id: m.parent_id.map(|p| p.to_string()),
            sort_order: m.sort_order,
            status: m.status.clone(),
            scope: scope.clone(),
            source_item_id: m.source_item_id.map(|id| id.to_string()),
            is_override: scope == "override",
            description: m.description.clone(),
            version: m.version,
        }
    }

    pub fn from_effective(e: &EffectiveDictItem) -> Self {
        Self {
            id: e.id.to_string(),
            dict_type_id: e.dict_type_id.to_string(),
            code: e.code.clone(),
            name: e.name.clone(),
            value: e.value.clone(),
            parent_id: e.parent_id.map(|p| p.to_string()),
            sort_order: e.sort_order,
            status: e.status.clone(),
            scope: e.scope.clone(),
            source_item_id: e.source_item_id.map(|id| id.to_string()),
            is_override: e.scope == "override",
            description: e.description.clone(),
            version: e.version,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictItemTreeResponse {
    pub id: String,
    pub dict_type_id: String,
    pub code: String,
    pub name: String,
    pub value: String,
    pub parent_id: Option<String>,
    pub sort_order: i32,
    pub status: String,
    pub scope: String,
    pub source_item_id: Option<String>,
    pub is_override: bool,
    pub description: Option<String>,
    pub version: i32,
    pub children: Vec<Self>,
}

impl DictItemTreeResponse {
    pub fn from_model(m: &dict_items::Model, children: Vec<Self>) -> Self {
        let scope = compute_item_scope(m.tenant_id, m.source_item_id);
        Self {
            id: m.id.to_string(),
            dict_type_id: m.dict_type_id.to_string(),
            code: m.code.clone(),
            name: m.name.clone(),
            value: m.value.clone(),
            parent_id: m.parent_id.map(|p| p.to_string()),
            sort_order: m.sort_order,
            status: m.status.clone(),
            scope: scope.clone(),
            source_item_id: m.source_item_id.map(|id| id.to_string()),
            is_override: scope == "override",
            description: m.description.clone(),
            version: m.version,
            children,
        }
    }

    pub fn from_effective(e: &EffectiveDictItem, children: Vec<Self>) -> Self {
        Self {
            id: e.id.to_string(),
            dict_type_id: e.dict_type_id.to_string(),
            code: e.code.clone(),
            name: e.name.clone(),
            value: e.value.clone(),
            parent_id: e.parent_id.map(|p| p.to_string()),
            sort_order: e.sort_order,
            status: e.status.clone(),
            scope: e.scope.clone(),
            source_item_id: e.source_item_id.map(|id| id.to_string()),
            is_override: e.scope == "override",
            description: e.description.clone(),
            version: e.version,
            children,
        }
    }
}

// ── Request DTOs ──

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDictTypeRequest {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDictTypeRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub version: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDictItemRequest {
    pub dict_type_id: Uuid,
    pub code: String,
    pub name: String,
    pub value: String,
    pub parent_id: Option<Uuid>,
    pub sort_order: Option<i32>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDictItemRequest {
    pub name: Option<String>,
    pub parent_id: Option<Option<Uuid>>,
    pub sort_order: Option<i32>,
    pub description: Option<Option<String>>,
    pub version: i32,
}

// ── Helpers ──

fn compute_type_scope(tenant_id: Option<Uuid>, source_type_id: Option<Uuid>) -> String {
    match (tenant_id, source_type_id) {
        (None, _) => "system".to_string(),
        (Some(_), Some(_)) => "override".to_string(),
        (Some(_), None) => "tenantOnly".to_string(),
    }
}

fn compute_item_scope(tenant_id: Option<Uuid>, source_item_id: Option<Uuid>) -> String {
    match (tenant_id, source_item_id) {
        (None, _) => "system".to_string(),
        (Some(_), Some(_)) => "override".to_string(),
        (Some(_), None) => "tenantOnly".to_string(),
    }
}
