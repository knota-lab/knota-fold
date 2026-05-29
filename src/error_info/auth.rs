use crate::error_info::ErrorInfo;

pub const INVALID_CREDENTIALS: ErrorInfo =
    ErrorInfo::Unauthorized("auth.invalid_credentials", "邮箱或密码错误");
pub const INVALID_TOKEN: ErrorInfo =
    ErrorInfo::Unauthorized("auth.invalid_token", "无效的认证令牌");
pub const INVALID_USER_ID: ErrorInfo =
    ErrorInfo::Unauthorized("auth.invalid_user_id", "令牌中的用户ID无效");
pub const PASSWORD_CHANGED: ErrorInfo =
    ErrorInfo::Unauthorized("auth.password_changed", "密码已修改，请重新登录");
pub const MISSING_TENANT_CODE: ErrorInfo =
    ErrorInfo::Unauthorized("auth.missing_tenant_code", "令牌中缺少租户编码");
pub const TENANT_NOT_FOUND: ErrorInfo =
    ErrorInfo::Unauthorized("auth.tenant_not_found", "租户不存在");
pub const TENANT_INACTIVE: ErrorInfo =
    ErrorInfo::Unauthorized("auth.tenant_inactive", "租户已停用");
pub const MISSING_AUTH_HEADER: ErrorInfo =
    ErrorInfo::Unauthorized("auth.missing_auth_header", "缺少认证请求头");
pub const INVALID_AUTH_HEADER: ErrorInfo =
    ErrorInfo::Unauthorized("auth.invalid_auth_header", "认证请求头格式无效");
pub const EMAIL_REQUIRED: ErrorInfo =
    ErrorInfo::BadRequest("auth.email_required", "邮箱地址是必需的");
