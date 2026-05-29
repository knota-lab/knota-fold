use crate::error_info::ErrorInfo;

pub const NO_TOKEN: ErrorInfo =
    ErrorInfo::Unauthorized("authz.no_token", "未提供认证令牌");
pub const INVALID_TOKEN: ErrorInfo =
    ErrorInfo::Unauthorized("authz.invalid_token", "认证令牌无效");
pub const TOKEN_VALIDATION_FAILED: ErrorInfo =
    ErrorInfo::Unauthorized("authz.token_validation_failed", "令牌验证失败");
pub const PASSWORD_CHANGED: ErrorInfo =
    ErrorInfo::Unauthorized("authz.password_changed", "密码已修改，请重新登录");
pub const USER_LOAD_FAILED: ErrorInfo =
    ErrorInfo::Unauthorized("authz.user_load_failed", "用户信息加载失败");
pub const ACCOUNT_DISABLED: ErrorInfo =
    ErrorInfo::Forbidden("authz.account_disabled", "账号已被禁用");
pub const NO_TENANT_IN_TOKEN: ErrorInfo =
    ErrorInfo::Unauthorized("authz.no_tenant_in_token", "令牌中缺少租户信息");
pub const TENANT_NOT_FOUND: ErrorInfo =
    ErrorInfo::Unauthorized("authz.tenant_not_found", "令牌中租户不存在");
pub const ROLES_LOAD_FAILED: ErrorInfo =
    ErrorInfo::Internal("authz.roles_load_failed", "用户角色加载失败");
pub const ACCESS_DENIED: ErrorInfo =
    ErrorInfo::Forbidden("authz.access_denied", "无权访问，请联系管理员分配对应权限");
pub const API_KEY_INVALID: ErrorInfo =
    ErrorInfo::Unauthorized("authz.api_key_invalid", "API Key 认证失败");
pub const API_KEY_ACCESS_DENIED: ErrorInfo =
    ErrorInfo::Forbidden("authz.api_key_access_denied", "API Key 无权访问");
