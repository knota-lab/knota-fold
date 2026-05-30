use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use tokio::sync::mpsc;

use crate::config::{AppSettings, ConfigExt};
use crate::extractors::TenantContext;
use crate::initializers::knowledge_base::{
    SessionLockMap, SharedMemoryStore, SharedSearchProvider,
};
use crate::models::_entities::kb_chunks;
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::providers::{SharedEmbeddingClient, SharedQaClient};
use crate::modules::knowledge_base::service::tools::tool_result_broker::{
    ResolveOutcome, ToolResult, ToolResultBroker,
};
use crate::modules::knowledge_base::service::{self, QaRequest};
use crate::modules::knowledge_base::views::{
    ChunkResponse, SearchRequest, SearchResultResponse,
};
use crate::utils::error::IntoModelResult;
use crate::views::errors::parse_uuid;

#[utoipa::path(
    post,
    path = "/api/kb/search",
    tag = "知识库",
    description = "语义搜索",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn search(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<SearchRequest>,
) -> Result<Response> {
    let embedding_client =
        ctx.shared_store
            .get::<SharedEmbeddingClient>()
            .ok_or_else(|| {
                crate::views::errors::err_internal(
                    "knowledge_base.embedding_client_not_initialized",
                    "Embedding client not initialized",
                )
            })?;
    let search_provider =
        ctx.shared_store
            .get::<SharedSearchProvider>()
            .ok_or_else(|| {
                crate::views::errors::err_internal(
                    "knowledge_base.search_provider_not_initialized",
                    "Search provider not initialized",
                )
            })?;

    let settings: AppSettings = ctx
        .config
        .typed_settings()
        .map_err(|e| Error::Message(format!("invalid settings: {e}")))?
        .ok_or_else(|| Error::Message("settings missing".into()))?;
    let kb_config = settings
        .knowledge_base
        .as_ref()
        .ok_or_else(|| Error::Message("knowledge base not configured".into()))?;

    let limit = params
        .limit
        .unwrap_or(kb_config.search.default_limit as usize);

    let results = service::search_service::hybrid_search(
        &embedding_client,
        &search_provider,
        &service::search_service::HybridSearchParams {
            model_name: kb_config.embedding.model.clone(),
            query: params.query,
            tenant_id: tc.tenant_id,
            user_id: tc.user_id,
            limit,
            document_ids: params.document_ids,
        },
    )
    .await
    .map_err(|e| KnowledgeBaseError::ProviderError(e.to_string()).to_err())?;

    let response: Vec<SearchResultResponse> = results
        .iter()
        .map(|r| SearchResultResponse {
            chunk_id: r.chunk_id.to_string(),
            document_id: r.document_id.to_string(),
            content: r.content.clone(),
            heading_path: r.heading_path.clone(),
            score: r.score,
        })
        .collect();

    format::json(response)
}

#[utoipa::path(
    get,
    path = "/api/kb/documents/{id}/chunks",
    tag = "知识库",
    description = "查询文档分块列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn chunks(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let doc_id = parse_uuid(id)?;

    // Verify document exists and belongs to tenant
    service::get_document(&ctx.db, doc_id, tc.tenant_id)
        .await
        .model_err()?;

    let items = kb_chunks::Entity::find()
        .filter(kb_chunks::Column::DocumentId.eq(doc_id))
        .order_by_asc(kb_chunks::Column::ChunkIndex)
        .all(&ctx.db)
        .await
        .model_err()?;

    let response: Vec<ChunkResponse> = items
        .iter()
        .map(|c| ChunkResponse {
            id: c.id.to_string(),
            document_id: c.document_id.to_string(),
            chunk_index: c.chunk_index,
            content: c.content.clone(),
            heading_path: c.heading_path.clone(),
            token_count: c.token_count,
            char_start: c.char_start,
            char_end: c.char_end,
        })
        .collect();

    format::json(response)
}

struct QaV3StreamDeps {
    embedding_client: SharedEmbeddingClient,
    search_provider: SharedSearchProvider,
    qa_client: SharedQaClient,
    memory_store: SharedMemoryStore,
    session_locks: SessionLockMap,
    compaction_guard: crate::initializers::knowledge_base::CompactionGuard,
    broker: Arc<dyn ToolResultBroker>,
    kb_config: crate::config::KnowledgeBaseConfig,
}

