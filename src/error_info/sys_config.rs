use crate::error_info::ErrorInfo;

pub const SUPER_ADMIN_REQUIRED: ErrorInfo = ErrorInfo::Forbidden(
    "sys_config.super_admin_required",
    "仅超级管理员可管理其他租户的配置",
);

// Validation
pub const VALUE_NOT_INT: ErrorInfo =
    ErrorInfo::BadRequest("sys_config.value_not_int", "值不是合法的 int");
pub const VALUE_NOT_BOOL: ErrorInfo =
    ErrorInfo::BadRequest("sys_config.value_not_bool", "值不是合法的 bool");
pub const VALUE_NOT_JSON: ErrorInfo =
    ErrorInfo::BadRequest("sys_config.value_not_json", "值不是合法的 JSON");
pub const VALUE_TYPE_UNSUPPORTED: ErrorInfo =
    ErrorInfo::BadRequest("sys_config.value_type_unsupported", "不支持的 value_type");
pub const KEY_LENGTH_INVALID: ErrorInfo = ErrorInfo::BadRequest(
    "sys_config.key_length_invalid",
    "key 长度必须在 1~128 字符之间",
);
pub const KEY_CHARS_INVALID: ErrorInfo = ErrorInfo::BadRequest(
    "sys_config.key_chars_invalid",
    "key 只允许小写字母、数字、下划线和点",
);
pub const KEY_DOT_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("sys_config.key_dot_invalid", "key 不能以点开头或结尾");
pub const KEY_CONSECUTIVE_DOTS: ErrorInfo =
    ErrorInfo::BadRequest("sys_config.key_consecutive_dots", "key 不能包含连续的点");
pub const KEY_GLOBAL_EXISTS: ErrorInfo =
    ErrorInfo::BadRequest("sys_config.key_global_exists", "全局配置 key 已存在");
pub const KEY_GLOBAL_NOT_FOUND: ErrorInfo =
    ErrorInfo::BadRequest("sys_config.key_global_not_found", "全局配置 key 不存在");
pub const CATEGORY_MISMATCH: ErrorInfo = ErrorInfo::BadRequest(
    "sys_config.category_mismatch",
    "category 必须等于 key 的第一段",
);
