use crate::error_info::ErrorInfo;

pub const INVALID_UUID: ErrorInfo =
    ErrorInfo::BadRequest("common.invalid_uuid", "无效的ID格式");
pub const NOT_FOUND: ErrorInfo = ErrorInfo::NotFound("common.not_found", "记录不存在");
pub const DUPLICATE: ErrorInfo =
    ErrorInfo::Conflict("common.duplicate", "数据重复，已存在相同记录");
pub const DB_ERROR: ErrorInfo = ErrorInfo::Internal("common.db_error", "数据库错误");
pub const INTERNAL: ErrorInfo = ErrorInfo::Internal("common.internal", "内部错误");
pub const INVALID_REFERENCE: ErrorInfo =
    ErrorInfo::BadRequest("common.invalid_reference", "引用的资源不存在");
pub const MISSING_FIELD: ErrorInfo =
    ErrorInfo::BadRequest("common.missing_field", "缺少必填字段");
