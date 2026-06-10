//! v3 streaming QA orchestration service.
//!
//! Uses rig-core tools + `PromptHook` + multi-turn streaming for a tool-augmented
//! QA pipeline that can read materials, search documents, and answer questions
//! in a multi-turn conversation.

use std::sync::Arc;

use futures_util::StreamExt;
use rig::agent::{MultiTurnStreamItem, StreamingResult};
use rig::client::CompletionClient;
use rig::streaming::{StreamedAssistantContent, StreamingChat};
use tokio::sync::mpsc;
use tracing::Instrument;
use uuid::Uuid;

use crate::config::QaConfig;
use crate::initializers::knowledge_base::{
    CompactionGuard, SessionLockMap, SharedMemoryStore, SharedSearchProvider,
};
use crate::models::_entities::{chat_messages, chat_sessions};
use crate::modules::knowledge_base::providers::{SharedEmbeddingClient, SharedQaClient};

use super::chat_service;
use super::chat_service::CreateMessageParams;
use super::library_service;
use super::memory_service;
use super::memory_service::IndexMessageParams;
use super::memory_service::RecallHistoryParams;
use super::qa_compaction_service::CompactHistoryParams;
use super::qa_stream_types::{QaEvent, QaPhase, QaStreamResponse};
use super::qa_types::{Citation, QaRequest, TokenUsage};
use super::tools::qa_v3_hook::QaV3Hook;
use super::tools::{
    ListConversationHistoryTool, ListKnowledgeBaseDocumentsTool,
    ListKnowledgeBaseScopeTool, ListMaterialsTool, MaterialRegistry,
    ReadConversationTurnTool, ReadKnowledgeBaseLinesTool, ReadMaterialTool,
    SearchKnowledgeBaseTool, SearchMaterialTool, ToolResultBroker,
};

mod materials;
pub(crate) mod prompt;

