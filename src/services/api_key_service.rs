use axum::http::StatusCode;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use loco_rs::{app::AppContext, controller::ErrorDetail, prelude::*};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::ConfigExt;
use crate::models::{api_keys, roles, tenants};

const EXCHANGE_TOKEN_PREFIX: &str = "ex_";
const SUPER_ADMIN_ROLE: &str = "SUPER_ADMIN";

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
    pub role_code: String,
    pub key_name: String,
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
