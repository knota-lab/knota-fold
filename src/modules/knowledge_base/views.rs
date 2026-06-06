use crate::models::_entities::kb_documents;
use serde::{Deserialize, Serialize};

// ---- Request types ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDocumentRequest {
    pub title: String,
    pub description: Option<String>,
    /// MIME type used by inline `content` parsing. For `fileId` documents the
    /// worker uses the file record's `mimeType`.
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
    pub status: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    pub limit: Option<usize>,
    pub document_ids: Option<Vec<uuid::Uuid>>,
}

// ---- Response types ----

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentResponse {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub source_type: String,
    pub scope: String,
    pub file_id: Option<String>,
    pub status: String,
    pub chunk_count: i32,
    pub total_tokens: i32,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl DocumentResponse {
    #[must_use]
    pub fn from_model(m: &kb_documents::Model) -> Self {
        Self {
            id: m.id.to_string(),
            title: m.title.clone(),
            description: m.description.clone(),
            source_type: m.source_type.clone(),
            scope: m.scope.clone(),
            file_id: m.file_id.map(|id| id.to_string()),
            status: m.status.clone(),
            chunk_count: m.chunk_count,
            total_tokens: m.total_tokens,
            error_message: m.error_message.clone(),
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
