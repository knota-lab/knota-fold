use crate::error_info::ErrorInfo;

pub const NOT_FOUND: ErrorInfo =
    ErrorInfo::NotFound("notification.not_found", "通知不存在");
pub const PLATFORM_REQUIRES_SUPER_ADMIN: ErrorInfo = ErrorInfo::Forbidden(
    "notification.platform_requires_super_admin",
    "只有超级管理员可以发送平台通知",
);
pub const CANNOT_NOTIFY_SELF: ErrorInfo =
    ErrorInfo::BadRequest("notification.cannot_notify_self", "不能发送通知给自己");
pub const ALREADY_REVOKED: ErrorInfo =
    ErrorInfo::BadRequest("notification.already_revoked", "通知已被撤回");
pub const NO_ROLES_SELECTED: ErrorInfo =
    ErrorInfo::BadRequest("notification.no_roles_selected", "请至少选择一个角色");
pub const UNSUPPORTED_TYPE: ErrorInfo =
    ErrorInfo::BadRequest("notification.unsupported_type", "不支持的通知类型");
pub const FORBIDDEN: ErrorInfo =
    ErrorInfo::Forbidden("notification.forbidden", "无权操作此通知");
