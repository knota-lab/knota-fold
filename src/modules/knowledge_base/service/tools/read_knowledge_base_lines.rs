use std::fmt;
use std::sync::Arc;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::models::_entities::{document_lines, kb_chunks, kb_documents};

#[derive(Debug)]
pub struct ReadKnowledgeBaseLinesError(pub String);

impl fmt::Display for ReadKnowledgeBaseLinesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "read knowledge base lines error: {}", self.0)
    }
}

impl std::error::Error for ReadKnowledgeBaseLinesError {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadKnowledgeBaseLinesArgs {
    /// Knowledge base document ID returned by `search_knowledge_base`.
    document_id: Option<Uuid>,
    /// Chunk ID returned by `search_knowledge_base`.
    chunk_id: Option<Uuid>,
    /// 1-indexed start line. Required when `chunk_id` is omitted.
    start_line: Option<i32>,
    /// 1-indexed end line. Defaults to `start_line`.
    end_line: Option<i32>,
    /// Extra lines to include before the requested range. Defaults to 0, max 100.
    before_lines: Option<i32>,
    /// Extra lines to include after the requested range. Defaults to 0, max 100.
    after_lines: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReadKnowledgeBaseLinesTool {
    #[serde(skip)]
    pub db: Arc<DatabaseConnection>,
    #[serde(skip)]
    pub tenant_id: Uuid,
    #[serde(skip)]
    pub user_id: Uuid,
    #[serde(skip)]
    pub library_id: Option<Uuid>,
    #[serde(skip)]
    pub folder_id: Option<Uuid>,
    #[serde(skip)]
    pub folder_ids: Option<Vec<Uuid>>,
    #[serde(skip)]
    pub document_ids: Option<Vec<Uuid>>,
}

impl Tool for ReadKnowledgeBaseLinesTool {
    const NAME: &'static str = "read_knowledge_base_lines";

    type Error = ReadKnowledgeBaseLinesError;
    type Args = ReadKnowledgeBaseLinesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "按行读取知识库文档原文上下文。search_knowledge_base 返回文档ID、分块ID或行号后，可用此工具读取命中片段前后的原文行。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "documentId": {
                        "type": "string",
                        "description": "知识库文档ID。按行号读取时必填。"
                    },
                    "chunkId": {
                        "type": "string",
                        "description": "知识库分块ID。传入后会自动定位该分块所在行。"
                    },
                    "startLine": {
                        "type": "integer",
                        "description": "起始行号（1-indexed）。未传 chunkId 时必填。"
                    },
                    "endLine": {
                        "type": "integer",
                        "description": "结束行号（1-indexed，默认等于 startLine）。"
                    },
                    "beforeLines": {
                        "type": "integer",
                        "description": "额外读取命中范围前多少行（默认0，最大100）。"
                    },
                    "afterLines": {
                        "type": "integer",
                        "description": "额外读取命中范围后多少行（默认0，最大100）。"
                    }
                },
                "required": []
            }),
        }
    }

    #[tracing::instrument(
        skip(self, args),
        fields(tool = "read_knowledge_base_lines", tenant_id = %self.tenant_id)
    )]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let before = args.before_lines.unwrap_or(0).clamp(0, 100);
        let after = args.after_lines.unwrap_or(0).clamp(0, 100);

        let target = self.resolve_target(&args).await?;
        let document = self.visible_document(target.document_id).await?;
        let start_line = target.start_line.saturating_sub(before).max(1);
        let end_line = target.end_line.saturating_add(after).max(start_line);
        let lines = self
            .read_lines(target.document_id, start_line, end_line)
            .await?;

        if lines.is_empty() {
            return Ok(format!(
                "文档《{}》第 {}-{} 行没有可读取内容。",
                document.title, start_line, end_line
            ));
        }

        let mut output = Vec::with_capacity(lines.len() + 1);
        output.push(format!(
            "文档《{}》({}) 第 {}-{} 行：",
            document.title, document.id, start_line, end_line
        ));
        output.extend(
            lines
                .into_iter()
                .map(|line| format!("{}: {}", line.line_number, line.line_text)),
        );
        Ok(output.join("\n"))
    }
}

impl ReadKnowledgeBaseLinesTool {
    async fn resolve_target(
        &self,
        args: &ReadKnowledgeBaseLinesArgs,
    ) -> Result<LineTarget, ReadKnowledgeBaseLinesError> {
        if let Some(chunk_id) = args.chunk_id {
            return self.resolve_chunk_target(chunk_id).await;
        }

        let document_id = args.document_id.ok_or_else(|| {
            ReadKnowledgeBaseLinesError(
                "documentId is required when chunkId is not provided".to_string(),
            )
        })?;
        let start_line = args.start_line.unwrap_or(1).max(1);
        let end_line = args.end_line.unwrap_or(start_line).max(start_line);
        Ok(LineTarget {
            document_id,
            start_line,
            end_line,
        })
    }

