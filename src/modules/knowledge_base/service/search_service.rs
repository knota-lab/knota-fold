use uuid::Uuid;

use crate::initializers::knowledge_base::SharedSearchProvider;
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::providers::search::{SearchFilter, SearchResult};
use crate::modules::knowledge_base::providers::SharedEmbeddingClient;
use crate::modules::knowledge_base::service::numeric::embedding_vec_f64_to_f32;

use super::qa_types::Citation;

/// Parameters for [`hybrid_search`].
#[derive(Debug)]
pub struct HybridSearchParams {
    pub model_name: String,
    pub query: String,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub limit: usize,
    pub library_id: Option<Uuid>,
    pub folder_id: Option<Uuid>,
    pub document_ids: Option<Vec<Uuid>>,
}

/// Perform hybrid search (dense + sparse) using the search provider.
#[tracing::instrument(skip(embedding_client, search_provider, params), fields(tenant_id = %params.tenant_id, query_len = params.query.len(), limit = params.limit))]
pub async fn hybrid_search(
    embedding_client: &SharedEmbeddingClient,
    search_provider: &SharedSearchProvider,
    params: &HybridSearchParams,
) -> Result<Vec<SearchResult>, KnowledgeBaseError> {
    use rig::client::EmbeddingsClient;
    use rig::embeddings::EmbeddingModel;

    // Embed the query
    let model = embedding_client.0.embedding_model(&params.model_name);
    let embedding: rig::embeddings::Embedding = model
        .embed_text(&params.query)
        .await
        .map_err(|e| KnowledgeBaseError::EmbeddingError(e.to_string()))?;

    let query_vector = embedding_vec_f64_to_f32(&embedding.vec);

    // Build filter
    let filter = Some(SearchFilter {
        document_ids: params.document_ids.clone(),
        library_id: params.library_id,
        folder_id: params.folder_id,
        min_score: None,
        user_id: Some(params.user_id),
    });

    // Hybrid search
    search_provider
        .search(
            &query_vector,
            &params.query,
            params.tenant_id,
            params.limit,
            filter,
        )
        .await
}

/// Convert search results to citations.
#[must_use]
pub fn results_to_citations(results: &[SearchResult]) -> Vec<Citation> {
    results
        .iter()
        .map(|r| Citation {
            document_id: r.document_id,
            chunk_id: Some(r.chunk_id),
            content: r.content.clone(),
            score: r.score,
        })
        .collect()
}
