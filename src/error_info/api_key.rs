use crate::error_info::ErrorInfo;

pub const INVALID: ErrorInfo =
    ErrorInfo::Unauthorized("api_key.invalid", "API密钥无效或已失效");
pub const TENANT_INACTIVE: ErrorInfo =
    ErrorInfo::Unauthorized("api_key.tenant_inactive", "API密钥所属租户已停用");
pub const SUPER_ADMIN_NOT_ALLOWED: ErrorInfo = ErrorInfo::Unauthorized(
    "api_key.super_admin_not_allowed",
    "超级管理员角色不允许使用API密钥",
);

// BadRequest
pub const INVALID_EXCHANGE_TOKEN: ErrorInfo =
    ErrorInfo::BadRequest("api_key.invalid_exchange_token", "无效的兑换令牌");
pub const EXPIRES_AT_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("api_key.expires_at_invalid", "无效的过期时间");
pub const API_KEY_EXPIRES_AT_INVALID: ErrorInfo = ErrorInfo::BadRequest(
    "api_key.api_key_expires_at_invalid",
    "无效的 API Key 过期时间",
);
pub const LIMIT_EXCEEDED: ErrorInfo =
    ErrorInfo::BadRequest("api_key.limit_exceeded", "API Key 数量已达上限");
pub const EXCHANGE_TOKEN_EXPIRED: ErrorInfo =
    ErrorInfo::BadRequest("api_key.exchange_token_expired", "兑换令牌已过期");
pub const API_KEY_DISABLED: ErrorInfo =
    ErrorInfo::BadRequest("api_key.api_key_disabled", "API Key 已禁用");
