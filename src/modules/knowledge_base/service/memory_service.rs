//! Chat Memory Service (§19.7 — 向量化记忆模块)
//!
//! Each chat message is embedded and stored in the `chat_memory` Qdrant collection.
//! On subsequent turns, relevant history is recalled via hybrid search instead of
//! injecting full history into the prompt.

use qdrant_client::qdrant::{
    Condition, CreateCollectionBuilder, DeletePointsBuilder, Distance, Filter, Fusion,
    Modifier, NamedVectors, PointStruct, PrefetchQueryBuilder, QueryPointsBuilder,
    SparseVectorParamsBuilder, SparseVectorsConfigBuilder, UpsertPointsBuilder,
    VectorParamsBuilder,
};
use qdrant_client::Payload;
use serde_json::json;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use uuid::Uuid;

use crate::modules::knowledge_base::errors::KnowledgeBaseError;

// ── Public types ─────────────────────────────────────────────────────

/// A single recalled memory hit from vector search.
#[derive(Debug, Clone)]
pub struct MemoryHit {
    pub message_id: Uuid,
    pub role: String,
    pub content: String,
    pub score: f64,
    pub turn_index: i32,
    pub has_material: bool,
}

/// 对话记忆检索策略（§19.7.5）
#[derive(Debug, Clone)]
pub enum HistoryStrategy {
    /// 不检索历史（简单闲聊、第一轮）
    None,
    /// 向量检索 top-K 相关历史消息（大多数场景）
    RetrieveRelevant { top_k: usize },
    /// 通读当前 session 内所有含材料的原始消息（需精读的场景）
    ReadOriginalMaterial,
    /// 最近 N 轮完整注入 + 旧消息向量检索（需要连续上下文的场景）
    Hybrid { recent_turns: usize, top_k: usize },
}

/// Aggregated recall result from the memory module.
#[derive(Debug, Clone, Default)]
pub struct RecalledContext {
    /// Top-K relevant messages from vector search.
    pub relevant_messages: Vec<MemoryHit>,
    /// 最近 N 轮完整对话（Hybrid 策略时）
    pub recent_turns: Vec<(String, String)>, // (role, content)
    /// 含材料的原始消息（ReadOriginalMaterial 策略时）
    pub original_material_messages: Vec<String>,
}

// ── Core functions ───────────────────────────────────────────────────

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

fn simple_hash(s: &str) -> u32 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish() as u32
}

// ── Core functions ───────────────────────────────────────────────────

/// Embed a single text using the shared embedding client and model name.
async fn embed_text(
    embedding_client: &rig::providers::openai::Client,
    model_name: &str,
    text: &str,
) -> Result<Vec<f32>, KnowledgeBaseError> {
    use rig::client::EmbeddingsClient;
    use rig::embeddings::EmbeddingModel;

    let model = embedding_client.embedding_model(model_name);
    let embedding: rig::embeddings::Embedding = model
        .embed_text(text)
        .await
        .map_err(|e| KnowledgeBaseError::EmbeddingError(e.to_string()))?;
    Ok(embedding.vec.iter().map(|&v| v as f32).collect())
}

/// Parameters for [`index_message`].
#[derive(Debug)]
pub struct IndexMessageParams {
    pub collection_name: String,
    pub model_name: String,
    pub session_id: Uuid,
    pub tenant_id: Uuid,
    pub message_id: Uuid,
    pub role: String,
    pub content: String,
    pub has_material: bool,
    pub turn_index: i32,
}

