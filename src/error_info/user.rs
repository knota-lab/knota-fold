use crate::error_info::ErrorInfo;

pub const CROSS_TENANT: ErrorInfo =
    ErrorInfo::Forbidden("user.cross_tenant", "不能跨租户操作");
pub const CANNOT_DISABLE_SELF: ErrorInfo =
    ErrorInfo::BadRequest("user.cannot_disable_self", "管理员不能禁用自己的帐户");
pub const CANNOT_REMOVE_SELF_ROLE: ErrorInfo =
    ErrorInfo::BadRequest("user.cannot_remove_self_role", "不能移除自己的角色");
pub const LAST_SUPER_ADMIN: ErrorInfo = ErrorInfo::BadRequest(
    "user.last_super_admin",
    "系统中仅剩一个超级管理员，不可禁用",
);
