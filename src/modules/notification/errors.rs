use axum::response::Response;
use loco_rs::prelude::*;

/// Notification module error codes.
///
/// All codes follow the `notification.<detail>` format.
/// The frontend errorMap can use these for i18n translation.
#[derive(Debug)]
pub enum NotificationError {
    /// Notification not found.
    NotFound,
    /// Only `super_admin` can send platform notifications.
    PlatformRequiresSuperAdmin,
    /// Cannot notify yourself.
    CannotNotifySelf,
    /// Notification already revoked.
    AlreadyRevoked,
    /// No roles selected for `tenant_role` notification.
    NoRolesSelected,
    /// Unsupported notification type.
    UnsupportedType,
    /// Cross-tenant operation forbidden.
    Forbidden,
}

impl std::fmt::Display for NotificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "通知不存在"),
            Self::PlatformRequiresSuperAdmin => {
                write!(f, "只有超级管理员可以发送平台通知")
            }
            Self::CannotNotifySelf => write!(f, "不能发送通知给自己"),
            Self::AlreadyRevoked => write!(f, "通知已被撤回"),
            Self::NoRolesSelected => write!(f, "请至少选择一个角色"),
            Self::UnsupportedType => write!(f, "不支持的通知类型"),
            Self::Forbidden => write!(f, "无权操作此通知"),
        }
    }
}

impl std::error::Error for NotificationError {}

impl NotificationError {
    /// Convert to a `loco_rs::Error` using module-local `error_info` constants.
    ///
    /// Delegates to `from_info()` for unified location tracking via `#[track_caller]`.
    #[track_caller]
    #[must_use]
    pub fn to_err(&self) -> loco_rs::Error {
        use crate::modules::notification::error_info as ei;
        let info = match self {
            Self::NotFound => ei::NOT_FOUND,
            Self::PlatformRequiresSuperAdmin => ei::PLATFORM_REQUIRES_SUPER_ADMIN,
            Self::CannotNotifySelf => ei::CANNOT_NOTIFY_SELF,
            Self::AlreadyRevoked => ei::ALREADY_REVOKED,
            Self::NoRolesSelected => ei::NO_ROLES_SELECTED,
            Self::UnsupportedType => ei::UNSUPPORTED_TYPE,
            Self::Forbidden => ei::FORBIDDEN,
        };
        crate::views::errors::from_info(info)
    }

    pub fn to_response(&self) -> Result<Response> {
        Err(self.to_err())
    }
}