use materials::prepare_materials;
pub(crate) use prompt::estimate_message_tokens;
use prompt::{
    build_chat_history_with_budget, build_page_tool_stubs, build_system_prompt,
    build_user_prompt, estimate_text_tokens,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

type EventSender = mpsc::Sender<String>;

/// Serialize a [`QaEvent`] to JSON and send it through the channel.
///
/// Returns `Err(())` if the receiver has been dropped (frontend disconnected).
async fn send_event(tx: &EventSender, event: QaEvent) -> Result<(), ()> {
    let json = serde_json::to_string(&event).map_err(|e| {
        tracing::error!(error = %e, "Failed to serialize QaEvent");
    })?;
    tx.send(json).await.map_err(|_| {
        tracing::debug!("Event channel closed — frontend disconnected");
    })
}

/// Synchronous variant for use in `map_err` closures.
fn send_event_blocking(tx: &EventSender, event: &QaEvent) -> Result<(), ()> {
    let json = serde_json::to_string(&event).map_err(|e| {
        tracing::error!(error = %e, "Failed to serialize QaEvent");
    })?;
    tx.blocking_send(json).map_err(|_| {
        tracing::debug!("Event channel closed — frontend disconnected");
    })
}

/// Parameters for [`spawn_index_message`].
#[derive(Debug)]
struct SpawnIndexParams {
    embedding_model_name: String,
    session_id: Uuid,
    tenant_id: Uuid,
    msg_id: Uuid,
    role: String,
    content: String,
    has_material: bool,
    turn_index: i32,
}

/// Async fire-and-forget message indexing to Qdrant `chat_memory`.
/// Call after both user and assistant message persistence.
fn spawn_index_message(
    memory_store: &SharedMemoryStore,
    embedding_client: &SharedEmbeddingClient,
    params: &SpawnIndexParams,
) {
    let ms = memory_store.clone();
    let ec = embedding_client.clone();
    let collection_name = memory_store.collection_name.clone();
    let embedding_model_name = params.embedding_model_name.clone();
    let session_id = params.session_id;
    let tenant_id = params.tenant_id;
    let msg_id = params.msg_id;
    let role = params.role.clone();
    let content = params.content.clone();
    let has_material = params.has_material;
    let turn_index = params.turn_index;
    let span = tracing::info_span!(
        "qa.message_index",
        session_id = %session_id,
        tenant_id = %tenant_id,
        message_id = %msg_id,
        role = %role,
        has_material,
        turn_index,
    );

    tokio::spawn(
        async move {
            if let Err(e) = memory_service::index_message(
                &ec.0,
                &ms.client,
                &IndexMessageParams {
                    collection_name,
                    model_name: embedding_model_name,
                    session_id,
                    tenant_id,
                    message_id: msg_id,
                    role,
                    content,
                    has_material,
                    turn_index,
                },
            )
            .await
            {
                tracing::error!(error = %e, "Failed to index message to chat_memory");
            }
        }
        .instrument(span),
    );
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Parameters for [`process_qa_v3_stream`].
pub struct QaStreamParams {
    pub search_provider: SharedSearchProvider,
    pub memory_store: SharedMemoryStore,
    pub request: QaRequest,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub config: QaConfig,
    pub embedding_model_name: String,
    pub tx: mpsc::Sender<String>,
    pub session_locks: SessionLockMap,
    pub compaction_guard: CompactionGuard,
    pub broker: Arc<dyn ToolResultBroker>,
}

struct SessionPrep {
    session: chat_sessions::Model,
    session_id_str: String,
    history: Vec<chat_messages::Model>,
}

struct MaterialPrep {
    registry: Arc<MaterialRegistry>,
    current_turn_materials: Vec<String>,
}

struct CompactionPrep {
    summary: String,
    needs_compaction: bool,
    recent_start: usize,
    recent_history: Vec<chat_messages::Model>,
    history_tokens: usize,
}

struct QaTurnResult {
    final_answer: String,
    tool_call_count: u32,
    captured_usage: Option<rig::completion::Usage>,
    client_connected: bool,
}

struct QaPersistResult {
    citations: Vec<Citation>,
    prompt_tokens: i32,
    completion_tokens: i32,
    total_tokens: i32,
}

struct TurnDebugData {
    system_prompt: String,
    system_prompt_tokens: usize,
    user_prompt: String,
    chat_history: Vec<rig::completion::message::Message>,
    relevant_context: Option<String>,
}

struct QaStreamCtx<'a> {
    db: &'a sea_orm::DatabaseConnection,
    embedding_client: &'a SharedEmbeddingClient,
    qa_client: &'a SharedQaClient,
    params: &'a QaStreamParams,
    request: &'a QaRequest,
    tenant_id: Uuid,
    user_id: Uuid,
    config: &'a QaConfig,
    embedding_model_name: &'a String,
    search_provider: &'a SharedSearchProvider,
    tx: &'a mpsc::Sender<String>,
    session_locks: &'a SessionLockMap,
    compaction_guard: &'a CompactionGuard,
    broker: &'a Arc<dyn ToolResultBroker>,
}

async fn prepare_session_and_history(ctx: &QaStreamCtx<'_>) -> Result<SessionPrep, ()> {
    let session_span = tracing::Span::current();
    let session = match ctx.request.session_id {
        Some(sid) => chat_service::get_session(ctx.db, sid, ctx.tenant_id, ctx.user_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to get session");
            })
            .ok(),
        None => chat_service::create_session(ctx.db, ctx.tenant_id, ctx.user_id, None)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to create session");
            })
            .ok(),
    };

    let Some(session) = session else {
        let _ = send_event(
            ctx.tx,
            QaEvent::Error {
                message: "Failed to create or retrieve session".to_string(),
            },
        )
        .await;
        return Err(());
    };

    session_span.record("session_id", session.id.to_string());
    let session_id_str = session.id.to_string();
    let session_guard_lock = {
        let mut locks = ctx.session_locks.lock().await;
        if locks.len() > 100 {
            locks.retain(|_, arc| arc.try_lock().is_err());
        }
        let cloned = locks
            .entry(session.id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        drop(locks);
        cloned
    };
    let session_guard_lock = session_guard_lock.lock().await;
    let history = chat_service::get_session_messages(
        ctx.db,
        session.id,
        ctx.tenant_id,
        ctx.user_id,
    )
    .await
    .unwrap_or_default();
    session_span.record("history_len", history.len());
    tracing::info!(
        session_id = %session.id,
        history_len = history.len(),
        "Session ready: id={} history_len={}",
        session.id,
        history.len(),
    );
    drop(session_guard_lock);

    let _ = session_id_str;
    Ok(SessionPrep {
        session,
        session_id_str,
        history,
    })
}

