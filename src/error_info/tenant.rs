use crate::error_info::ErrorInfo;

pub const DEFAULT_CANNOT_DISABLE: ErrorInfo =
    ErrorInfo::BadRequest("tenant.default_cannot_disable", "默认租户不允许被禁用");
