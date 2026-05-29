use crate::error_info::ErrorInfo;

pub const TYPE_FORBIDDEN: ErrorInfo =
    ErrorInfo::Forbidden("dict.type_forbidden", "无权操作此字典类型");
pub const ITEM_FORBIDDEN: ErrorInfo =
    ErrorInfo::Forbidden("dict.item_forbidden", "无权操作此字典项");
pub const TYPE_CODE_CONFLICT: ErrorInfo =
    ErrorInfo::BadRequest("dict.type_code_conflict", "字典类型编码与系统字典冲突");
pub const TYPE_OVERRIDE_ONLY: ErrorInfo =
    ErrorInfo::BadRequest("dict.type_override_only", "只能重置租户的覆盖字典类型");
pub const ITEM_OVERRIDE_ONLY: ErrorInfo =
    ErrorInfo::BadRequest("dict.item_override_only", "只能重置租户的覆盖字典项");
pub const TYPE_SYSTEM_ONLY: ErrorInfo =
    ErrorInfo::BadRequest("dict.type_system_only", "超级管理员只能编辑系统字典类型");
pub const TYPE_SYSTEM_ONLY_TOGGLE: ErrorInfo = ErrorInfo::BadRequest(
    "dict.type_system_only_toggle",
    "超级管理员只能操作系统字典类型",
);
pub const ITEM_SYSTEM_ONLY: ErrorInfo =
    ErrorInfo::BadRequest("dict.item_system_only", "超级管理员只能编辑系统字典项");
pub const ITEM_SYSTEM_ONLY_TOGGLE: ErrorInfo = ErrorInfo::BadRequest(
    "dict.item_system_only_toggle",
    "超级管理员只能操作系统字典项",
);
pub const TYPE_CODE_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("dict.type_code_empty", "字典类型编码不能为空");
pub const TYPE_NAME_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("dict.type_name_empty", "字典类型名称不能为空");
pub const ITEM_CODE_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("dict.item_code_empty", "字典项编码不能为空");
pub const ITEM_NAME_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("dict.item_name_empty", "字典项名称不能为空");