async fn save_user_turn(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    history: &[chat_messages::Model],
    inline_material_id: Option<&str>,
) -> Result<(), ()> {
    let material_refs_json = build_material_refs_json(ctx.request, inline_material_id);
    let has_material = material_refs_json.is_some();
    let user_msg = chat_service::create_message(
        ctx.db,
        &CreateMessageParams {
            session_id: session.id,
            tenant_id: ctx.tenant_id,
            user_id: ctx.user_id,
            role: "user".to_string(),
            content: ctx.request.instruction.clone(),
            material_refs: material_refs_json,
            intent: None,
            strategy: None,
            token_usage: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        },
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to save user message");
    })?;

    spawn_index_message(
        &ctx.params.memory_store,
        ctx.embedding_client,
        &SpawnIndexParams {
            embedding_model_name: ctx.embedding_model_name.clone(),
            session_id: session.id,
            tenant_id: ctx.tenant_id,
            msg_id: user_msg.id,
            role: "user".to_string(),
            content: ctx.request.instruction.clone(),
            has_material,
            turn_index: (i32::try_from(history.len()).unwrap_or(i32::MAX) + 1) / 2,
        },
    );
    Ok(())
}

fn build_material_refs_json(
    request: &QaRequest,
    inline_material_id: Option<&str>,
) -> Option<serde_json::Value> {
    if request.material.inline.is_none()
        && request.material.library_id.is_none()
        && request.material.folder_id.is_none()
        && request.material.file_ids.is_empty()
        && request.material.document_ids.is_empty()
    {
        return None;
    }

    let mut refs = serde_json::json!({});
    if let Some(library_id) = request.material.library_id {
        refs["libraryId"] = serde_json::json!(library_id.to_string());
    }
    if let Some(folder_id) = request.material.folder_id {
        refs["folderId"] = serde_json::json!(folder_id.to_string());
        refs["includeSubfolders"] =
            serde_json::json!(request.material.include_subfolders);
    }
    if let Some(ref inline_text) = request.material.inline {
        refs["inline"] = serde_json::json!({
            "type": "inline",
            "name": "粘贴文本",
            "id": inline_material_id,
            "content": inline_text,
        });
    }
    if !request.material.file_ids.is_empty() {
        refs["fileIds"] = serde_json::json!(request
            .material
            .file_ids
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>());
    }
    if !request.material.document_ids.is_empty() {
        refs["documentIds"] = serde_json::json!(request
            .material
            .document_ids
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>());
    }
    Some(refs)
}

async fn prepare_compaction(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    history: &[chat_messages::Model],
) -> CompactionPrep {
    let compaction_span = tracing::Span::current();
    let history_tokens: usize = history.iter().map(estimate_message_tokens).sum();
    let token_threshold = usize::try_from(ctx.config.max_context_tokens)
        .unwrap_or_default()
        .saturating_sub(ctx.config.compaction_reserve_tokens);
    let needs_compaction = history_tokens > token_threshold
        && history.len() > ctx.config.compaction_threshold;
    let recent_start = if needs_compaction {
        history
            .len()
            .saturating_sub(ctx.config.compaction_recent_turns)
    } else {
        0
    };
    let recent_history = if needs_compaction {
        history[recent_start..].to_vec()
    } else {
        history.to_vec()
    };

    tracing::info!(
        history_total = history.len(),
        history_tokens,
        needs_compaction,
        recent_history_len = recent_history.len(),
        recent_start,
        "Compaction: history={} total_tokens={} triggered={} recent={}",
        history.len(),
        history_tokens,
        needs_compaction,
        recent_history.len(),
    );
    compaction_span.record("triggered", needs_compaction);
    compaction_span.record("history_tokens", history_tokens);

    let summary = load_cached_summary(ctx, session, needs_compaction, recent_start).await;
    maybe_spawn_compaction(ctx, session, history, needs_compaction, recent_start);

    CompactionPrep {
        summary,
        needs_compaction,
        recent_start,
        recent_history,
        history_tokens,
    }
}

async fn load_cached_summary(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    needs_compaction: bool,
    recent_start: usize,
) -> String {
    if !needs_compaction || recent_start == 0 {
        return String::new();
    }
    super::qa_compaction_service::get_cached_summary(ctx.db, session.id, ctx.tenant_id)
        .await
        .ok()
        .flatten()
        .map(|c| c.summary)
        .unwrap_or_default()
}

