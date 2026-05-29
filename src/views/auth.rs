use serde::{Deserialize, Serialize};

use crate::models::_entities::users;
use crate::services::auth_cache::CachedUserProfile;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub token: String,
    pub id: String,
    pub name: String,
    pub is_verified: bool,
}

impl LoginResponse {
    #[must_use]
    pub fn new(user: &users::Model, token: &str) -> Self {
        Self {
            token: token.to_owned(),
            id: user.id.to_string(),
            name: user.name.clone(),
            is_verified: user.email_verified_at.is_some(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentResponse {
    pub id: String,
    pub name: String,
    pub email: String,
    pub tenant_id: String,
    pub tenant_code: String,
    pub tenant_name: String,
    pub roles: Vec<String>,
    pub is_super_admin: bool,
    pub is_tenant_admin: bool,
    pub avatar_file_id: Option<String>,
}

impl CurrentResponse {
    #[must_use]
    pub fn new(
        user: &users::Model,
        tenant_id: uuid::Uuid,
        tenant_code: &str,
        tenant_name: &str,
        roles: Vec<String>,
        is_super_admin: bool,
        is_tenant_admin: bool,
    ) -> Self {
        Self {
            id: user.id.to_string(),
            name: user.name.clone(),
            email: user.email.clone(),
            tenant_id: tenant_id.to_string(),
            tenant_code: tenant_code.to_string(),
            tenant_name: tenant_name.to_string(),
            roles,
            is_super_admin,
            is_tenant_admin,
            avatar_file_id: user.avatar_file_id.map(|id| id.to_string()),
        }
    }

    #[must_use]
    pub fn from_cached(
        profile: &CachedUserProfile,
        tenant_id: uuid::Uuid,
        tenant_code: &str,
        tenant_name: &str,
        roles: Vec<String>,
        is_super_admin: bool,
        is_tenant_admin: bool,
    ) -> Self {
        Self {
            id: profile.id.to_string(),
            name: profile.name.clone(),
            email: profile.email.clone(),
            tenant_id: tenant_id.to_string(),
            tenant_code: tenant_code.to_string(),
            tenant_name: tenant_name.to_string(),
            roles,
            is_super_admin,
            is_tenant_admin,
            avatar_file_id: profile.avatar_file_id.map(|id| id.to_string()),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProfileRequest {
    pub name: Option<String>,
    pub avatar_file_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// `POST /api/admin/auth/unlock` request body. Identifies the account to
/// be unlocked by primary email (matches `users.email`, normalised to
/// lowercase server-side to align with `login_guard`'s cache keys).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockAccountRequest {
    pub email: String,
}

/// `POST /api/auth/login` request body.
///
/// `captchaToken` + `captchaAnswer` are optional on the wire so the first
/// (un-throttled) attempt does not need a captcha. The login handler decides
/// whether they are required based on prior failures + global config.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub captcha_token: Option<String>,
    #[serde(default)]
    pub captcha_answer: Option<String>,
}

/// `GET /api/auth/captcha` response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptchaResponse {
    /// `data:image/jpeg;base64,...` ready for `<img src>`.
    pub image: String,
    /// Opaque JWT to be echoed back as `captchaToken` on login.
    pub token: String,
    /// Same value as `settings.captcha.ttlSeconds`; lets the UI auto-refresh.
    pub ttl_seconds: u64,
}

/// Structured error body returned by `POST /api/auth/login` on failure so the
/// frontend can react (show captcha, show lock countdown, etc.) without
/// scraping the message string.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginErrorResponse {
    /// Stable machine code: `INVALID_CREDENTIALS` | `CAPTCHA_REQUIRED` |
    /// `CAPTCHA_INVALID` | `ACCOUNT_DISABLED` | `ACCOUNT_LOCKED`.
    pub code: String,
    /// Human-readable message (zh-CN by default).
    pub message: String,
    /// True when the next attempt MUST carry a captcha.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub require_captcha: bool,
    /// Epoch seconds when the account becomes unlocked. Only present for
    /// `ACCOUNT_LOCKED`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unlock_at_epoch: Option<i64>,
}

impl LoginErrorResponse {
    pub fn invalid_credentials(require_captcha: bool) -> Self {
        Self {
            code: "INVALID_CREDENTIALS".to_string(),
            message: "邮箱或密码错误".to_string(),
            require_captcha,
            unlock_at_epoch: None,
        }
    }

    pub fn captcha_required() -> Self {
        Self {
            code: "CAPTCHA_REQUIRED".to_string(),
            message: "请先完成验证码校验".to_string(),
            require_captcha: true,
            unlock_at_epoch: None,
        }
    }

    pub fn captcha_invalid() -> Self {
        Self {
            code: "CAPTCHA_INVALID".to_string(),
            message: "验证码错误或已过期".to_string(),
            require_captcha: true,
            unlock_at_epoch: None,
        }
    }

    pub fn account_disabled() -> Self {
        Self {
            code: "ACCOUNT_DISABLED".to_string(),
            message: "账号已被禁用".to_string(),
            require_captcha: false,
            unlock_at_epoch: None,
        }
    }

    pub fn account_locked(unlock_at_epoch: i64) -> Self {
        Self {
            code: "ACCOUNT_LOCKED".to_string(),
            message: "登录失败次数过多，账号已临时锁定".to_string(),
            require_captcha: true,
            unlock_at_epoch: Some(unlock_at_epoch),
        }
    }
}
