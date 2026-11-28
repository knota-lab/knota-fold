use axum::http::StatusCode;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use loco_rs::{app::AppContext, controller::ErrorDetail, prelude::*};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::ConfigExt;
use crate::modules::api_keys::models::api_keys;
use crate::modules::roles::models::roles;
use crate::modules::tenants::models::tenants;

const EXCHANGE_TOKEN_PREFIX: &str = "ex_";
const SUPER_ADMIN_ROLE: &str = "SUPER_ADMIN";

/// Short-lived session token returned by the API-key exchange endpoint.
pub const API_KEY_SESSION_TTL_SECS: u64 = 300;
/// Claim marker for JWTs minted from an API key.
pub const API_KEY_SESSION_AUTH_TYPE: &str = "api_key";

#[derive(Debug, Clone)]
pub struct GeneratedKey {
    pub id: Uuid,
    pub plain_key: String,
    pub prefix: String,
}

impl GeneratedKey {
    #[must_use]
    pub fn generate(env_prefix: &str) -> Self {
        Self::generate_with_prefix(env_prefix, 32)
    }

    #[must_use]
    pub fn hash_key(plain: &str) -> String {
        let digest = Sha256::digest(plain.as_bytes());
        digest.iter().fold(String::new(), |mut acc, b| {
            use std::fmt::Write;
            let _ = write!(acc, "{b:02x}");
            acc
        })
    }

    #[must_use]
    pub fn generate_with_bytes(env_prefix: &str, secret_bytes: usize) -> Self {
        Self::generate_with_prefix(env_prefix, secret_bytes)
    }

    fn generate_with_prefix(prefix: &str, secret_bytes: usize) -> Self {
        let id = crate::utils::id::generate_id();
        let secret = random_secret(secret_bytes);
        let plain_key = format!("{prefix}{}", URL_SAFE_NO_PAD.encode(secret));
        let suffix: String = plain_key
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        Self {
            id,
            prefix: format!("{prefix}****{suffix}"),
            plain_key,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApiKeyIdentity {
    pub api_key_id: Uuid,
    pub tenant_id: Uuid,
    pub tenant_code: String,
    pub tenant_name: String,
    pub created_by: Uuid,
    pub role_code: String,
    pub key_name: String,
}

/// Returns true when the JWT claims were minted by the API-key exchange flow.
#[must_use]
pub fn is_api_key_session(claims: &Map<String, Value>) -> bool {
    claims.get("auth_type").and_then(Value::as_str) == Some(API_KEY_SESSION_AUTH_TYPE)
}

/// Rebuild the API-key identity from signed session claims without touching
/// the database. Returns `None` for malformed or non-API-key claims.
#[must_use]
pub fn identity_from_session_claims(
    claims: &Map<String, Value>,
    subject: &str,
) -> Option<ApiKeyIdentity> {
    if !is_api_key_session(claims) {
        return None;
    }

    let api_key_id = Uuid::parse_str(claims.get("api_key_id")?.as_str()?).ok()?;
    let tenant_id = Uuid::parse_str(claims.get("tenant_id")?.as_str()?).ok()?;
    let created_by = Uuid::parse_str(subject).ok()?;
    let role_code = claims.get("role")?.as_str()?.to_string();
    if role_code == SUPER_ADMIN_ROLE {
        return None;
    }

    Some(ApiKeyIdentity {
        api_key_id,
        tenant_id,
        tenant_code: claims.get("tenant_code")?.as_str()?.to_string(),
        tenant_name: claims
            .get("tenant_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        created_by,
        role_code,
        key_name: claims
            .get("key_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

impl ApiKeyIdentity {
    pub async fn authenticate(db: &DatabaseConnection, plain_key: &str) -> Result<Self> {
        let hash = GeneratedKey::hash_key(plain_key);
        let api_key = api_keys::Model::find_by_hash(db, &hash)
            .await?
            .ok_or_else(crate::views::errors::api_key::err_invalid)?;

        if !api_key.is_valid() {
            return Err(crate::views::errors::api_key::err_invalid());
        }

        let tenant = tenants::Model::find_by_id(db, api_key.tenant_id).await?;
        if tenant.status != "active" {
            return Err(crate::views::errors::api_key::err_tenant_inactive());
        }

        let role =
            roles::Model::find_by_id_and_tenant(db, api_key.role_id, api_key.tenant_id)
                .await?;
        if role.code == SUPER_ADMIN_ROLE {
            return Err(crate::views::errors::api_key::err_super_admin_not_allowed());
        }

        api_keys::Model::touch_last_used(db, api_key.id).await?;

        Ok(Self {
            api_key_id: api_key.id,
            tenant_id: api_key.tenant_id,
            tenant_code: tenant.code,
            tenant_name: tenant.name,
            created_by: api_key.created_by,
            role_code: role.code,
            key_name: api_key.name,
        })
    }
}

#[must_use]
pub fn generate_exchange_token() -> GeneratedKey {
    GeneratedKey::generate_with_prefix(EXCHANGE_TOKEN_PREFIX, 32)
}

pub fn api_key_settings(ctx: &AppContext) -> Result<crate::config::ApiKeyConfig> {
    ctx.config
        .typed_settings()
        .map_err(|e| {
            let desc = format!("Failed to parse apiKey settings: {e}");
            Error::CustomError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetail::new("api_key.config_parse_error", &desc),
            )
        })?
        .map(|s| s.api_key)
        .ok_or_else(|| {
            Error::CustomError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetail::new("api_key.config_missing", "apiKey 配置缺失"),
            )
        })
}

fn random_secret(secret_bytes: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(secret_bytes);
    while bytes.len() < secret_bytes {
        bytes.extend_from_slice(Uuid::new_v4().as_bytes());
    }
    bytes.truncate(secret_bytes);
    bytes
}