/// Index (vectorize + upsert) a chat message into the `chat_memory` Qdrant collection.
///
/// This should be called asynchronously (fire-and-forget via `tokio::spawn`)
/// after persisting the message to SQLite.
#[tracing::instrument(skip(embedding_client, memory_provider, params))]
pub async fn index_message(
    embedding_client: &rig::providers::openai::Client,
    memory_provider: &qdrant_client::Qdrant,
    params: &IndexMessageParams,
) -> Result<(), KnowledgeBaseError> {
    // Truncate content for embedding quality
    let truncated: String = params.content.chars().take(2000).collect();

    let dense = embed_text(embedding_client, &params.model_name, &truncated).await?;

    let (sparse_indices, sparse_values) = tokenize_to_sparse(&truncated);
    let sparse_vec =
        qdrant_client::qdrant::Vector::new_sparse(sparse_indices, sparse_values);

    let payload: Payload = Payload::try_from(json!({
        "tenant_id": params.tenant_id.to_string(),
        "session_id": params.session_id.to_string(),
        "message_id": params.message_id.to_string(),
        "role": params.role,
        "content": truncated,
        "has_material": params.has_material,
        "turn_index": params.turn_index,
    }))
    .map_err(|e| KnowledgeBaseError::IndexingError(e.to_string()))?;

    let point = PointStruct::new(
        params.message_id.to_string(),
        NamedVectors::default()
            .add_vector("", dense)
            .add_vector("chat_text", sparse_vec),
        payload,
    );

    memory_provider
        .upsert_points(
            UpsertPointsBuilder::new(&params.collection_name, [point]).wait(true),
        )
        .await
        .map_err(|e| KnowledgeBaseError::IndexingError(e.to_string()))?;

    tracing::debug!(
        session_id = %params.session_id,
        message_id = %params.message_id,
        role = %params.role,
        turn_index = params.turn_index,
        "Indexed message to chat_memory"
    );

    Ok(())
}

// ── Private helpers ──────────────────────────────────────────────────

/// Parameters for [`vector_recall`].
#[derive(Debug)]
struct VectorRecallParams<'a> {
    collection_name: &'a str,
    query_text: &'a str,
    top_k: usize,
}

/// Core vector recall: hybrid search (dense + sparse + RRF) on chat_memory collection.
async fn vector_recall(
    embedding_client: &rig::providers::openai::Client,
    memory_provider: &qdrant_client::Qdrant,
    model_name: &str,
    session_id: Uuid,
    tenant_id: Uuid,
    params: &VectorRecallParams<'_>,
) -> Result<Vec<MemoryHit>, KnowledgeBaseError> {
    let query_vector =
        embed_text(embedding_client, model_name, params.query_text).await?;
    let (sparse_indices, sparse_values) = tokenize_to_sparse(params.query_text);
    let sparse_query: Vec<(u32, f32)> =
        sparse_indices.into_iter().zip(sparse_values).collect();

    let filter = Filter::must([
        Condition::matches("tenant_id", tenant_id.to_string()),
        Condition::matches("session_id", session_id.to_string()),
    ]);

    let response = memory_provider
        .query(
            QueryPointsBuilder::new(params.collection_name)
                .add_prefetch(
                    PrefetchQueryBuilder::default()
                        .query(query_vector)
                        .limit(50u64),
                )
                .add_prefetch(
                    PrefetchQueryBuilder::default()
                        .query(sparse_query)
                        .using("chat_text")
                        .limit(50u64),
                )
                .query(Fusion::Rrf)
                .filter(filter)
                .limit(params.top_k as u64)
                .with_payload(true),
        )
        .await
        .map_err(|e| KnowledgeBaseError::ProviderError(e.to_string()))?;

    let hits: Vec<MemoryHit> = response
        .result
        .into_iter()
        .filter_map(|r| {
            let payload = r.payload;
            let message_id = payload.get("message_id").and_then(|v| match &v.kind {
                Some(qdrant_client::qdrant::value::Kind::StringValue(s)) => {
                    Uuid::parse_str(s).ok()
                }
                _ => None,
            })?;
            let role = payload
                .get("role")
                .and_then(|v| match &v.kind {
                    Some(qdrant_client::qdrant::value::Kind::StringValue(s)) => {
                        Some(s.clone())
                    }
                    _ => None,
                })
                .unwrap_or_default();
            let content = payload
                .get("content")
                .and_then(|v| match &v.kind {
                    Some(qdrant_client::qdrant::value::Kind::StringValue(s)) => {
                        Some(s.clone())
                    }
                    _ => None,
                })
                .unwrap_or_default();
            let turn_index = payload
                .get("turn_index")
                .and_then(|v| match &v.kind {
                    Some(qdrant_client::qdrant::value::Kind::IntegerValue(i)) => {
                        Some(*i as i32)
                    }
                    _ => None,
                })
                .unwrap_or(0);
            let has_material = payload
                .get("has_material")
                .and_then(|v| match &v.kind {
                    Some(qdrant_client::qdrant::value::Kind::BoolValue(b)) => Some(*b),
                    _ => None,
                })
                .unwrap_or(false);

            Some(MemoryHit {
                message_id,
                role,
                content,
                score: r.score as f64,
                turn_index,
                has_material,
            })
        })
        .collect();

    Ok(hits)
}