fn maybe_spawn_compaction(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    history: &[chat_messages::Model],
    needs_compaction: bool,
    recent_start: usize,
) {
    if !needs_compaction || recent_start == 0 {
        return;
    }

    let mut should_spawn = ctx.compaction_guard.entry(session.id).or_insert(false);
    if *should_spawn.value() {
        return;
    }
    *should_spawn.value_mut() = true;
    drop(should_spawn);

    let bg_db = ctx.db.clone();
    let bg_qa_client = ctx.qa_client.clone();
    let bg_session_id = session.id;
    let bg_tenant_id = ctx.tenant_id;
    let bg_history = history.to_vec();
    let bg_model = ctx.config.model.clone();
    let bg_threshold = ctx.config.compaction_threshold;
    let bg_recent_turns = ctx.config.compaction_recent_turns;
    let bg_max_ctx = ctx.config.max_context_tokens;
    let bg_reserve = ctx.config.compaction_reserve_tokens;
    let bg_guard = ctx.compaction_guard.clone();
    let span = tracing::info_span!(
        "qa.compaction",
        session_id = %bg_session_id,
        tenant_id = %bg_tenant_id,
        history_len = bg_history.len(),
    );

    tokio::spawn(
        async move {
            tracing::info!(
                session_id = %bg_session_id,
                history_len = bg_history.len(),
                "Spawning background compaction"
            );
            let bg_params = CompactHistoryParams {
                session_id: bg_session_id,
                tenant_id: bg_tenant_id,
                max_context_tokens: bg_max_ctx,
                recent_turns: bg_recent_turns,
                compaction_reserve_tokens: bg_reserve,
            };
            let result = super::qa_compaction_service::compact_history(
                &bg_db,
                &bg_qa_client.0,
                &bg_history,
                &bg_model,
                bg_threshold,
                &bg_params,
            )
            .await;
            match result {
                Ok(summary) => tracing::info!(
                    session_id = %bg_session_id,
                    summary_len = summary.len(),
                    "Background compaction completed"
                ),
                Err(e) => tracing::error!(
                    session_id = %bg_session_id,
                    error = %e,
                    "Background compaction failed"
                ),
            }
            bg_guard.remove(&bg_session_id);
        }
        .instrument(span),
    );
}

async fn recall_relevant_context(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    history: &[chat_messages::Model],
    recent_history: &[chat_messages::Model],
) -> Option<String> {
    let recall_span = tracing::Span::current();
    let relevant_context =
        recall_relevant_context_inner(ctx, session, history, recent_history).await;
    recall_span.record("has_recall", relevant_context.is_some());
    relevant_context
}

async fn recall_relevant_context_inner(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    history: &[chat_messages::Model],
    recent_history: &[chat_messages::Model],
) -> Option<String> {
    if ctx.config.history_strategy == "none" {
        return None;
    }
    let strategy = match ctx.config.history_strategy.as_str() {
        "retrieve" => memory_service::HistoryStrategy::RetrieveRelevant {
            top_k: ctx.config.semantic_top_k,
        },
        "original" => memory_service::HistoryStrategy::ReadOriginalMaterial,
        "hybrid" => memory_service::HistoryStrategy::Hybrid {
            recent_turns: ctx.config.compaction_recent_turns,
            top_k: ctx.config.semantic_top_k,
        },
        other => {
            tracing::warn!(
                strategy = other,
                "Unknown history strategy, skipping recall"
            );
            return None;
        }
    };

    let mut recalled = memory_service::recall_history(
        &ctx.embedding_client.0,
        &ctx.params.memory_store.client,
        &ctx.params.memory_store.collection_name,
        ctx.embedding_model_name,
        &strategy,
        &RecallHistoryParams {
            session_id: session.id,
            tenant_id: ctx.tenant_id,
            query: &ctx.request.instruction,
            history_db: history,
        },
    )
    .await
    .unwrap_or_default();
    let recent_msg_ids: std::collections::HashSet<Uuid> =
        recent_history.iter().map(|m| m.id).collect();
    recalled
        .relevant_messages
        .retain(|m| !recent_msg_ids.contains(&m.message_id));
    let ctx_text = memory_service::format_recalled_context(&recalled);
    (!ctx_text.is_empty()).then_some(ctx_text)
}

async fn stream_qa_turn(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    history: &[chat_messages::Model],
    materials: &MaterialPrep,
    compaction: &CompactionPrep,
) -> Result<(QaTurnResult, QaV3Hook), ()> {
    let records_hook = QaV3Hook::new(ctx.tx.clone());
    let turn_debug =
        prepare_turn_debug_data(ctx, session, history, materials, compaction).await;
    set_turn_debug_context(&records_hook, ctx, history, compaction, &turn_debug);

    let agent_span = tracing::info_span!(
        "qa.agent",
        model = %ctx.config.model,
        provider = %ctx.config.provider,
        system_prompt_tokens = turn_debug.system_prompt_tokens,
    );
    async move {
        send_event(
            ctx.tx,
            QaEvent::PhaseChanged {
                phase: QaPhase::GeneratingAnswer,
            },
        )
        .await?;
        tracing::info!(
            system_prompt_len = turn_debug.system_prompt.len(),
            system_prompt_tokens = turn_debug.system_prompt_tokens,
            chat_history_messages = turn_debug.chat_history.len(),
            user_prompt_len = turn_debug.user_prompt.len(),
            model = %ctx.config.model,
            provider = %ctx.config.provider,
            max_context_tokens = ctx.config.max_context_tokens,
            "Agent: model={} provider={} prompt_tokens={} history_msgs={} user_len={}",
            ctx.config.model,
            ctx.config.provider,
            turn_debug.system_prompt_tokens,
            turn_debug.chat_history.len(),
            turn_debug.user_prompt.len(),
        );
        let result =
            run_agent_stream(ctx, session, materials, &records_hook, turn_debug).await;
        let current_span = tracing::Span::current();
        current_span.record("answer_len", result.final_answer.len());
        current_span.record("tool_call_count", result.tool_call_count);
        Ok((result, records_hook))
    }
    .instrument(agent_span)
    .await
}

