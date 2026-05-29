use crate::error_info::ErrorInfo;

pub const NAMESPACE_NOT_PUBLIC: ErrorInfo =
    ErrorInfo::Forbidden("i18n.namespace_not_public", "命名空间不对外公开");

// Validation
pub const VALUE_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("i18n.value_empty", "value 不能为空");
pub const ENTRIES_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("i18n.entries_empty", "entries 不能为空");
pub const MANIFEST_ENTRIES_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("i18n.manifest_entries_empty", "manifest entries 不能为空");
pub const NAMESPACE_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("i18n.namespace_empty", "namespace 不能为空");
pub const KEY_EMPTY: ErrorInfo = ErrorInfo::BadRequest("i18n.key_empty", "key 不能为空");
pub const LOCALE_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("i18n.locale_empty", "locale 不能为空");
pub const ENTRY_VALUE_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("i18n.entry_value_empty", "value 不能为空");

// Locale validation
pub const LOCALE_LENGTH_INVALID: ErrorInfo = ErrorInfo::BadRequest(
    "i18n.locale_length_invalid",
    "locale 长度必须在 2~35 字符之间",
);
pub const LOCALE_CHARS_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("i18n.locale_chars_invalid", "locale 仅允许字母、数字与 '-'");
pub const LOCALE_MUST_START_WITH_LETTER: ErrorInfo = ErrorInfo::BadRequest(
    "i18n.locale_must_start_with_letter",
    "locale 必须以字母开头",
);
pub const LOCALE_EXISTS: ErrorInfo =
    ErrorInfo::BadRequest("i18n.locale_exists", "locale 已存在");
pub const BASE_LOCALE_PROTECTED: ErrorInfo =
    ErrorInfo::BadRequest("i18n.base_locale_protected", "基础 locale 不可删除");
pub const GLOBAL_DELETE_ONLY: ErrorInfo =
    ErrorInfo::BadRequest("i18n.global_delete_only", "该接口仅可删除全局翻译");

// Import validation
pub const IMPORT_FORMAT_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("i18n.import_format_invalid", "不支持的导入格式");
pub const ENTRY_NAMESPACE_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("i18n.entry_namespace_invalid", "namespace 格式非法");
pub const ENTRY_KEY_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("i18n.entry_key_invalid", "key 格式非法");
pub const ENTRY_LOCALE_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("i18n.entry_locale_invalid", "locale 格式非法");

// Manifest validation
pub const MANIFEST_NAMESPACE_INVALID: ErrorInfo = ErrorInfo::BadRequest(
    "i18n.manifest_namespace_invalid",
    "manifest namespace 格式非法",
);
pub const MANIFEST_KEY_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("i18n.manifest_key_invalid", "manifest key 格式非法");
pub const MANIFEST_LOCALE_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("i18n.manifest_locale_invalid", "manifest locale 格式非法");
pub const MANIFEST_VALUE_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("i18n.manifest_value_empty", "manifest value 不能为空");
pub const MANIFEST_ENTRY_DUPLICATE: ErrorInfo =
    ErrorInfo::BadRequest("i18n.manifest_entry_duplicate", "manifest 条目重复");
pub const MANIFEST_ENTRIES_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("i18n.manifest_entries_invalid", "manifest entries 格式非法");
pub const UNSUPPORTED_LOCALE: ErrorInfo =
    ErrorInfo::BadRequest("i18n.unsupported_locale", "不支持的语言");
pub const NAMESPACE_LENGTH_INVALID: ErrorInfo = ErrorInfo::BadRequest(
    "i18n.namespace_length_invalid",
    "namespace 长度必须在 1~64 字符之间",
);
pub const NAMESPACE_CHARS_INVALID: ErrorInfo = ErrorInfo::BadRequest(
    "i18n.namespace_chars_invalid",
    "namespace 仅允许字母、数字、'.'、'_'",
);
pub const KEY_LENGTH_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("i18n.key_length_invalid", "key 长度必须在 1~256 字符之间");
pub const KEY_CHARS_INVALID: ErrorInfo = ErrorInfo::BadRequest(
    "i18n.key_chars_invalid",
    "key 仅允许字母、数字、'.'、'_'、'-'",
);
pub const LOCALE_REGEX_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("i18n.locale_regex_invalid", "locale 格式非法");
pub const BATCH_SOURCE_REQUIRED: ErrorInfo =
    ErrorInfo::BadRequest("i18n.batch_source_required", "批量操作需要指定来源");
