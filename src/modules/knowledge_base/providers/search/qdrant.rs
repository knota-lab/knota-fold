use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use async_trait::async_trait;
use qdrant_client::qdrant::Value;
use qdrant_client::qdrant::{
    point_id::PointIdOptions, value::Kind as ValueKind, Condition,
    CreateCollectionBuilder, DeletePointsBuilder, Distance, Filter, Fusion, Modifier,
    NamedVectors, PointStruct, PrefetchQueryBuilder, QueryPointsBuilder,
    SetPayloadPointsBuilder, SparseVectorParamsBuilder, SparseVectorsConfigBuilder,
    UpsertPointsBuilder, VectorParamsBuilder,
};
use qdrant_client::Payload;
use qdrant_client::Qdrant;
use serde_json::json;
use uuid::Uuid;

use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::providers::search::{
    ChunkPoint, SearchFilter, SearchProvider, SearchResult,
};

// ── Struct ────────────────────────────────────────────────────────────

pub struct QdrantSearchProvider {
    client: Qdrant,
    collection_name: String,
}

fn map_err(e: &qdrant_client::QdrantError) -> KnowledgeBaseError {
    KnowledgeBaseError::ProviderError(e.to_string())
}

impl QdrantSearchProvider {
    pub async fn new(
        url: &str,
        api_key: Option<&str>,
        dimension: usize,
        collection_name: &str,
    ) -> Result<Self, KnowledgeBaseError> {
        let client = Qdrant::from_url(url)
            .api_key(api_key)
            .build()
            .map_err(|e| map_err(&e))?;

        if !client
            .collection_exists(collection_name)
            .await
            .map_err(|e| map_err(&e))?
        {
            let mut sparse_config = SparseVectorsConfigBuilder::default();
            sparse_config.add_named_vector_params(
                "chunk_text",
                SparseVectorParamsBuilder::default().modifier(Modifier::Idf),
            );
            client
                .create_collection(
                    CreateCollectionBuilder::new(collection_name)
                        .vectors_config(VectorParamsBuilder::new(
                            dimension as u64,
                            Distance::Cosine,
                        ))
                        .sparse_vectors_config(sparse_config),
                )
                .await
                .map_err(|e| map_err(&e))?;
        }

        Ok(Self {
            client,
            collection_name: collection_name.to_string(),
        })
    }
}

// ── Tokenization helpers ─────────────────────────────────────────────

/// Phase 1: Simple character-level tokenization for sparse vectors.
/// Splits by whitespace/CJK, hashes each token to u32, counts frequency.
fn tokenize_to_sparse(text: &str) -> (Vec<u32>, Vec<f32>) {
    let mut token_freq: HashMap<u32, f32> = HashMap::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !current.is_empty() {
                let hash = simple_hash(&current);
                *token_freq.entry(hash).or_insert(0.0) += 1.0;
                current.clear();
            }
        } else if ch.len_utf8() > 1 {
            if !current.is_empty() {
                let hash = simple_hash(&current);
                *token_freq.entry(hash).or_insert(0.0) += 1.0;
                current.clear();
            }
            let hash = simple_hash(&ch.to_string());
            *token_freq.entry(hash).or_insert(0.0) += 1.0;
        } else {
            current.push(ch.to_ascii_lowercase());
        }
    }
    if !current.is_empty() {
        let hash = simple_hash(&current);
        *token_freq.entry(hash).or_insert(0.0) += 1.0;
    }
    let mut indices: Vec<u32> = token_freq.keys().copied().collect();
    indices.sort_unstable();
    let values: Vec<f32> = indices.iter().map(|i| token_freq[i]).collect();
    (indices, values)
}

/// Simple deterministic string hash to u32.
fn simple_hash(s: &str) -> u32 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish() as u32
}

// ── Payload extraction helpers ───────────────────────────────────────

fn extract_string(payload: &HashMap<String, Value>, key: &str) -> Option<String> {
    payload.get(key).and_then(|v| match &v.kind {
        Some(ValueKind::StringValue(s)) => Some(s.clone()),
        _ => None,
    })
}

fn extract_i32(payload: &HashMap<String, Value>, key: &str) -> Option<i32> {
    payload.get(key).and_then(|v| match &v.kind {
        Some(ValueKind::IntegerValue(i)) => Some(*i as i32),
        _ => None,
    })
}

fn extract_uuid(payload: &HashMap<String, Value>, key: &str) -> Option<Uuid> {
    extract_string(payload, key).and_then(|s| Uuid::parse_str(&s).ok())
}

fn extract_point_id(id: &qdrant_client::qdrant::PointId) -> Option<String> {
    match &id.point_id_options {
        Some(PointIdOptions::Uuid(s)) => Some(s.clone()),
        _ => None,
    }
}

// ── Filter builder ───────────────────────────────────────────────────

fn build_filter(tenant_id: Uuid, filter: Option<&SearchFilter>) -> Filter {
    let tenant_cond = Condition::matches("tenant_id", tenant_id.to_string());

    // Visibility: (scope=tenant) OR (scope=private AND created_by=user)
    let visibility = filter.as_ref().and_then(|f| f.user_id).map_or_else(
        || Filter::should([Condition::matches("scope", "tenant".to_string())]),
        |uid| {
            Filter::should([
                Condition::matches("scope", "tenant".to_string()),
                Filter::must([
                    Condition::matches("scope", "private".to_string()),
                    Condition::matches("created_by", uid.to_string()),
                ])
                .into(),
            ])
        },
    );

    let mut must: Vec<qdrant_client::qdrant::Condition> =
        vec![tenant_cond, visibility.into()];

    if let Some(f) = filter {
        if let Some(doc_ids) = &f.document_ids {
            if !doc_ids.is_empty() {
                let doc_conds: Vec<Condition> = doc_ids
                    .iter()
                    .map(|id| Condition::matches("document_id", id.to_string()))
                    .collect();
                must.push(Filter::should(doc_conds).into());
            }
        }
    }

    Filter::must(must)
}

