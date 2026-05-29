use crate::error_info::ErrorInfo;

pub const MULTIPART_FIELD_READ_FAILED: ErrorInfo = ErrorInfo::BadRequest(
    "upload.multipart_field_read_failed",
    "无法读取 multipart 字段",
);
pub const FILE_NAME_NOT_FOUND: ErrorInfo =
    ErrorInfo::BadRequest("upload.file_name_not_found", "文件名未找到");
pub const FILE_BYTES_READ_FAILED: ErrorInfo =
    ErrorInfo::BadRequest("upload.file_bytes_read_failed", "无法读取文件数据");
pub const ATTACH_TO_FIELD_READ_FAILED: ErrorInfo = ErrorInfo::BadRequest(
    "upload.attach_to_field_read_failed",
    "无法读取 attachTo 字段",
);
pub const ATTACH_TO_INVALID_JSON: ErrorInfo =
    ErrorInfo::BadRequest("upload.attach_to_invalid_json", "attachTo 不是有效的 JSON");
pub const FILE_FIELD_REQUIRED: ErrorInfo = ErrorInfo::BadRequest(
    "upload.file_field_required",
    "multipart 字段 `file` 是必需的",
);
pub const RESOURCE_TYPE_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("upload.resource_type_invalid", "无效的 resource_type");
pub const FILE_FIELD_READ_FAILED: ErrorInfo = ErrorInfo::BadRequest(
    "upload.file_field_read_failed",
    "无法读取 multipart file 字段",
);