    async fn resolve_chunk_target(
        &self,
        chunk_id: Uuid,
    ) -> Result<LineTarget, ReadKnowledgeBaseLinesError> {
        let chunk = kb_chunks::Entity::find_by_id(chunk_id)
            .filter(kb_chunks::Column::TenantId.eq(self.tenant_id))
            .one(&*self.db)
            .await
            .map_err(|e| ReadKnowledgeBaseLinesError(e.to_string()))?
            .ok_or_else(|| {
                ReadKnowledgeBaseLinesError("chunk not found or not visible".to_string())
            })?;
        let lines = document_lines::Entity::find()
            .filter(document_lines::Column::TenantId.eq(self.tenant_id))
            .filter(document_lines::Column::DocumentId.eq(chunk.document_id))
            .order_by_asc(document_lines::Column::LineNumber)
            .all(&*self.db)
            .await
            .map_err(|e| ReadKnowledgeBaseLinesError(e.to_string()))?;
        let start_line =
            char_offset_to_line(i64::from(chunk.char_start.unwrap_or(0)), &lines)
                .unwrap_or(1);
        let end_offset = i64::from(
            chunk
                .char_end
                .unwrap_or_else(|| chunk.char_start.unwrap_or(0)),
        )
        .saturating_sub(1)
        .max(i64::from(chunk.char_start.unwrap_or(0)));
        let end_line = char_offset_to_line(end_offset, &lines).unwrap_or(start_line);
        Ok(LineTarget {
            document_id: chunk.document_id,
            start_line,
            end_line: end_line.max(start_line),
        })
    }

    async fn visible_document(
        &self,
        document_id: Uuid,
    ) -> Result<kb_documents::Model, ReadKnowledgeBaseLinesError> {
        let mut query = kb_documents::Entity::find_by_id(document_id)
            .filter(kb_documents::Column::TenantId.eq(self.tenant_id))
            .filter(kb_documents::Column::DeletedAt.is_null())
            .filter(kb_documents::Column::Status.eq("ready"))
            .filter(
                kb_documents::Column::Scope
                    .eq("tenant")
                    .or(kb_documents::Column::Scope
                        .eq("private")
                        .and(kb_documents::Column::CreatedBy.eq(self.user_id))),
            );

        if let Some(document_ids) = &self.document_ids {
            if !document_ids.is_empty() && !document_ids.contains(&document_id) {
                return Err(ReadKnowledgeBaseLinesError(
                    "document is outside the current knowledge base scope".to_string(),
                ));
            }
        }
        if let Some(library_id) = self.library_id {
            query = query.filter(kb_documents::Column::LibraryId.eq(library_id));
        }
        if let Some(folder_ids) = &self.folder_ids {
            if !folder_ids.is_empty() {
                query = query
                    .filter(kb_documents::Column::FolderId.is_in(folder_ids.clone()));
            }
        } else if let Some(folder_id) = self.folder_id {
            query = query.filter(kb_documents::Column::FolderId.eq(folder_id));
        }

        query
            .one(&*self.db)
            .await
            .map_err(|e| ReadKnowledgeBaseLinesError(e.to_string()))?
            .ok_or_else(|| {
                ReadKnowledgeBaseLinesError(
                    "document not found or not visible".to_string(),
                )
            })
    }

    async fn read_lines(
        &self,
        document_id: Uuid,
        start_line: i32,
        end_line: i32,
    ) -> Result<Vec<document_lines::Model>, ReadKnowledgeBaseLinesError> {
        document_lines::Entity::find()
            .filter(document_lines::Column::TenantId.eq(self.tenant_id))
            .filter(document_lines::Column::DocumentId.eq(document_id))
            .filter(document_lines::Column::LineNumber.gte(start_line))
            .filter(document_lines::Column::LineNumber.lte(end_line))
            .order_by_asc(document_lines::Column::LineNumber)
            .limit(600)
            .all(&*self.db)
            .await
            .map_err(|e| ReadKnowledgeBaseLinesError(e.to_string()))
    }
}

#[derive(Debug, Clone, Copy)]
struct LineTarget {
    document_id: Uuid,
    start_line: i32,
    end_line: i32,
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
    use super::char_offset_to_line;
    use crate::models::_entities::document_lines;
    use uuid::Uuid;

    #[test]
    fn char_offset_to_line_accounts_for_newline_spans() {
        let lines = vec![line(1, "aaa", 4), line(2, "bbb", 8), line(3, "ccc", 11)];

        assert_eq!(char_offset_to_line(0, &lines), Some(1));
        assert_eq!(char_offset_to_line(3, &lines), Some(1));
        assert_eq!(char_offset_to_line(4, &lines), Some(2));
        assert_eq!(char_offset_to_line(10, &lines), Some(3));
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
}
