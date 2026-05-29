use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::models::_entities::sys_configs;

// ── Query params ──

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalConfigListParams {
    pub category: Option<String>,
    pub prefix: Option<String>,
    pub page: Option<u64>,
    pub page_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantConfigListParams {
    pub category: Option<String>,
    pub prefix: Option<String>,
}

// ── Request bodies ──

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGlobalConfigRequest {
    pub key: String,
    pub value: String,
    pub value_type: String,
    pub category: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGlobalConfigRequest {
    pub value: String,
    pub label: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertTenantConfigRequest {
    /// Only the value is mutable; value_type is inherited from the global config.
    pub value: String,
}

// ── Response DTOs ──

/// Full config record — returned by global config management endpoints.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SysConfigResponse {
    pub id: String,
    pub key: String,
    pub value: String,
    pub value_type: String,
    pub category: String,
    pub scope: String,
    pub tenant_id: Option<String>,
    pub label: String,
    pub description: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&sys_configs::Model> for SysConfigResponse {
    fn from(m: &sys_configs::Model) -> Self {
        Self {
            id: m.id.to_string(),
            key: m.key.clone(),
            value: m.value.clone(),
            value_type: m.value_type.clone(),
            category: m.category.clone(),
            scope: m.scope.clone(),
            tenant_id: m.tenant_id.map(|id| id.to_string()),
            label: m.label.clone(),
            description: m.description.clone(),
            updated_by: m.updated_by.map(|id| id.to_string()),
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

impl SysConfigResponse {
    pub fn from_model(m: &sys_configs::Model) -> Self {
        m.into()
    }
}

/// Paginated list of global configs.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SysConfigListResponse {
    pub items: Vec<SysConfigResponse>,
    pub total_items: u64,
    pub total_pages: u64,
    pub page: u64,
    pub page_size: u64,
}

/// Slim entry for the frontend bulk resolved endpoint.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedConfigSlim {
    pub value: String,
    pub value_type: String,
    pub source: String,
}

/// Full resolved config entry including layers — for the debug/admin single-key endpoint.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedConfigDetail {
    pub key: String,
    pub resolved_value: String,
    pub value_type: String,
    pub source: String,
    pub layers: ConfigLayers,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConfigLayers {
    pub tenant: Option<String>,
    pub global: Option<String>,
}

/// Bulk resolved response — used by the frontend init endpoint.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedConfigsResponse {
    pub configs: HashMap<String, ResolvedConfigSlim>,
}

/// Response after tenant upsert — returns resolved view of the updated key.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantUpsertResponse {
    pub key: String,
    pub resolved_value: String,
    pub value_type: String,
    pub source: String,
    pub layers: ConfigLayers,
}
