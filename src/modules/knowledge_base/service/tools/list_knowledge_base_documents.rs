use std::collections::HashMap;
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

use crate::models::_entities::{kb_documents, kb_folders};

#[derive(Debug)]
pub struct ListKnowledgeBaseDocumentsError(pub String);

impl fmt::Display for ListKnowledgeBaseDocumentsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "list knowledge base documents error: {}", self.0)
    }
}

impl std::error::Error for ListKnowledgeBaseDocumentsError {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListKnowledgeBaseDocumentsArgs {
    /// Max documents to return. Defaults to 50, max 100.
    limit: Option<u64>,
    /// Include documents that are still indexing or failed. Defaults to false.
    include_non_ready: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ListKnowledgeBaseDocumentsTool {
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

impl Tool for ListKnowledgeBaseDocumentsTool {
    const NAME: &'static str = "list_knowledge_base_documents";

    type Error = ListKnowledgeBaseDocumentsError;
    type Args = ListKnowledgeBaseDocumentsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "列出当前知识库范围内你可见的文档清单。适合回答“有哪些资料/文档可用”。{}",
                self.scope_description()
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "返回的最大文档数（默认50，最大100）"
                    },
                    "includeNonReady": {
                        "type": "boolean",
                        "description": "是否包含入库中或失败的文档（默认false，只返回可用文档）"
                    }
                },
                "required": []
            }),
        }
    }

    #[tracing::instrument(
        skip(self, args),
        fields(tool = "list_knowledge_base_documents", tenant_id = %self.tenant_id)
    )]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let limit = args.limit.unwrap_or(50).clamp(1, 100);
        let docs = self
            .query_documents(limit, args.include_non_ready.unwrap_or(false))
            .await?;

        if docs.is_empty() {
            return Ok("当前知识库范围内没有可见文档。".to_string());
        }

        let folder_paths = folder_paths(&self.db, self.tenant_id).await?;
        let mut lines = Vec::with_capacity(docs.len() + 1);
        lines.push(format!("当前知识库范围内找到 {} 个可见文档：", docs.len()));
        for (idx, doc) in docs.iter().enumerate() {
            let folder = doc
                .folder_id
                .and_then(|id| folder_paths.get(&id))
                .map_or("未归档", String::as_str);
            lines.push(format!(
                "{}. {} (ID: {}, 状态: {}, 目录: {}, 分块: {}, tokens: {}, 更新时间: {}){}",
                idx + 1,
                doc.title,
                doc.id,
                doc.status,
                folder,
                doc.chunk_count,
                doc.total_tokens,
                doc.updated_at.format("%Y-%m-%d %H:%M:%S"),
                doc.description
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map_or_else(String::new, |desc| format!("\n   描述: {desc}")),
            ));
        }

        Ok(lines.join("\n"))
    }
}

impl ListKnowledgeBaseDocumentsTool {
    async fn query_documents(
        &self,
        limit: u64,
        include_non_ready: bool,
    ) -> Result<Vec<kb_documents::Model>, ListKnowledgeBaseDocumentsError> {
        let mut query = kb_documents::Entity::find()
            .filter(kb_documents::Column::TenantId.eq(self.tenant_id))
            .filter(
                kb_documents::Column::Scope
                    .eq("tenant")
                    .or(kb_documents::Column::Scope
                        .eq("private")
                        .and(kb_documents::Column::CreatedBy.eq(self.user_id))),
            );

        if !include_non_ready {
            query = query.filter(kb_documents::Column::Status.eq("ready"));
        }
        if let Some(document_ids) = &self.document_ids {
            if !document_ids.is_empty() {
                query =
                    query.filter(kb_documents::Column::Id.is_in(document_ids.clone()));
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
            .order_by_desc(kb_documents::Column::UpdatedAt)
            .limit(limit)
            .all(&*self.db)
            .await
            .map_err(|e| ListKnowledgeBaseDocumentsError(e.to_string()))
    }

    fn scope_description(&self) -> String {
        if self
            .document_ids
            .as_ref()
            .is_some_and(|ids| !ids.is_empty())
        {
            return "当前清单范围已限定为用户指定的文档集合。".to_string();
        }
        if self.folder_ids.as_ref().is_some_and(|ids| ids.len() > 1) {
            return "当前清单范围已限定为用户指定目录及其子目录。".to_string();
        }
        if self.folder_id.is_some() {
            return "当前清单范围已限定为用户指定目录。".to_string();
        }
        if self.library_id.is_some() {
            return "当前清单范围已限定为用户指定知识库。".to_string();
        }
        "当前清单范围为租户内可见知识库文档。".to_string()
    }
}

async fn folder_paths(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<HashMap<Uuid, String>, ListKnowledgeBaseDocumentsError> {
    let folders = kb_folders::Entity::find()
        .filter(kb_folders::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .map_err(|e| ListKnowledgeBaseDocumentsError(e.to_string()))?;
    let by_id: HashMap<Uuid, kb_folders::Model> = folders
        .into_iter()
        .map(|folder| (folder.id, folder))
        .collect();
    Ok(by_id
        .keys()
        .map(|id| (*id, build_folder_path(*id, &by_id)))
        .collect())
}

fn build_folder_path(id: Uuid, by_id: &HashMap<Uuid, kb_folders::Model>) -> String {
    let mut names = Vec::new();
    let mut current_id = Some(id);
    while let Some(id) = current_id {
        let Some(folder) = by_id.get(&id) else {
            break;
        };
        names.push(folder.name.clone());
        current_id = folder.parent_id;
    }
    names.reverse();
    names.join("/")
}