async fn prepare_turn_debug_data(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    history: &[chat_messages::Model],
    materials: &MaterialPrep,
    compaction: &CompactionPrep,
) -> TurnDebugData {
    let relevant_context =
        recall_relevant_context(ctx, session, history, &compaction.recent_history)
            .instrument(tracing::info_span!(
                "qa.recall",
                strategy = %ctx.config.history_strategy
            ))
            .await;
    let system_prompt = build_system_prompt(
        ctx.request,
        &materials.registry,
        &compaction.summary,
        relevant_context.as_deref(),
    );
    let system_prompt_tokens = estimate_text_tokens(&system_prompt, false);
    let user_prompt = build_user_prompt(ctx.request, &materials.current_turn_materials);
    let chat_history = build_chat_history_with_budget(
        &compaction.recent_history,
        ctx.config.max_context_tokens,
        ctx.config.response_reserve_tokens,
        system_prompt_tokens,
    );
    TurnDebugData {
        system_prompt,
        system_prompt_tokens,
        user_prompt,
        chat_history,
        relevant_context,
    }
}

async fn run_agent_stream(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    materials: &MaterialPrep,
    records_hook: &QaV3Hook,
    debug: TurnDebugData,
) -> QaTurnResult {
    let mut agent_builder = ctx
        .qa_client
        .0
        .agent(&ctx.config.model)
        .preamble(&debug.system_prompt)
        .hook(records_hook.clone())
        .tool(ListMaterialsTool {
            registry: materials.registry.clone(),
        })
        .tool(ReadMaterialTool {
            registry: materials.registry.clone(),
        })
        .tool(SearchMaterialTool {
            registry: materials.registry.clone(),
        });

    let conversation_db = std::sync::Arc::new(ctx.db.clone());
    if ctx.request.material.use_knowledge_base {
        let folder_ids = match resolve_search_folder_ids(ctx).await {
            Ok(ids) => ids,
            Err(message) => {
                let _ = send_event(ctx.tx, QaEvent::Error { message }).await;
                return disconnected_turn_result();
            }
        };
        let document_ids = (!ctx.request.material.document_ids.is_empty())
            .then(|| ctx.request.material.document_ids.clone());
        agent_builder = agent_builder
            .tool(SearchKnowledgeBaseTool {
                db: conversation_db.clone(),
                embedding_client: ctx.embedding_client.clone(),
                search_provider: ctx.search_provider.clone(),
                embedding_model_name: ctx.embedding_model_name.clone(),
                tenant_id: ctx.tenant_id,
                user_id: ctx.user_id,
                library_id: ctx.request.material.library_id,
                folder_id: ctx.request.material.folder_id,
                folder_ids: folder_ids.clone(),
                document_ids: document_ids.clone(),
            })
            .tool(ListKnowledgeBaseDocumentsTool {
                db: conversation_db.clone(),
                tenant_id: ctx.tenant_id,
                user_id: ctx.user_id,
                library_id: ctx.request.material.library_id,
                folder_id: ctx.request.material.folder_id,
                folder_ids: folder_ids.clone(),
                document_ids: document_ids.clone(),
            })
            .tool(ListKnowledgeBaseScopeTool {
                db: conversation_db.clone(),
                tenant_id: ctx.tenant_id,
                user_id: ctx.user_id,
                library_id: ctx.request.material.library_id,
                folder_id: ctx.request.material.folder_id,
                folder_ids: folder_ids.clone(),
            })
            .tool(ReadKnowledgeBaseLinesTool {
                db: conversation_db.clone(),
                tenant_id: ctx.tenant_id,
                user_id: ctx.user_id,
                library_id: ctx.request.material.library_id,
                folder_id: ctx.request.material.folder_id,
                folder_ids,
                document_ids,
            });
    }

    agent_builder = agent_builder
        .tool(ListConversationHistoryTool {
            db: conversation_db.clone(),
            session_id: session.id,
            tenant_id: ctx.tenant_id,
            user_id: ctx.user_id,
        })
        .tool(ReadConversationTurnTool {
            db: conversation_db,
            session_id: session.id,
            tenant_id: ctx.tenant_id,
            user_id: ctx.user_id,
        });

    for stub in build_page_tool_stubs(ctx) {
        agent_builder = agent_builder.tool(stub);
    }

    let mut agent_builder = agent_builder.default_max_turns(15);
    if ctx.config.provider == "ollama" {
        agent_builder = agent_builder.additional_params(serde_json::json!({
            "options": { "num_ctx": ctx.config.max_context_tokens }
        }));
    }

    let mut stream = agent_builder
        .build()
        .stream_chat(&debug.user_prompt, debug.chat_history)
        .multi_turn(15)
        .await;
    consume_agent_stream(ctx, session, records_hook, &mut stream).await
}

