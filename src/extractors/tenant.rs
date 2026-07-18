use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use loco_rs::app::AppContext;
use loco_rs::{auth::jwt, prelude::*};
use uuid::Uuid;

use crate::models::roles;
use crate::models::tenants;
use crate::services::api_key_service::ApiKeyIdentity;
use crate::services::auth_cache;
use crate::utils::error::IntoModelResult;
use crate::views::errors::auth as auth_err;

const SUPER_ADMIN_ROLE: &str = "SUPER_ADMIN";
const TENANT_ADMIN_ROLE: &str = "TENANT_ADMIN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationType {
    Jwt,
    ApiKey,
}

#[derive(Debug, Clone)]
pub struct TenantContext {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub tenant_code: String,
    pub tenant_name: String,
    pub is_super_admin: bool,
    pub is_tenant_admin: bool,
    pub auth_type: AuthenticationType,
    pub api_key_id: Option<Uuid>,
}

impl TenantContext {
    #[must_use]
    pub const fn tenant_filter(&self) -> Option<Uuid> {
        if self.is_super_admin {
            None
        } else {
            Some(self.tenant_id)
        }
    }

    #[must_use]
    pub const fn is_api_key(&self) -> bool {
        matches!(self.auth_type, AuthenticationType::ApiKey)
    }
}

impl FromRequestParts<AppContext> for TenantContext {
    type Rejection = loco_rs::Error;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppContext,
    ) -> Result<Self, Self::Rejection> {
        if let Some(identity) = parts.extensions.get::<ApiKeyIdentity>() {
            return Ok(Self {
                user_id: identity.created_by,
                tenant_id: identity.tenant_id,
                tenant_code: identity.tenant_code.clone(),
                tenant_name: identity.tenant_name.clone(),
                is_super_admin: false,
                is_tenant_admin: identity.role_code == TENANT_ADMIN_ROLE,
                auth_type: AuthenticationType::ApiKey,
                api_key_id: Some(identity.api_key_id),
            });
        }

        let token = extract_token_from_header(parts)?;
        let jwt_secret = state.config.get_jwt_config()?;
        let claims = jwt::JWT::new(&jwt_secret.secret)
            .validate(&token)
            .map_err(|_| auth_err::err_invalid_token())?;

        let user_claims = claims.claims;

        let user_id = Uuid::parse_str(&user_claims.pid)
            .map_err(|_| auth_err::err_invalid_user_id())?;

        // ── password_iat gate ──────────────────────────────────
        let token_pwd_iat = user_claims
            .claims
            .get("password_iat")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        let db_pwd_iat = auth_cache::get_password_iat(&state.cache, &state.db, user_id)
            .await
            .map_err(|_| auth_err::err_invalid_token())?;

        if token_pwd_iat < db_pwd_iat {
            return Err(auth_err::err_password_changed());
        }

        let tenant_code_claim: &str = user_claims
            .claims
            .get("tenant_code")
            .and_then(|v| v.as_str())
            .ok_or_else(auth_err::err_missing_tenant_code)?;
        let tenant_code = tenant_code_claim.to_string();

        let tenant = tenants::Model::find_by_code(&state.db, &tenant_code)
            .await
            .map_err(|_| auth_err::err_tenant_not_found(&tenant_code))?;

        if tenant.status != "active" {
            return Err(auth_err::err_tenant_inactive(&tenant_code));
        }

        let role_codes =
            roles::Model::find_user_role_codes(&state.db, user_id, tenant.id)
                .await
                .model_err()?;

        let is_super_admin = role_codes.iter().any(|code| code == SUPER_ADMIN_ROLE);
        let is_tenant_admin = role_codes.iter().any(|code| code == TENANT_ADMIN_ROLE);

        Ok(Self {
            user_id,
            tenant_id: tenant.id,
            tenant_code,
            tenant_name: tenant.name.clone(),
            is_super_admin,
            is_tenant_admin,
            auth_type: AuthenticationType::Jwt,
            api_key_id: None,
        })
    }
}

fn extract_token_from_header(parts: &Parts) -> Result<String, loco_rs::Error> {
    let auth_header = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(auth_err::err_missing_auth_header)?;

    auth_header
        .strip_prefix("Bearer ")
        .map(std::string::ToString::to_string)
        .ok_or_else(auth_err::err_invalid_auth_header)
}