/// Extract the N most recent complete turns (user + assistant pairs) from DB messages.
/// Returns Vec<(role, content)> ordered chronologically (oldest first within the window).
fn extract_recent_turns(
    history_db: &[crate::models::_entities::chat_messages::Model],
    turn_count: usize,
) -> Vec<(String, String)> {
    // A "turn" = user + assistant pair. We want the last `turn_count` pairs.
    // Take the last turn_count * 2 messages, then collect chronologically.
    let take_count = turn_count * 2;
    let start = history_db.len().saturating_sub(take_count);
    history_db[start..]
        .iter()
        .map(|m| (m.role.clone(), m.content.chars().take(800).collect()))
        .collect()
}

/// Parameters for [`recall_history`].
#[derive(Debug)]
pub struct RecallHistoryParams<'a> {
    pub session_id: Uuid,
    pub tenant_id: Uuid,
    pub query: &'a str,
    pub history_db: &'a [crate::models::_entities::chat_messages::Model],
}

/// Recall relevant history messages based on intent-driven strategy (§19.7.5).
///
/// Dispatches to the appropriate retrieval method based on `HistoryStrategy`.
/// The DB messages are passed in for strategies that read directly from SQLite
/// (ReadOriginalMaterial, Hybrid recent_turns).
#[tracing::instrument(skip(
    embedding_client,
    memory_provider,
    collection_name,
    model_name,
    strategy,
    params
))]
pub async fn recall_history(
    embedding_client: &rig::providers::openai::Client,
    memory_provider: &qdrant_client::Qdrant,
    collection_name: &str,
    model_name: &str,
    strategy: &HistoryStrategy,
    params: &RecallHistoryParams<'_>,
) -> Result<RecalledContext, KnowledgeBaseError> {
    match strategy {
        HistoryStrategy::None => {
            tracing::debug!(session_id = %params.session_id, "Strategy: None — skipping memory recall");
            Ok(RecalledContext::default())
        }

        HistoryStrategy::RetrieveRelevant { top_k } => {
            let relevant = vector_recall(
                embedding_client,
                memory_provider,
                model_name,
                params.session_id,
                params.tenant_id,
                &VectorRecallParams {
                    collection_name,
                    query_text: params.query,
                    top_k: *top_k,
                },
            )
            .await?;
            tracing::debug!(
                session_id = %params.session_id,
                hits = relevant.len(),
                "Strategy: RetrieveRelevant"
            );
            Ok(RecalledContext {
                relevant_messages: relevant,
                ..Default::default()
            })
        }

        HistoryStrategy::ReadOriginalMaterial => {
            // 从 DB 直接读取含材料的用户消息（content 已含格式化后的完整材料内容）
            let original: Vec<String> = params
                .history_db
                .iter()
                .filter(|m| m.material_refs.is_some())
                .filter(|m| m.role == "user")
                .map(|m| {
                    let content: String = m.content.chars().take(2000).collect();
                    let ellipsis = if m.content.chars().count() > 2000 {
                        "…"
                    } else {
                        ""
                    };
                    format!("{content}{ellipsis}")
                })
                .collect();
            tracing::debug!(
                session_id = %params.session_id,
                material_messages = original.len(),
                "Strategy: ReadOriginalMaterial"
            );
            Ok(RecalledContext {
                original_material_messages: original,
                ..Default::default()
            })
        }

        HistoryStrategy::Hybrid {
            recent_turns,
            top_k,
        } => {
            // 1. 从 DB 取最近 N 轮完整对话
            let recent = extract_recent_turns(params.history_db, *recent_turns);

            // 2. 同时做向量检索 top_k 条相关旧消息（去重已取的最近轮次）
            let recent_msg_ids: std::collections::HashSet<Uuid> = params
                .history_db
                .iter()
                .rev()
                .take(*recent_turns * 2) // each turn = user + assistant
                .map(|m| m.id)
                .collect();

            let relevant = vector_recall(
                embedding_client,
                memory_provider,
                model_name,
                params.session_id,
                params.tenant_id,
                &VectorRecallParams {
                    collection_name,
                    query_text: params.query,
                    top_k: *top_k,
                },
            )
            .await?;

            let filtered: Vec<MemoryHit> = relevant
                .into_iter()
                .filter(|h| !recent_msg_ids.contains(&h.message_id))
                .collect();

            tracing::debug!(
                session_id = %params.session_id,
                recent_count = recent.len(),
                vector_hits = filtered.len(),
                "Strategy: Hybrid"
            );

            Ok(RecalledContext {
                relevant_messages: filtered,
                recent_turns: recent,
                ..Default::default()
            })
        }
    }
}