const fn disconnected_turn_result() -> QaTurnResult {
    QaTurnResult {
        final_answer: String::new(),
        tool_call_count: 0,
        captured_usage: None,
        client_connected: false,
    }
}

async fn resolve_search_folder_ids(
    ctx: &QaStreamCtx<'_>,
) -> Result<Option<Vec<Uuid>>, String> {
    if !ctx.request.material.include_subfolders {
        return Ok(None);
    }

    let Some(folder_id) = ctx.request.material.folder_id else {
        return Ok(None);
    };

    library_service::list_folder_subtree_ids(ctx.db, ctx.tenant_id, folder_id)
        .await
        .map(Some)
        .map_err(|e| {
            tracing::warn!(
                error = %e,
                folder_id = %folder_id,
                "failed to resolve knowledge base folder subtree"
            );
            e.to_string()
        })
}

async fn consume_agent_stream(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    records_hook: &QaV3Hook,
    stream: &mut StreamingResult<rig::providers::deepseek::StreamingCompletionResponse>,
) -> QaTurnResult {
    let mut final_answer = String::new();
    let mut tool_call_count = 0;
    let mut captured_usage = None;
    let mut client_connected = true;

    while let Some(item) = stream.next().await {
        match item {
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::Text(text),
            )) => {
                final_answer.push_str(&text.text);
                if send_event(ctx.tx, QaEvent::AnswerToken { token: text.text })
                    .await
                    .is_err()
                {
                    client_connected = false;
                    break;
                }
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::ToolCall { .. },
            )) => {
                tool_call_count += 1;
            }
            Ok(MultiTurnStreamItem::FinalResponse(fin)) => {
                captured_usage = Some(fin.usage());
                tracing::info!(
                    answer_len = final_answer.len(),
                    tool_calls = tool_call_count,
                    "Agent done: answer_len={} tool_calls={}",
                    final_answer.len(),
                    tool_call_count,
                );
            }
            Err(e) => {
                let error_message = e.to_string();
                persist_stream_error(
                    ctx,
                    session,
                    records_hook,
                    &final_answer,
                    error_message.clone(),
                )
                .await;
                let _ = send_event(
                    ctx.tx,
                    QaEvent::Error {
                        message: error_message,
                    },
                )
                .await;
                client_connected = false;
                break;
            }
            _ => {}
        }
    }

    QaTurnResult {
        final_answer,
        tool_call_count,
        captured_usage,
        client_connected,
    }
}

