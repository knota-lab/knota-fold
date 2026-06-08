use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use rig::completion::ToolDefinition;
use rig::tool::Tool;

use crate::initializers::knowledge_base::SharedSearchProvider;
use crate::modules::knowledge_base::providers::SharedEmbeddingClient;
use crate::modules::knowledge_base::service::search_service;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct SearchKBError(pub String);

impl fmt::Display for SearchKBError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "search knowledge base error: {}", self.0)
    }
}

impl std::error::Error for SearchKBError {}

// ---------------------------------------------------------------------------
// Args
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SearchKnowledgeBaseArgs {
    /// Search query text
    query: String,
    /// Max results (default 5, max 20)
    top_k: Option<u32>,
}

// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

/// Tool that performs semantic search across knowledge base documents.
///
/// Contains `dyn trait` and client fields that cannot auto-derive
/// `Debug`/`Clone`/`Deserialize`/`Serialize`, so we provide manual impls.
pub struct SearchKnowledgeBaseTool {
    pub embedding_client: SharedEmbeddingClient,
    pub search_provider: SharedSearchProvider,
    pub embedding_model_name: String,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub library_id: Option<Uuid>,
    pub folder_id: Option<Uuid>,
    pub document_ids: Option<Vec<Uuid>>,
}

impl fmt::Debug for SearchKnowledgeBaseTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchKnowledgeBaseTool")
            .field("embedding_model_name", &self.embedding_model_name)
            .field("tenant_id", &self.tenant_id)
            .field("user_id", &self.user_id)
            .field("library_id", &self.library_id)
            .field("folder_id", &self.folder_id)
            .field("document_ids", &self.document_ids)
            .finish_non_exhaustive()
    }
}

impl Clone for SearchKnowledgeBaseTool {
    fn clone(&self) -> Self {
        Self {
            embedding_client: self.embedding_client.clone(),
            search_provider: self.search_provider.clone(),
            embedding_model_name: self.embedding_model_name.clone(),
            tenant_id: self.tenant_id,
            user_id: self.user_id,
            library_id: self.library_id,
            folder_id: self.folder_id,
            document_ids: self.document_ids.clone(),
        }
    }
}

impl Serialize for SearchKnowledgeBaseTool {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        unreachable!("SearchKnowledgeBaseTool is never serialized")
    }
}

impl<'de> Deserialize<'de> for SearchKnowledgeBaseTool {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        unreachable!("SearchKnowledgeBaseTool is never deserialized")
    }
}

// ---------------------------------------------------------------------------
// Tool trait implementation
// ---------------------------------------------------------------------------

impl Tool for SearchKnowledgeBaseTool {
    const NAME: &'static str = "search_knowledge_base";

    type Error = SearchKBError;
    type Args = SearchKnowledgeBaseArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let scope_description = self.scope_description();
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "在知识库已上传的文档中进行语义搜索。返回最相关的文档片段。当你需要查找材料中没有的信息时使用此工具。{scope_description}"
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "搜索查询文本"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "返回的最大结果数量（默认5，最大20）"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    #[tracing::instrument(
        skip(self, args),
        fields(tool = "search_knowledge_base", tenant_id = %self.tenant_id)
    )]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let top_k = args.top_k.unwrap_or(5).clamp(1, 20);

        let results = search_service::hybrid_search(
            &self.embedding_client,
            &self.search_provider,
            &search_service::HybridSearchParams {
                model_name: self.embedding_model_name.clone(),
                query: args.query.clone(),
                tenant_id: self.tenant_id,
                user_id: self.user_id,
                limit: top_k as usize,
                library_id: self.library_id,
                folder_id: self.folder_id,
                document_ids: self.document_ids.clone(),
            },
        )
        .await
        .map_err(|e| SearchKBError(e.to_string()))?;

        if results.is_empty() {
            return Ok(format!(
                "知识库搜索 \"{query}\" 未找到相关结果。",
                query = args.query
            ));
        }

        let mut output = Vec::with_capacity(results.len() * 3 + 1);
        output.push(format!(
            "知识库搜索 \"{query}\"，找到 {count} 条相关结果：\n",
            query = args.query,
            count = results.len(),
        ));

        for (i, r) in results.iter().enumerate() {
            // Char-boundary-safe truncation (CJK chars are multi-byte)
            let truncated = if r.content.len() > 500 {
                let end = r
                    .content
                    .char_indices()
                    .take_while(|(i, c)| i + c.len_utf8() <= 500)
                    .last()
                    .map_or(0, |(i, c)| i + c.len_utf8());
                format!("{}…", &r.content[..end])
            } else {
                r.content.clone()
            };

            let heading = r.heading_path.as_deref().unwrap_or("无标题");

            output.push(format!(
                "{}. [{}] (分数: {:.2}, 文档ID: {}, 分块ID: {})\n相关内容:\n{}\n",
                i + 1,
                heading,
                r.score,
                r.document_id,
                r.chunk_id,
                truncated,
            ));
        }

        Ok(output.join("\n"))
    }
}

impl SearchKnowledgeBaseTool {
    fn scope_description(&self) -> String {
        if self
            .document_ids
            .as_ref()
            .is_some_and(|ids| !ids.is_empty())
        {
            return "当前搜索范围已限定为用户指定的文档集合。".to_string();
        }
        if self.folder_id.is_some() {
            return "当前搜索范围已限定为用户指定目录下的直接文档。".to_string();
        }
        if self.library_id.is_some() {
            return "当前搜索范围已限定为用户指定知识库。".to_string();
        }
        "当前搜索范围为租户内可见知识库文档。".to_string()
    }
}