fn resolve_qa_v3_deps(ctx: &AppContext) -> Result<QaV3StreamDeps> {
    let embedding_client =
        ctx.shared_store
            .get::<SharedEmbeddingClient>()
            .ok_or_else(|| {
                crate::views::errors::err_internal(
                    "knowledge_base.embedding_client_not_initialized",
                    "Embedding client not initialized",
                )
            })?;
    let search_provider =
        ctx.shared_store
            .get::<SharedSearchProvider>()
            .ok_or_else(|| {
                crate::views::errors::err_internal(
                    "knowledge_base.search_provider_not_initialized",
                    "Search provider not initialized",
                )
            })?;
    let qa_client = ctx.shared_store.get::<SharedQaClient>().ok_or_else(|| {
        crate::views::errors::err_internal(
            "knowledge_base.qa_client_not_initialized",
            "QA client not initialized",
        )
    })?;
    let memory_store = ctx.shared_store.get::<SharedMemoryStore>().ok_or_else(|| {
        crate::views::errors::err_internal(
            "knowledge_base.memory_store_not_initialized",
            "Memory store not initialized — is knowledge_base enabled?",
        )
    })?;
    let session_locks = ctx.shared_store.get::<SessionLockMap>().ok_or_else(|| {
        crate::views::errors::err_internal(
            "knowledge_base.session_locks_not_initialized",
            "Session locks not initialized",
        )
    })?;
    let compaction_guard = ctx
        .shared_store
        .get::<crate::initializers::knowledge_base::CompactionGuard>()
        .ok_or_else(|| {
            crate::views::errors::err_internal(
                "knowledge_base.compaction_guard_not_initialized",
                "Compaction guard not initialized",
            )
        })?;
    let broker = ctx
        .shared_store
        .get::<Arc<dyn ToolResultBroker>>()
        .ok_or_else(|| {
            crate::views::errors::err_internal(
                "knowledge_base.tool_result_broker_not_initialized",
                "Tool result broker not initialized",
            )
        })?;

    let settings: AppSettings = ctx
        .config
        .typed_settings()
        .map_err(|e| Error::Message(format!("invalid settings: {e}")))?
        .ok_or_else(|| Error::Message("settings missing".into()))?;
    let kb_config = settings
        .knowledge_base
        .ok_or_else(|| Error::Message("knowledge base not configured".into()))?;

    Ok(QaV3StreamDeps {
        embedding_client,
        search_provider,
        qa_client,
        memory_store,
        session_locks,
        compaction_guard,
        broker,
        kb_config,
    })
}

#[utoipa::path(
    post,
    path = "/api/kb/qa/v3/stream",
    tag = "知识库",
    description = "智能问答v3（流式）",
    responses((status = 200, description = "SSE stream"))
)]
#[debug_handler]
#[allow(clippy::significant_drop_tightening, clippy::redundant_clone)]
pub(crate) async fn qa_v3_stream(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<QaRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    // M10: Validate instruction is not empty
    if params.instruction.trim().is_empty() {
        return Err(crate::views::errors::err_bad_request(
            "knowledge_base.instruction_empty",
            "instruction cannot be empty",
        ));
    }

    let deps = resolve_qa_v3_deps(&ctx)?;

    let (tx, rx) = mpsc::channel::<String>(64);

    // Clone/move all values for 'static spawn
    let db = ctx.db.clone();
    let qa_config = deps.kb_config.qa.clone();
    let embedding_model = deps.kb_config.embedding.model.clone();
    let tenant_id = tc.tenant_id;
    let user_id = tc.user_id;

    // Propagate the parent tracing span into the spawned task so pipeline
    // sub-spans appear as children of the http.request span.
    let parent_span = tracing::Span::current();

    tokio::spawn(async move {
        let _guard = parent_span.enter();
        let result =
            crate::modules::knowledge_base::service::qa_v3_service::process_qa_v3_stream(
                &db,
                &deps.embedding_client,
                &deps.qa_client,
                &crate::modules::knowledge_base::service::qa_v3_service::QaStreamParams {
                    search_provider: deps.search_provider,
                    memory_store: deps.memory_store,
                    request: params,
                    tenant_id,
                    user_id,
                    config: qa_config,
                    embedding_model_name: embedding_model,
                    tx,
                    session_locks: deps.session_locks,
                    compaction_guard: deps.compaction_guard,
                    broker: deps.broker,
                },
            )
            .await;

        if result == Err(()) {
            tracing::error!("process_qa_v3_stream failed");
        }
        // tx is dropped here, closing the channel
    });

    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|msg| (Ok::<_, Infallible>(Event::default().data(msg)), rx))
    });

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}

// ---------------------------------------------------------------------------
// Tool result callback (frontend → backend)
// ---------------------------------------------------------------------------

/// Request body for the frontend to POST tool execution results.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolResultRequest {
    tool_call_id: String,
    status: String, // "success" | "error"
    output: Option<serde_json::Value>,
    error: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/kb/qa/v3/tool-result",
    tag = "知识库",
    description = "回传前端 tool 执行结果",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn receive_tool_result(
    State(ctx): State<AppContext>,
    Json(params): Json<ToolResultRequest>,
) -> Result<Response> {
    let broker = ctx
        .shared_store
        .get::<Arc<dyn ToolResultBroker>>()
        .ok_or_else(|| {
            crate::views::errors::err_internal(
                "knowledge_base.tool_result_broker_not_initialized",
                "Tool result broker not initialized",
            )
        })?;

    let output_str = match params.output {
        Some(serde_json::Value::String(s)) => s,
        Some(v) => v.to_string(),
        None => params.error.unwrap_or_default(),
    };

    let result = ToolResult {
        output: output_str,
        is_error: params.status == "error",
    };

    tracing::info!(
        tool_call_id = %params.tool_call_id,
        status = %params.status,
        is_error = result.is_error,
        output_len = result.output.len(),
        "receive_tool_result: resolving"
    );

    match broker.resolve(&params.tool_call_id, result).await {
        ResolveOutcome::Fresh => {
            tracing::info!(tool_call_id = %params.tool_call_id, "receive_tool_result: resolved OK");
            format::json(serde_json::json!({ "ok": true }))
        }
        ResolveOutcome::AlreadyResolved => {
            tracing::info!(tool_call_id = %params.tool_call_id, "receive_tool_result: AlreadyResolved (idempotent 200)");
            format::json(serde_json::json!({ "ok": true, "alreadyResolved": true }))
        }
        ResolveOutcome::NotFound => {
            tracing::warn!(tool_call_id = %params.tool_call_id, "receive_tool_result: NotFound");
            Err(crate::views::errors::err_not_found(
                "knowledge_base.tool_call_id_not_found",
                "tool_call_id not found",
            ))
        }
    }
}
