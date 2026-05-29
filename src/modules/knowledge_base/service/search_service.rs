use uuid::Uuid;

use crate::initializers::knowledge_base::SharedSearchProvider;
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::providers::search::{SearchFilter, SearchResult};
use crate::modules::knowledge_base::providers::SharedEmbeddingClient;

use super::qa_types::Citation;

/// Perform hybrid search (dense + sparse) using the search provider.
#[tracing::instrument(skip(embedding_client, search_provider), fields(tenant_id = %tenant_id, query_len = query.len(), limit))]
pub async fn hybrid_search(
    embedding_client: &SharedEmbeddingClient,
    search_provider: &SharedSearchProvider,
    model_name: &str,
    query: &str,
    tenant_id: Uuid,
    user_id: Uuid,
    limit: usize,
    document_ids: Option<Vec<Uuid>>,
) -> Result<Vec<SearchResult>, KnowledgeBaseError> {
    use rig::client::EmbeddingsClient;
    use rig::embeddings::EmbeddingModel;

    // Embed the query
    let model = embedding_client.0.embedding_model(model_name);
    let embedding: rig::embeddings::Embedding = model
        .embed_text(query)
        .await
        .map_err(|e| KnowledgeBaseError::EmbeddingError(e.to_string()))?;

    // Convert f64 → f32
    let query_vector: Vec<f32> = embedding.vec.iter().map(|&v| v as f32).collect();

    // Build filter
    let filter = Some(SearchFilter {
        document_ids,
        min_score: None,
        user_id: Some(user_id),
    });

    // Hybrid search
    search_provider
        .search(&query_vector, query, tenant_id, limit, filter)
        .await
}

/// Convert search results to citations.
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