fn set_turn_debug_context(
    records_hook: &QaV3Hook,
    ctx: &QaStreamCtx<'_>,
    history: &[chat_messages::Model],
    compaction: &CompactionPrep,
    debug: &TurnDebugData,
) {
    use rig::completion::message::{AssistantContent, Message};
    let debug_context = serde_json::json!({
        "system_prompt": &debug.system_prompt,
        "system_prompt_tokens": debug.system_prompt_tokens,
        "user_prompt": &debug.user_prompt,
        "chat_history_summary": {
            "message_count": debug.chat_history.len(),
            "messages": debug.chat_history.iter().map(|msg| match msg {
                Message::User { content } => serde_json::json!({
                    "role": "user",
                    "content_count": content.len()
                }),
                Message::Assistant { content, .. } => {
                    let text_count = content.iter().filter(|c| matches!(c, AssistantContent::Text(_))).count();
                    let has_tool_calls = content.iter().any(|c| matches!(c, AssistantContent::ToolCall(_)));
                    serde_json::json!({
                        "role": "assistant",
                        "has_tool_calls": has_tool_calls,
                        "text_items": text_count
                    })
                }
                Message::System { .. } => serde_json::json!({ "role": "other" }),
            }).collect::<Vec<_>>()
        },
        "config_snapshot": {
            "model": ctx.config.model,
            "provider": ctx.config.provider,
            "max_context_tokens": ctx.config.max_context_tokens,
            "temperature": ctx.config.temperature,
            "history_strategy": ctx.config.history_strategy,
            "compaction_threshold": ctx.config.compaction_threshold,
            "compaction_recent_turns": ctx.config.compaction_recent_turns,
        },
        "compaction": {
            "triggered": compaction.needs_compaction,
            "summary_length": compaction.summary.len(),
            "recent_start": compaction.recent_start,
            "history_total": history.len(),
            "history_tokens": compaction.history_tokens,
        },
        "semantic_recall": {
            "strategy": ctx.config.history_strategy,
            "context_length": debug.relevant_context.as_ref().map_or(0, std::string::String::len),
            "has_recall": debug.relevant_context.is_some(),
        }
    });
    records_hook.set_debug_context(debug_context);
}

async fn persist_stream_error(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    records_hook: &QaV3Hook,
    final_answer: &str,
    error: String,
) {
    let tool_usage_json = collect_tool_usage_json(records_hook);
    let error_content = if final_answer.is_empty() {
        format!("⚠️ 回答生成失败：{error}")
    } else {
        format!("{final_answer}\n\n⚠️ 后续生成失败：{error}")
    };
    let _ = chat_service::create_message(
        ctx.db,
        &CreateMessageParams {
            session_id: session.id,
            tenant_id: ctx.tenant_id,
            user_id: ctx.user_id,
            role: "assistant".to_string(),
            content: error_content,
            material_refs: None,
            intent: None,
            strategy: None,
            token_usage: tool_usage_json,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        },
    )
    .await;
}

fn collect_tool_usage_json(records_hook: &QaV3Hook) -> Option<serde_json::Value> {
    let tool_records = records_hook.take_tool_records();
    let content_parts = records_hook.take_content_parts();
    let mut debug_context = records_hook.take_debug_context();
    let tool_rounds = records_hook.take_tool_rounds();

    if !tool_rounds.is_empty() {
        if let Some(ctx) = debug_context.as_mut() {
            if let Some(obj) = ctx.as_object_mut() {
                obj.insert("toolRounds".to_string(), serde_json::json!(tool_rounds));
            }
        }
    }

    if tool_records.is_empty() && content_parts.is_empty() && debug_context.is_none() {
        return None;
    }

    let mut obj = serde_json::Map::new();
    if !tool_records.is_empty() {
        obj.insert("toolCalls".to_string(), serde_json::json!(tool_records));
    }
    if !content_parts.is_empty() {
        obj.insert("contentParts".to_string(), serde_json::json!(content_parts));
    }
    if let Some(ctx) = debug_context {
        obj.insert("debugContext".to_string(), ctx);
    }
    Some(serde_json::Value::Object(obj))
}

async fn persist_successful_turn(
    ctx: &QaStreamCtx<'_>,
    session_prep: &SessionPrep,
    records_hook: &QaV3Hook,
    turn_result: &QaTurnResult,
) -> Result<QaPersistResult, ()> {
    let persist_span = tracing::info_span!("qa.persist");

    async move {
        if turn_result.client_connected {
            send_event(
                ctx.tx,
                QaEvent::PhaseChanged {
                    phase: QaPhase::Persisting,
                },
            )
            .await?;
        }

        let tool_usage_json = collect_tool_usage_json(records_hook);
        let citations = records_hook.take_citations();
        let (prompt_tokens, completion_tokens, total_tokens) =
            turn_result.captured_usage.map_or((0, 0, 0), |u| {
                (
                    i32::try_from(u.input_tokens).unwrap_or(i32::MAX),
                    i32::try_from(u.output_tokens).unwrap_or(i32::MAX),
                    i32::try_from(u.total_tokens).unwrap_or(i32::MAX),
                )
            });

        let assistant_msg = chat_service::create_message(
            ctx.db,
            &CreateMessageParams {
                session_id: session_prep.session.id,
                tenant_id: ctx.tenant_id,
                user_id: ctx.user_id,
                role: "assistant".to_string(),
                content: turn_result.final_answer.clone(),
                material_refs: None,
                intent: None,
                strategy: None,
                token_usage: tool_usage_json,
                prompt_tokens,
                completion_tokens,
                total_tokens,
            },
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to save assistant message");
        })?;

        spawn_index_message(
            &ctx.params.memory_store,
            ctx.embedding_client,
            &SpawnIndexParams {
                embedding_model_name: ctx.embedding_model_name.clone(),
                session_id: session_prep.session.id,
                tenant_id: ctx.tenant_id,
                msg_id: assistant_msg.id,
                role: "assistant".to_string(),
                content: turn_result.final_answer.clone(),
                has_material: false,
                turn_index: i32::try_from(session_prep.history.len().saturating_add(1))
                    .unwrap_or(i32::MAX)
                    .saturating_add(1)
                    / 2,
            },
        );

        if session_prep.session.title.is_none() {
            let title: String = ctx.request.instruction.chars().take(50).collect();
            let _ = chat_service::update_session_title(
                ctx.db,
                session_prep.session.id,
                ctx.tenant_id,
                ctx.user_id,
                &title,
            )
            .await;
        }

        let current_span = tracing::Span::current();
        current_span.record("answer_len", turn_result.final_answer.len());
        current_span.record("tool_call_count", turn_result.tool_call_count);

        Ok(QaPersistResult {
            citations,
            prompt_tokens,
            completion_tokens,
            total_tokens,
        })
    }
    .instrument(persist_span)
    .await
}

