use serde::{Deserialize, Serialize};

use crate::models::_entities::users;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserResponse {
    pub id: String,
    pub tenant_code: String,
    pub tenant_name: String,
    pub email: String,
    pub name: String,
    pub email_verified_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub status: String,
    /// True iff `login_guard` has an active lock entry for this account.
    /// Always `false` for endpoints that do not look up cache state (e.g.
    /// individual create/update responses) — only the list endpoint is
    /// expected to populate this. Treat as a hint, not a source of truth.
    #[serde(default)]
    pub is_locked: bool,
    /// Absolute epoch second when the lock lifts, if `is_locked == true`.
    pub unlock_at_epoch: Option<i64>,
}

impl UserResponse {
    pub fn from_model(m: &users::Model, tenant_code: &str, tenant_name: &str) -> Self {
        Self {
            id: m.id.to_string(),
            tenant_code: tenant_code.to_string(),
            tenant_name: tenant_name.to_string(),
            email: m.email.clone(),
            name: m.name.clone(),
            email_verified_at: m.email_verified_at.map(|t| t.to_rfc3339()),
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
            status: m.status.clone(),
            is_locked: false,
            unlock_at_epoch: None,
        }
    }

    /// Build a list-response variant that includes lock state from the
    /// login-guard cache. Only `unlock_at_epoch.is_some()` implies an
    /// actually-active lock; the field is `None` for non-locked accounts
    /// so the JSON is small (camelCase `unlockAtEpoch` is `null`).
    pub fn from_model_with_lock(
        m: &users::Model,
        tenant_code: &str,
        tenant_name: &str,
        unlock_at_epoch: Option<i64>,
    ) -> Self {
        let mut r = Self::from_model(m, tenant_code, tenant_name);
        r.is_locked = unlock_at_epoch.is_some();
        r.unlock_at_epoch = unlock_at_epoch;
        r
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserRequest {
    pub email: String,
    pub password: String,
    pub name: String,
    pub tenant_code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRequest {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPasswordRequest {
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleStatusRequest {
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSuperAdminRequest {
    pub email: String,
    pub password: String,
    pub name: String,
}

/// Query parameters for GET /api/users with optional search filters
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserListParams {
    pub page: u64,
    pub page_size: u64,
    pub name: Option<String>,
    pub email: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRolesResponse {
    pub role_ids: Vec<String>,
}
