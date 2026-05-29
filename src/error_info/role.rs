use crate::error_info::ErrorInfo;

pub const CROSS_TENANT: ErrorInfo =
    ErrorInfo::Forbidden("role.cross_tenant", "不能跨租户操作");
pub const NOT_FOUND: ErrorInfo = ErrorInfo::NotFound("role.not_found", "角色不存在");
pub const OUT_OF_SCOPE_PERMISSIONS: ErrorInfo = ErrorInfo::Forbidden(
    "role.out_of_scope_permissions",
    "不能分配超出自身范围的权限",
);
pub const OUT_OF_SCOPE_MENUS: ErrorInfo =
    ErrorInfo::Forbidden("role.out_of_scope_menus", "不能分配超出自身范围的菜单");