// ── Map ScoredPoints → SearchResults ─────────────────────────────────

fn map_scored_points(
    points: Vec<qdrant_client::qdrant::ScoredPoint>,
) -> Vec<SearchResult> {
    points
        .into_iter()
        .filter_map(|r| {
            let id_str = r.id.as_ref().and_then(extract_point_id)?;
            Some(SearchResult {
                chunk_id: Uuid::parse_str(&id_str).ok()?,
                document_id: extract_uuid(&r.payload, "document_id")?,
                content: extract_string(&r.payload, "content").unwrap_or_default(),
                heading_path: extract_string(&r.payload, "heading_path"),
                page_number: extract_i32(&r.payload, "page_number"),
                char_start: extract_i32(&r.payload, "char_start"),
                char_end: extract_i32(&r.payload, "char_end"),
                score: r.score as f64,
            })
        })
        .collect()
}

// ── SearchProvider impl ──────────────────────────────────────────────

#[async_trait]
impl SearchProvider for QdrantSearchProvider {
    async fn upsert_chunks(
        &self,
        chunks: &[ChunkPoint],
        tenant_id: Uuid,
    ) -> Result<(), KnowledgeBaseError> {
        let mut points = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let (sparse_indices, sparse_values) = tokenize_to_sparse(&chunk.content);
            let sparse_vec =
                qdrant_client::qdrant::Vector::new_sparse(sparse_indices, sparse_values);

            let payload: Payload = Payload::try_from(json!({
                "tenant_id": tenant_id.to_string(),
                "document_id": chunk.document_id.to_string(),
                "scope": chunk.scope,
                "created_by": chunk.created_by.to_string(),
                "chunk_index": chunk.chunk_index,
                "content": chunk.content,
                "heading_path": chunk.heading_path,
                "page_number": chunk.page_number,
                "token_count": chunk.token_count,
            }))
            .map_err(|e| KnowledgeBaseError::IndexingError(e.to_string()))?;

            let point = PointStruct::new(
                chunk.chunk_id.to_string(),
                NamedVectors::default()
                    .add_vector("", chunk.embedding.clone())
                    .add_vector("chunk_text", sparse_vec),
                payload,
            );
            points.push(point);
        }

        self.client
            .upsert_points(
                UpsertPointsBuilder::new(&self.collection_name, points).wait(true),
            )
            .await
            .map_err(|e| map_err(&e))?;

        Ok(())
    }

    async fn delete_by_document(
        &self,
        document_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<(), KnowledgeBaseError> {
        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection_name)
                    .points(Filter::must([
                        Condition::matches("tenant_id", tenant_id.to_string()),
                        Condition::matches("document_id", document_id.to_string()),
                    ]))
                    .wait(true),
            )
            .await
            .map_err(|e| map_err(&e))?;

        Ok(())
    }

    async fn search(
        &self,
        query_vector: &[f32],
        query_text: &str,
        tenant_id: Uuid,
        limit: usize,
        filter: Option<SearchFilter>,
    ) -> Result<Vec<SearchResult>, KnowledgeBaseError> {
        let (sparse_indices, sparse_values) = tokenize_to_sparse(query_text);
        let sparse_query: Vec<(u32, f32)> =
            sparse_indices.into_iter().zip(sparse_values).collect();

        let qdrant_filter = build_filter(tenant_id, filter.as_ref());

        let response = self
            .client
            .query(
                QueryPointsBuilder::new(&self.collection_name)
                    .add_prefetch(
                        PrefetchQueryBuilder::default()
                            .query(query_vector.to_vec())
                            .limit(50u64),
                    )
                    .add_prefetch(
                        PrefetchQueryBuilder::default()
                            .query(sparse_query)
                            .using("chunk_text")
                            .limit(50u64),
                    )
                    .query(Fusion::Rrf)
                    .filter(qdrant_filter)
                    .limit(limit as u64)
                    .with_payload(true),
            )
            .await
            .map_err(|e| map_err(&e))?;

        Ok(map_scored_points(response.result))
    }

    async fn vector_search(
        &self,
        query_vector: &[f32],
        tenant_id: Uuid,
        limit: usize,
        filter: Option<SearchFilter>,
    ) -> Result<Vec<SearchResult>, KnowledgeBaseError> {
        let qdrant_filter = build_filter(tenant_id, filter.as_ref());

        let response = self
            .client
            .query(
                QueryPointsBuilder::new(&self.collection_name)
                    .query(query_vector.to_vec())
                    .filter(qdrant_filter)
                    .limit(limit as u64)
                    .with_payload(true),
            )
            .await
            .map_err(|e| map_err(&e))?;

        Ok(map_scored_points(response.result))
    }

    async fn promote_document_scope(
        &self,
        document_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<(), KnowledgeBaseError> {
        let payload: Payload = Payload::try_from(json!({"scope": "tenant"}))
            .map_err(|e| KnowledgeBaseError::IndexingError(e.to_string()))?;

        let filter = Filter::must([
            Condition::matches("tenant_id", tenant_id.to_string()),
            Condition::matches("document_id", document_id.to_string()),
        ]);

        self.client
            .set_payload(
                SetPayloadPointsBuilder::new(&self.collection_name, payload)
                    .points_selector(filter)
                    .wait(true),
            )
            .await
            .map_err(|e| map_err(&e))?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "qdrant"
    }
}
