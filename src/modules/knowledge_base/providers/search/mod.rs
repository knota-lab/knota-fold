pub mod qdrant;

use async_trait::async_trait;
use uuid::Uuid;

use crate::modules::knowledge_base::errors::KnowledgeBaseError;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub content: String,
    pub heading_path: Option<String>,
    pub page_number: Option<i32>,
    pub char_start: Option<i32>,
    pub char_end: Option<i32>,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct ChunkPoint {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub chunk_index: i32,
    pub content: String,
    pub heading_path: Option<String>,
    pub page_number: Option<i32>,
    pub char_start: Option<i32>,
    pub char_end: Option<i32>,
    pub token_count: i32,
    pub embedding: Vec<f32>,
    pub library_id: Option<Uuid>,
    pub folder_id: Option<Uuid>,
    // Visibility fields for Qdrant payload
    pub scope: String,    // "private" | "tenant"
    pub created_by: Uuid, // uploader user_id
}

#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub document_ids: Option<Vec<Uuid>>,
    pub library_id: Option<Uuid>,
    pub folder_id: Option<Uuid>,
    pub folder_ids: Option<Vec<Uuid>>,
    pub min_score: Option<f64>,
    pub user_id: Option<Uuid>, // for visibility filtering
}

#[async_trait]
pub trait SearchProvider: Send + Sync {
    async fn upsert_chunks(
        &self,
        chunks: &[ChunkPoint],
        tenant_id: Uuid,
    ) -> Result<(), KnowledgeBaseError>;
    async fn delete_by_document(
        &self,
        document_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<(), KnowledgeBaseError>;
    async fn search(
        &self,
        query_vector: &[f32],
        query_text: &str,
        tenant_id: Uuid,
        limit: usize,
        filter: Option<SearchFilter>,
    ) -> Result<Vec<SearchResult>, KnowledgeBaseError>;
    async fn vector_search(
        &self,
        query_vector: &[f32],
        tenant_id: Uuid,
        limit: usize,
        filter: Option<SearchFilter>,
    ) -> Result<Vec<SearchResult>, KnowledgeBaseError>;
    fn name(&self) -> &str;

    /// Update scope for all chunks of a document (private -> tenant).
    /// Used by the "promote to KB" feature.
    async fn promote_document_scope(
        &self,
        document_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<(), KnowledgeBaseError>;
}
