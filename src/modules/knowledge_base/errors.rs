use axum::response::Response;
use loco_rs::prelude::*;

/// Knowledge Base module error codes.
///
/// All codes follow the `knowledge_base.<detail>` format.
/// The frontend errorMap can use these for i18n translation.
#[derive(Debug)]
pub enum KnowledgeBaseError {
    /// 文档/分块不存在
    NotFound,
    /// 跨租户操作被拒绝
    Forbidden,
    /// Provider 级别错误
    ProviderError(String),
    /// 文档解析失败
    ParsingError(String),
    /// 索引/写入 Qdrant 失败
    IndexingError(String),
    /// 配置缺失或无效
    ConfigError(String),
    /// 不支持的文件格式
    UnsupportedFormat(String),
    /// 嵌入生成失败
    EmbeddingError(String),
}

impl std::fmt::Display for KnowledgeBaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "文档/分块不存在"),
            Self::Forbidden => write!(f, "跨租户操作被拒绝"),
            Self::ProviderError(msg) => write!(f, "Provider 错误: {msg}"),
            Self::ParsingError(msg) => write!(f, "文档解析失败: {msg}"),
            Self::IndexingError(msg) => write!(f, "索引失败: {msg}"),
            Self::ConfigError(msg) => write!(f, "配置错误: {msg}"),
            Self::UnsupportedFormat(msg) => write!(f, "不支持的文件格式: {msg}"),
            Self::EmbeddingError(msg) => write!(f, "嵌入生成失败: {msg}"),
        }
    }
}

impl std::error::Error for KnowledgeBaseError {}

impl KnowledgeBaseError {
    /// Convert to a `loco_rs::Error` using module-local `error_info` constants.
    ///
    /// Delegates to `from_info()` / `from_info_with_desc()` for unified location
    /// tracking via `#[track_caller]`.
    #[track_caller]
    #[must_use]
    pub fn to_err(&self) -> loco_rs::Error {
        use crate::modules::knowledge_base::error_info as ei;
        match self {
            Self::NotFound => crate::views::errors::from_info(ei::NOT_FOUND),
            Self::Forbidden => crate::views::errors::from_info(ei::FORBIDDEN),
            Self::ProviderError(msg) => crate::views::errors::from_info_with_desc(
                ei::PROVIDER_ERROR,
                format!("Provider错误: {msg}"),
            ),
            Self::ParsingError(msg) => crate::views::errors::from_info_with_desc(
                ei::PARSING_ERROR,
                format!("文档解析失败: {msg}"),
            ),
            Self::IndexingError(msg) => crate::views::errors::from_info_with_desc(
                ei::INDEXING_ERROR,
                format!("索引失败: {msg}"),
            ),
            Self::ConfigError(msg) => crate::views::errors::from_info_with_desc(
                ei::CONFIG_ERROR,
                format!("配置错误: {msg}"),
            ),
            Self::UnsupportedFormat(msg) => crate::views::errors::from_info_with_desc(
                ei::UNSUPPORTED_FORMAT,
                format!("不支持的文件格式: {msg}"),
            ),
            Self::EmbeddingError(msg) => crate::views::errors::from_info_with_desc(
                ei::EMBEDDING_ERROR,
                format!("嵌入生成失败: {msg}"),
            ),
        }
    }

    pub fn to_response(&self) -> Result<Response> {
        Err(self.to_err())
    }
}
