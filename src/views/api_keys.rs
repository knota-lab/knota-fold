use serde::{Deserialize, Serialize};

use crate::models::_entities::api_keys;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyResponse {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub role_id: String,
    pub role_name: String,
    pub description: Option<String>,
    pub exchanged_from_id: Option<String>,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub last_used_at: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

impl ApiKeyResponse {
    pub fn from_model(model: &api_keys::Model, role_name: String) -> Self {
        Self {
            id: model.id.to_string(),
            name: model.name.clone(),
            key_prefix: model.key_prefix.clone(),
            role_id: model.role_id.to_string(),
            role_name,
            description: model.description.clone(),
            exchanged_from_id: model.exchanged_from_id.map(|v| v.to_string()),
            expires_at: model.expires_at.map(|v| v.to_rfc3339()),
            revoked_at: model.revoked_at.map(|v| v.to_rfc3339()),
            last_used_at: model.last_used_at.map(|v| v.to_rfc3339()),
            created_by: model.created_by.to_string(),
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApiKeyRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeApiKeyRoleRequest {
    pub role_id: String,
}
