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

use crate::models::_entities::{kb_documents, kb_folders, kb_libraries};

#[derive(Debug)]
pub struct ListKnowledgeBaseScopeError(pub String);

impl fmt::Display for ListKnowledgeBaseScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "list knowledge base scope error: {}", self.0)
    }
}

impl std::error::Error for ListKnowledgeBaseScopeError {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListKnowledgeBaseScopeArgs {
    /// Max libraries to return. Defaults to 20, max 50.
    max_libraries: Option<u64>,
    /// Max folders to return. Defaults to 80, max 200.
    max_folders: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ListKnowledgeBaseScopeTool {
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
}

impl Tool for ListKnowledgeBaseScopeTool {
    const NAME: &'static str = "list_knowledge_base_scope";

    type Error = ListKnowledgeBaseScopeError;
    type Args = ListKnowledgeBaseScopeArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "列出当前用户可见的知识库和目录范围摘要。适合回答“你现在能访问哪些知识库/目录”。只返回结构摘要，不返回文档正文。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "maxLibraries": {
                        "type": "integer",
                        "description": "返回的最大知识库数量（默认20，最大50）"
                    },
                    "maxFolders": {
                        "type": "integer",
                        "description": "返回的最大目录数量（默认80，最大200）"
                    }
                },
                "required": []
            }),
        }
    }

    #[tracing::instrument(
        skip(self, args),
        fields(tool = "list_knowledge_base_scope", tenant_id = %self.tenant_id)
    )]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let max_libraries = args.max_libraries.unwrap_or(20).clamp(1, 50);
        let max_folders = args.max_folders.unwrap_or(80).clamp(1, 200);

        let libraries = self.query_libraries(max_libraries).await?;
        if libraries.is_empty() {
            return Ok("当前租户下没有可见知识库。".to_string());
        }
        let folders = self.query_folders(max_folders).await?;
        let doc_counts = self.document_counts().await?;

        let folders_by_library = group_folders_by_library(folders);
        let mut lines = Vec::new();
        lines.push("当前可见知识库范围：".to_string());
        for library in &libraries {
            let library_doc_count = doc_counts
                .library_counts
                .get(&library.id)
                .copied()
                .unwrap_or(0);
            lines.push(format!(
                "- {} (ID: {}, 可用文档: {}){}",
                library.name,
                library.id,
                library_doc_count,
                library
                    .description
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map_or_else(String::new, |desc| format!("，描述: {desc}")),
            ));

            if let Some(folders) = folders_by_library.get(&library.id) {
                for folder in folders {
                    let indent = "  "
                        .repeat(usize::try_from(folder.depth).unwrap_or_default() + 1);
                    let folder_doc_count = doc_counts
                        .folder_counts
                        .get(&folder.id)
                        .copied()
                        .unwrap_or(0);
                    lines.push(format!(
                        "{indent}- {} (ID: {}, 可用文档: {})",
                        folder.name, folder.id, folder_doc_count
                    ));
                }
            }
        }

        Ok(lines.join("\n"))
    }
}

impl ListKnowledgeBaseScopeTool {
    async fn query_libraries(
        &self,
        max_libraries: u64,
    ) -> Result<Vec<kb_libraries::Model>, ListKnowledgeBaseScopeError> {
        let mut query = kb_libraries::Entity::find()
            .filter(kb_libraries::Column::TenantId.eq(self.tenant_id));
        if let Some(library_id) = self.library_id {
            query = query.filter(kb_libraries::Column::Id.eq(library_id));
        }
        query
            .order_by_asc(kb_libraries::Column::SortOrder)
            .order_by_asc(kb_libraries::Column::CreatedAt)
            .limit(max_libraries)
            .all(&*self.db)
            .await
            .map_err(|e| ListKnowledgeBaseScopeError(e.to_string()))
    }

    async fn query_folders(
        &self,
        max_folders: u64,
    ) -> Result<Vec<kb_folders::Model>, ListKnowledgeBaseScopeError> {
        let mut query = kb_folders::Entity::find()
            .filter(kb_folders::Column::TenantId.eq(self.tenant_id));
        if let Some(library_id) = self.library_id {
            query = query.filter(kb_folders::Column::LibraryId.eq(library_id));
        }
        if let Some(folder_ids) = &self.folder_ids {
            if !folder_ids.is_empty() {
                query = query.filter(kb_folders::Column::Id.is_in(folder_ids.clone()));
            }
        } else if let Some(folder_id) = self.folder_id {
            query = query.filter(kb_folders::Column::Id.eq(folder_id));
        }
        query
            .order_by_asc(kb_folders::Column::LibraryId)
            .order_by_asc(kb_folders::Column::Path)
            .limit(max_folders)
            .all(&*self.db)
            .await
            .map_err(|e| ListKnowledgeBaseScopeError(e.to_string()))
    }

    async fn document_counts(
        &self,
    ) -> Result<DocumentCounts, ListKnowledgeBaseScopeError> {
        let mut query = kb_documents::Entity::find()
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

        let docs = query
            .all(&*self.db)
            .await
            .map_err(|e| ListKnowledgeBaseScopeError(e.to_string()))?;

        Ok(count_documents(docs))
    }
}

fn group_folders_by_library(
    folders: Vec<kb_folders::Model>,
) -> HashMap<Uuid, Vec<kb_folders::Model>> {
    let mut result: HashMap<Uuid, Vec<kb_folders::Model>> = HashMap::new();
    for folder in folders {
        result.entry(folder.library_id).or_default().push(folder);
    }
    result
}

struct DocumentCounts {
    library_counts: HashMap<Uuid, usize>,
    folder_counts: HashMap<Uuid, usize>,
}

fn count_documents(docs: Vec<kb_documents::Model>) -> DocumentCounts {
    let mut library_counts = HashMap::new();
    let mut folder_counts = HashMap::new();
    for doc in docs {
        if let Some(library_id) = doc.library_id {
            *library_counts.entry(library_id).or_insert(0) += 1;
        }
        if let Some(folder_id) = doc.folder_id {
            *folder_counts.entry(folder_id).or_insert(0) += 1;
        }
    }

    DocumentCounts {
        library_counts,
        folder_counts,
    }
}