async fn send_completed_event(
    ctx: &QaStreamCtx<'_>,
    session_id_str: String,
    final_answer: String,
    persist: QaPersistResult,
) -> Result<(), ()> {
    send_event(
        ctx.tx,
        QaEvent::Completed {
            response: QaStreamResponse {
                answer: final_answer,
                session_id: session_id_str,
                citations: persist.citations,
                intent: "v3_agent".to_string(),
                output_format: "free_text".to_string(),
                strategy: "agent_tool_calling".to_string(),
                mode: "v3".to_string(),
                usage: TokenUsage {
                    prompt_tokens: persist.prompt_tokens,
                    completion_tokens: persist.completion_tokens,
                    total_tokens: persist.total_tokens,
                },
            },
        },
    )
    .await
}

/// v3 streaming QA pipeline: tool-augmented multi-turn agent.
///
/// 6-step pipeline:
/// 0. Session management
/// 1. Material registration
/// 2. Tool construction
/// 3. Agent building (with `PromptHook`)
/// 4. Streaming loop
/// 5. Persistence
/// 6. Complete
#[tracing::instrument(
    skip(db, embedding_client, qa_client, params),
    fields(tenant_id = %params.tenant_id, user_id = %params.user_id)
)]
pub async fn process_qa_v3_stream(
    db: &sea_orm::DatabaseConnection,
    embedding_client: &SharedEmbeddingClient,
    qa_client: &SharedQaClient,
    params: &QaStreamParams,
) -> Result<(), ()> {
    let ctx = QaStreamCtx {
        db,
        embedding_client,
        qa_client,
        params,
        request: &params.request,
        tenant_id: params.tenant_id,
        user_id: params.user_id,
        config: &params.config,
        embedding_model_name: &params.embedding_model_name,
        search_provider: &params.search_provider,
        tx: &params.tx,
        session_locks: &params.session_locks,
        compaction_guard: &params.compaction_guard,
        broker: &params.broker,
    };

    run_qa_pipeline(ctx).await
}

async fn run_qa_pipeline(ctx: QaStreamCtx<'_>) -> Result<(), ()> {
    let session_prep = prepare_session_and_history(&ctx)
        .instrument(tracing::info_span!("qa.session"))
        .await?;
    send_event(
        ctx.tx,
        QaEvent::Started {
            session_id: session_prep.session_id_str.clone(),
        },
    )
    .await?;
    let material_prep =
        prepare_materials(&ctx, &session_prep.session, &session_prep.history)
            .instrument(tracing::info_span!("qa.material"))
            .await?;
    let compaction =
        prepare_compaction(&ctx, &session_prep.session, &session_prep.history)
            .instrument(tracing::info_span!("qa.compaction"))
            .await;
    let (turn_result, records_hook) = stream_qa_turn(
        &ctx,
        &session_prep.session,
        &session_prep.history,
        &material_prep,
        &compaction,
    )
    .await?;
    let persist =
        persist_successful_turn(&ctx, &session_prep, &records_hook, &turn_result).await?;

    if turn_result.client_connected {
        send_completed_event(
            &ctx,
            session_prep.session_id_str,
            turn_result.final_answer,
            persist,
        )
        .await?;
    }

    Ok(())
}