/// Ensure the `chat_memory` collection exists in Qdrant with the right schema.
/// Called during initialization.
pub async fn ensure_collection(
    client: &qdrant_client::Qdrant,
    collection_name: &str,
    dimension: usize,
) -> Result<(), KnowledgeBaseError> {
    let map_err =
        |e: qdrant_client::QdrantError| KnowledgeBaseError::ProviderError(e.to_string());

    if !client
        .collection_exists(collection_name)
        .await
        .map_err(map_err)?
    {
        let mut sparse_config = SparseVectorsConfigBuilder::default();
        sparse_config.add_named_vector_params(
            "chat_text",
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
            .map_err(map_err)?;

        tracing::info!(
            collection = collection_name,
            "Created chat_memory collection"
        );
    }

    Ok(())
}

/// Delete all memory vectors for a given session.
/// Called when a chat session is deleted.
pub async fn delete_by_session(
    client: &qdrant_client::Qdrant,
    collection_name: &str,
    session_id: Uuid,
    tenant_id: Uuid,
) -> Result<(), KnowledgeBaseError> {
    let filter = Filter::must([
        Condition::matches("tenant_id", tenant_id.to_string()),
        Condition::matches("session_id", session_id.to_string()),
    ]);

    client
        .delete_points(
            DeletePointsBuilder::new(collection_name)
                .points(filter)
                .wait(true),
        )
        .await
        .map_err(|e| KnowledgeBaseError::ProviderError(e.to_string()))?;

    tracing::info!(
        session_id = %session_id,
        "Deleted chat_memory vectors for session"
    );

    Ok(())
}

/// Format recalled context into a text block for injection into the LLM prompt.
/// Handles all three context types: relevant_messages, recent_turns, original_material_messages.
pub fn format_recalled_context(ctx: &RecalledContext) -> String {
    let mut parts = Vec::new();

    // 1. Original material messages (ReadOriginalMaterial strategy)
    if !ctx.original_material_messages.is_empty() {
        parts.push("--- 原始材料历史 ---".to_string());
        for (i, msg) in ctx.original_material_messages.iter().enumerate() {
            let content: String = msg.chars().take(1500).collect();
            let ellipsis = if msg.chars().count() > 1500 {
                "…"
            } else {
                ""
            };
            parts.push(format!("[材料 {}]: {}{}", i + 1, content, ellipsis));
        }
        parts.push("--- 原始材料历史结束 ---".to_string());
    }

    // 2. Relevant messages from vector search (RetrieveRelevant / Hybrid strategy)
    if !ctx.relevant_messages.is_empty() {
        parts.push("--- 相关历史对话 ---".to_string());
        for hit in &ctx.relevant_messages {
            let label = match hit.role.as_str() {
                "user" => "用户",
                "assistant" => "助手",
                _ => "系统",
            };
            let content: String = hit.content.chars().take(800).collect();
            let ellipsis = if hit.content.chars().count() > 800 {
                "…"
            } else {
                ""
            };
            parts.push(format!("[{label}]: {content}{ellipsis}"));
        }
        parts.push("--- 相关历史对话结束 ---".to_string());
    }

    // 3. Recent turns (Hybrid strategy)
    if !ctx.recent_turns.is_empty() {
        parts.push("--- 最近对话 ---".to_string());
        for (role, content) in &ctx.recent_turns {
            let label = match role.as_str() {
                "user" => "用户",
                "assistant" => "助手",
                _ => "系统",
            };
            let truncated: String = content.chars().take(800).collect();
            let ellipsis = if content.chars().count() > 800 {
                "…"
            } else {
                ""
            };
            parts.push(format!("[{label}]: {truncated}{ellipsis}"));
        }
        parts.push("--- 最近对话结束 ---".to_string());
    }

    if parts.is_empty() {
        return String::new();
    }

    format!("\n{}\n", parts.join("\n\n"))
}
