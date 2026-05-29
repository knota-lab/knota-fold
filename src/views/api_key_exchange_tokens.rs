use serde::{Deserialize, Serialize};

use crate::models::_entities::api_key_exchange_tokens;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeTokenResponse {
    pub id: String,
    pub name: String,
    pub token_prefix: String,
    pub role_id: String,
    pub role_name: String,
    pub description: Option<String>,
    pub expires_at: String,
    pub max_usage: i32,
    pub used_count: i32,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

impl ExchangeTokenResponse {
    pub fn from_model(model: &api_key_exchange_tokens::Model, role_name: String) -> Self {
        Self {
            id: model.id.to_string(),
            name: model.name.clone(),
            token_prefix: model.token_prefix.clone(),
            role_id: model.role_id.to_string(),
            role_name,
            description: model.description.clone(),
            expires_at: model.expires_at.to_rfc3339(),
            max_usage: model.max_usage,
            used_count: model.used_count,
            created_by: model.created_by.to_string(),
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateExchangeTokenResponse {
    pub id: String,
    pub name: String,
    pub exchange_token: String,
    pub exchange_url: String,
    pub token_prefix: String,
    pub role_id: String,
    pub role_name: String,
    pub expires_at: String,
    pub max_usage: i32,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateExchangeTokenRequest {
    pub name: String,
    pub role_id: String,
    pub description: Option<String>,
    pub expires_at: Option<String>,
    pub api_key_expires_at: Option<String>,
    pub max_usage: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeRequest {
    pub exchange_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeInfoQuery {
    pub token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeInfoResponse {
    pub tenant_name: String,
    pub role_name: String,
    pub expires_at: String,
    pub already_used: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeKeyResponse {
    pub api_key_id: String,
    pub api_key: String,
    pub key_prefix: String,
    pub role_name: String,
    pub expires_at: Option<String>,
    pub created_at: String,
}
