use crate::models::_entities::{kb_documents, kb_folders, kb_libraries};
use serde::{Deserialize, Serialize};

// ---- Request types ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDocumentRequest {
    pub title: String,
    pub description: Option<String>,
    pub library_id: Option<uuid::Uuid>,
    pub folder_id: Option<uuid::Uuid>,
    /// MIME type observed by the business flow. For file-backed documents it is
    /// copied into the file reference snapshot instead of mutating `files`.
    /// Defaults to `text/plain` when inline `content` is present.
    pub source_type: Option<String>,
    /// Document visibility: "private" (only uploader) or "tenant" (shared in tenant).
    /// Defaults to "tenant" when omitted.
    pub scope: Option<String>,
    pub file_id: Option<uuid::Uuid>,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentListQuery {
    pub page: Option<u64>,
    pub page_size: Option<u64>,
    pub library_id: Option<uuid::Uuid>,
    pub folder_id: Option<uuid::Uuid>,
    pub status: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    pub limit: Option<usize>,
    pub library_id: Option<uuid::Uuid>,
    pub folder_id: Option<uuid::Uuid>,
    pub document_ids: Option<Vec<uuid::Uuid>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresignDocumentAssetsRequest {
    pub asset_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateLibraryRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLibraryRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderListQuery {
    pub library_id: uuid::Uuid,
    pub parent_id: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderRequest {
    pub library_id: uuid::Uuid,
    pub parent_id: Option<uuid::Uuid>,
    pub name: String,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFolderRequest {
    pub name: String,
    #[serde(default)]
    pub sort_order: i32,
}

// ---- Response types ----

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexingProgressResponse {
    pub stage: String,
    pub label: String,
    pub message: Option<String>,
    pub current: Option<i32>,
    pub total: Option<i32>,
    pub stage_started_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentResponse {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub library_id: Option<String>,
    pub folder_id: Option<String>,
    pub source_type: String,
    pub scope: String,
    pub file_id: Option<String>,
    pub file_reference_id: Option<String>,
    pub status: String,
    pub chunk_count: i32,
    pub total_tokens: i32,
    pub indexing_progress: Option<IndexingProgressResponse>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reused_existing: Option<bool>,
}

impl DocumentResponse {
    #[must_use]
    pub fn from_model(m: &kb_documents::Model) -> Self {
        Self {
            id: m.id.to_string(),
            title: m.title.clone(),
            description: m.description.clone(),
            library_id: m.library_id.map(|id| id.to_string()),
            folder_id: m.folder_id.map(|id| id.to_string()),
            source_type: m.source_type.clone(),
            scope: m.scope.clone(),
            file_id: m.file_id.map(|id| id.to_string()),
            file_reference_id: m.file_reference_id.map(|id| id.to_string()),
            status: m.status.clone(),
            chunk_count: m.chunk_count,
            total_tokens: m.total_tokens,
            indexing_progress: indexing_progress_from_metadata(m.metadata.as_ref()),
            error_message: m.error_message.clone(),
            created_at: m.created_at.and_utc().to_rfc3339(),
            updated_at: m.updated_at.and_utc().to_rfc3339(),
            reused_existing: None,
        }
    }

    #[must_use]
    pub fn from_reused_model(m: &kb_documents::Model) -> Self {
        Self {
            reused_existing: Some(true),
            ..Self::from_model(m)
        }
    }
}

fn indexing_progress_from_metadata(
    metadata: Option<&serde_json::Value>,
) -> Option<IndexingProgressResponse> {
    let indexing = metadata?.get("indexing")?;
    let stage = indexing.get("stage")?.as_str()?.to_string();
    let label = indexing
        .get("label")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&stage)
        .to_string();
    let message = indexing
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(std::string::ToString::to_string);
    let stage_started_at = indexing
        .get("stageStartedAt")
        .and_then(serde_json::Value::as_str)
        .map(std::string::ToString::to_string);
    let current = indexing
        .get("current")
        .and_then(serde_json::Value::as_i64)
        .and_then(|value| i32::try_from(value).ok());
    let total = indexing
        .get("total")
        .and_then(serde_json::Value::as_i64)
        .and_then(|value| i32::try_from(value).ok());

    Some(IndexingProgressResponse {
        stage,
        label,
        message,
        current,
        total,
        stage_started_at,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

impl LibraryResponse {
    #[must_use]
    pub fn from_model(m: &kb_libraries::Model) -> Self {
        Self {
            id: m.id.to_string(),
            name: m.name.clone(),
            description: m.description.clone(),
            sort_order: m.sort_order,
            created_by: m.created_by.to_string(),
            created_at: m.created_at.and_utc().to_rfc3339(),
            updated_at: m.updated_at.and_utc().to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderResponse {
    pub id: String,
    pub library_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub path: String,
    pub depth: i32,
    pub sort_order: i32,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

impl FolderResponse {
    #[must_use]
    pub fn from_model(m: &kb_folders::Model) -> Self {
        Self {
            id: m.id.to_string(),
            library_id: m.library_id.to_string(),
            parent_id: m.parent_id.map(|id| id.to_string()),
            name: m.name.clone(),
            path: m.path.clone(),
            depth: m.depth,
            sort_order: m.sort_order,
            created_by: m.created_by.to_string(),
            created_at: m.created_at.and_utc().to_rfc3339(),
            updated_at: m.updated_at.and_utc().to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultResponse {
    pub chunk_id: String,
    pub document_id: String,
    pub content: String,
    pub heading_path: Option<String>,
    pub score: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkResponse {
    pub id: String,
    pub document_id: String,
    pub chunk_index: i32,
    pub content: String,
    pub heading_path: Option<String>,
    pub token_count: i32,
    pub char_start: Option<i32>,
    pub char_end: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentAssetResponse {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub storage_key: String,
    pub size: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPreviewResponse {
    pub document_id: String,
    pub title: String,
    pub markdown: String,
    pub assets: Vec<DocumentAssetResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresignedDocumentAssetResponse {
    pub asset_key: String,
    pub url: String,
    pub expires_in: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresignDocumentAssetsResponse {
    pub items: Vec<PresignedDocumentAssetResponse>,
}
