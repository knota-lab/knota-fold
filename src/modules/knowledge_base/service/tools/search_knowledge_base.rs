use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

use crate::initializers::knowledge_base::SharedSearchProvider;
use crate::models::_entities::document_lines;
use crate::modules::knowledge_base::providers::search::SearchResult;
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
    pub db: Arc<DatabaseConnection>,
    pub embedding_client: SharedEmbeddingClient,
    pub search_provider: SharedSearchProvider,
    pub embedding_model_name: String,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub library_id: Option<Uuid>,
    pub folder_id: Option<Uuid>,
    pub folder_ids: Option<Vec<Uuid>>,
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
            .field("folder_ids", &self.folder_ids)
            .field("document_ids", &self.document_ids)
            .finish_non_exhaustive()
    }
}

impl Clone for SearchKnowledgeBaseTool {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            embedding_client: self.embedding_client.clone(),
            search_provider: self.search_provider.clone(),
            embedding_model_name: self.embedding_model_name.clone(),
            tenant_id: self.tenant_id,
            user_id: self.user_id,
            library_id: self.library_id,
            folder_id: self.folder_id,
            folder_ids: self.folder_ids.clone(),
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
                folder_ids: self.folder_ids.clone(),
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

        let line_ranges = resolve_line_ranges(&self.db, self.tenant_id, &results)
            .await
            .map_err(SearchKBError)?;
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
            let location = line_ranges.get(&r.chunk_id).map_or_else(
                || "位置: 未知".to_string(),
                |range| {
                    if range.start_line == range.end_line {
                        format!("位置: 第 {} 行", range.start_line)
                    } else {
                        format!("位置: 第 {}-{} 行", range.start_line, range.end_line)
                    }
                },
            );

            output.push(format!(
                "{}. [{}] (分数: {:.2}, {}, 文档ID: {}, 分块ID: {})\n相关内容:\n{}\n",
                i + 1,
                heading,
                r.score,
                location,
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
        if self.folder_ids.as_ref().is_some_and(|ids| ids.len() > 1) {
            return "当前搜索范围已限定为用户指定目录及其子目录。".to_string();
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

#[derive(Debug, Clone, Copy)]
struct LineRange {
    start_line: i32,
    end_line: i32,
}

async fn resolve_line_ranges(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    results: &[SearchResult],
) -> Result<std::collections::HashMap<Uuid, LineRange>, String> {
    let document_ids: Vec<Uuid> =
        results.iter().map(|result| result.document_id).collect();
    if document_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let lines = document_lines::Entity::find()
        .filter(document_lines::Column::TenantId.eq(tenant_id))
        .filter(document_lines::Column::DocumentId.is_in(document_ids))
        .order_by_asc(document_lines::Column::DocumentId)
        .order_by_asc(document_lines::Column::LineNumber)
        .all(db)
        .await
        .map_err(|e| e.to_string())?;

    let mut lines_by_doc: std::collections::HashMap<Uuid, Vec<document_lines::Model>> =
        std::collections::HashMap::new();
    for line in lines {
        lines_by_doc.entry(line.document_id).or_default().push(line);
    }

    Ok(results
        .iter()
        .filter_map(|result| {
            let lines = lines_by_doc.get(&result.document_id)?;
            let range = line_range_for_result(result, lines)?;
            Some((result.chunk_id, range))
        })
        .collect())
}

fn line_range_for_result(
    result: &SearchResult,
    lines: &[document_lines::Model],
) -> Option<LineRange> {
    let start = i64::from(result.char_start?);
    let end_exclusive = i64::from(result.char_end?);
    let start_line = char_offset_to_line(start, lines)?;
    let end_line =
        char_offset_to_line(end_exclusive.saturating_sub(1).max(start), lines)?;
    Some(LineRange {
        start_line,
        end_line: end_line.max(start_line),
    })
}

fn char_offset_to_line(offset: i64, lines: &[document_lines::Model]) -> Option<i32> {
    if lines.is_empty() {
        return None;
    }

    for line in lines {
        let line_start = line
            .cumulative_chars
            .saturating_sub(i64::from(line.line_chars));
        if offset >= line_start && offset < line.cumulative_chars {
            return Some(line.line_number);
        }
        if line.line_chars == 0 && offset == line_start {
            return Some(line.line_number);
        }
    }

    lines.last().map(|line| line.line_number)
}

#[cfg(test)]
mod tests {
    use super::{char_offset_to_line, line_range_for_result};
    use crate::models::_entities::document_lines;
    use crate::modules::knowledge_base::providers::search::SearchResult;
    use uuid::Uuid;

    #[test]
    fn char_offset_to_line_maps_boundaries_to_next_line() {
        let lines = vec![line(1, "aaa", 4), line(2, "bbb", 8), line(3, "ccc", 11)];

        assert_eq!(char_offset_to_line(0, &lines), Some(1));
        assert_eq!(char_offset_to_line(3, &lines), Some(1));
        assert_eq!(char_offset_to_line(4, &lines), Some(2));
        assert_eq!(char_offset_to_line(10, &lines), Some(3));
    }

    #[test]
    fn line_range_for_result_maps_chunk_char_range() {
        let lines = vec![line(1, "aaa", 4), line(2, "bbb", 8), line(3, "ccc", 11)];
        let result = search_result(Some(2), Some(9));

        let range = line_range_for_result(&result, &lines).unwrap();

        assert_eq!(range.start_line, 1);
        assert_eq!(range.end_line, 3);
    }

    fn line(
        line_number: i32,
        text: &str,
        cumulative_chars: i64,
    ) -> document_lines::Model {
        document_lines::Model {
            tenant_id: Uuid::now_v7(),
            document_id: Uuid::now_v7(),
            line_number,
            line_text: text.to_string(),
            line_chars: if line_number < 3 {
                i32::try_from(text.chars().count() + 1).unwrap()
            } else {
                i32::try_from(text.chars().count()).unwrap()
            },
            cumulative_chars,
            created_at: chrono::Utc::now().naive_utc(),
        }
    }

    fn search_result(char_start: Option<i32>, char_end: Option<i32>) -> SearchResult {
        SearchResult {
            chunk_id: Uuid::now_v7(),
            document_id: Uuid::now_v7(),
            content: String::new(),
            heading_path: None,
            page_number: None,
            char_start,
            char_end,
            score: 1.0,
        }
    }
}
