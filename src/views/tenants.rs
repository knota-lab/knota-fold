use serde::{Deserialize, Serialize};

use crate::models::_entities::tenants;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantResponse {
    pub id: String,
    pub name: String,
    pub code: String,
    pub status: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl TenantResponse {
    #[must_use]
    pub fn from_model(m: &tenants::Model) -> Self {
        Self {
            id: m.id.to_string(),
            name: m.name.clone(),
            code: m.code.clone(),
            status: m.status.clone(),
            description: m.description.clone(),
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTenantRequest {
    pub name: String,
    pub code: String,
    pub status: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTenantRequest {
    pub name: Option<String>,
    pub status: Option<String>,
    pub description: Option<Option<String>>,
}

/// Query parameters for GET /api/tenants with optional search filters
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantListParams {
    pub page: u64,
    #[serde(alias = "page_size")]
    pub page_size: u64,
    pub name: Option<String>,
    pub code: Option<String>,
    pub status: Option<String>,
}
