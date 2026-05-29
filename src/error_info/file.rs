use crate::error_info::ErrorInfo;

// file_service / file_upload_service
pub const FILE_NAME_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("file.name_empty", "文件名不能为空");
pub const FILE_NAME_TOO_LONG: ErrorInfo =
    ErrorInfo::BadRequest("file.name_too_long", "文件名过长");
pub const FILE_SIZE_OVERFLOW: ErrorInfo =
    ErrorInfo::BadRequest("file.size_overflow", "文件大小溢出");
pub const FILE_SIZE_MUST_BE_POSITIVE: ErrorInfo =
    ErrorInfo::BadRequest("file.size_must_be_positive", "文件大小必须大于 0");
pub const FILE_CONTENT_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("file.content_empty", "文件内容不能为空");
pub const EXPECTED_SIZE_MUST_BE_POSITIVE: ErrorInfo = ErrorInfo::BadRequest(
    "file.expected_size_must_be_positive",
    "expectedSize 必须大于 0",
);
pub const INVALID_PART_NUMBER: ErrorInfo =
    ErrorInfo::BadRequest("file.invalid_part_number", "无效的分片编号");
pub const ENV_EMPTY: ErrorInfo =
    ErrorInfo::BadRequest("file.env_empty", "environment 不能为空");

// file_hash
pub const HASH_PREFIX_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("file.hash_prefix_invalid", "contentHash 必须以 b3: 开头");
pub const HASH_LENGTH_INVALID: ErrorInfo = ErrorInfo::BadRequest(
    "file.hash_length_invalid",
    "contentHash 必须包含 64 个小写十六进制字符",
);
pub const HASH_HEX_INVALID: ErrorInfo = ErrorInfo::BadRequest(
    "file.hash_hex_invalid",
    "contentHash 只能包含小写十六进制字符",
);

// file_uploads views (fast hash)
pub const HASH_FAST_PREFIX_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("file.hash_fast_prefix_invalid", "字段必须以 b3fast: 开头");
pub const HASH_FAST_LENGTH_INVALID: ErrorInfo = ErrorInfo::BadRequest(
    "file.hash_fast_length_invalid",
    "字段必须包含 64 个小写十六进制字符",
);
pub const HASH_FAST_HEX_INVALID: ErrorInfo =
    ErrorInfo::BadRequest("file.hash_fast_hex_invalid", "字段只能包含小写十六进制字符");
