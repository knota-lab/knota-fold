use crate::error_info::ErrorInfo;

pub const NOT_FOUND: ErrorInfo =
    ErrorInfo::NotFound("knowledge_base.not_found", "文档/分块不存在");
pub const FORBIDDEN: ErrorInfo =
    ErrorInfo::Forbidden("knowledge_base.forbidden", "跨租户操作被拒绝");
// Dynamic description codes — ErrorInfo provides defaults, overridden at runtime via from_info_with_desc()
pub const PROVIDER_ERROR: ErrorInfo =
    ErrorInfo::Internal("knowledge_base.provider_error", "Provider错误");
pub const PARSING_ERROR: ErrorInfo =
    ErrorInfo::BadRequest("knowledge_base.parsing_error", "文档解析失败");
pub const INDEXING_ERROR: ErrorInfo =
    ErrorInfo::Internal("knowledge_base.indexing_error", "索引失败");
pub const CONFIG_ERROR: ErrorInfo =
    ErrorInfo::Internal("knowledge_base.config_error", "配置错误");
pub const UNSUPPORTED_FORMAT: ErrorInfo =
    ErrorInfo::BadRequest("knowledge_base.unsupported_format", "不支持的文件格式");
pub const EMBEDDING_ERROR: ErrorInfo =
    ErrorInfo::Internal("knowledge_base.embedding_error", "嵌入生成失败");
