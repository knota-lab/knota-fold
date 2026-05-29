//! v3 streaming QA orchestration service.
//!
//! Uses rig-core tools + `PromptHook` + multi-turn streaming for a tool-augmented
//! QA pipeline that can read materials, search documents, and answer questions
//! in a multi-turn conversation.

use std::sync::Arc;

use futures_util::StreamExt;
use rig::agent::MultiTurnStreamItem;
use rig::client::CompletionClient;
use rig::streaming::{StreamedAssistantContent, StreamingChat};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::config::QaConfig;
use crate::initializers::knowledge_base::{
    CompactionGuard, SessionLockMap, SharedMemoryStore, SharedSearchProvider,
};
use crate::models::_entities::{chat_messages, kb_documents};
use crate::modules::knowledge_base::providers::{SharedEmbeddingClient, SharedQaClient};

use super::chat_service;
use super::chat_service::CreateMessageParams;
use super::memory_service;
use super::memory_service::IndexMessageParams;
use super::memory_service::RecallHistoryParams;
use super::qa_compaction_service::CompactHistoryParams;
use super::qa_stream_types::{QaEvent, QaPhase, QaStreamResponse};
use super::qa_types::{QaRequest, TokenUsage};
use super::tools::qa_v3_hook::QaV3Hook;
use super::tools::{
    DocumentContent, FrontendToolStub, InlineText, ListConversationHistoryTool,
    ListMaterialsTool, MaterialRegistry, ReadConversationTurnTool, ReadMaterialTool,
    SearchKnowledgeBaseTool, SearchMaterialTool, ToolResultBroker,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const BASE_SYSTEM_PROMPT: &str = "\
你是一个智能知识库问答助手。你可以通过工具访问用户提供的参考材料和知识库文档。

## 工作方式

1. 回答任何与材料/文档相关的问题时，你**必须**遵守以下流程：
   - **先调用 `list_materials`** 确认当前会话中有哪些可用材料
   - 材料在多轮对话中**持续可用**，即使本轮用户没有新提交材料
   - **不要假设材料不可用**——必须先查看再下结论
   - 然后调用 `read_material` 读取材料的指定部分
   - 可以调用 `search_material` 在材料中搜索关键词
   - 可以调用 `search_knowledge_base` 在知识库文档中搜索

2. 选择正确的材料：
   - `list_materials` 按提交顺序排列，第 1 份是最早提交的
   - 根据材料的**预览内容**判断哪份与用户问题相关，只读取相关材料
   - 当用户说\"第一份\"或\"第二份\"材料时，对应列表中的第 1 项或第 2 项
   - 如果用户本轮新提交了材料且提到\"这份材料\"，优先读取本轮新增的那份

3. 回答问题时：
   - 优先参考用户直接提供的材料
   - 可以分多次读取长材料（每次最多 500 行）
   - 引用材料内容时标注来源和行号
   - **如果在所有材料中都找不到相关信息，明确告知用户\"提供的材料中未找到相关内容\"，不要用自身知识回答材料相关问题**

4. 多轮对话时：
   - 你可以看到之前的对话历史
   - 即使本轮用户没有重新提交材料，之前的材料仍然可用
   - 对材料内容不确定时，**必须重新调用 `read_material`** 确认
   - 不要仅凭记忆引用材料细节，应当重新读取以确保准确
   - 如果需要回顾之前的对话内容，可以调用 `list_conversation_history` 浏览概览
   - 然后用 `read_conversation_turn` 读取感兴趣的具体轮次
   - 当用户问及\"之前讨论过什么\"、\"我第一个问题是什么\"等回顾性问题时，主动使用这些工具

 5. 注意事项：
    - 不要臆造材料中没有的内容
    - 如果材料很长，优先读取最相关的部分（可以通过 search_material 定位）
    - 回答要准确、完整、有条理
    - **不要在回答末尾主动追问或列举建议问题**，等待用户主动提问
    - 会话中有多份材料时，在回答开头说明引用的是哪份材料（如\"根据《xxx》...\"）";

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
fn send_event_blocking(tx: &EventSender, event: QaEvent) -> Result<(), ()> {
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

/// Async fire-and-forget message indexing to Qdrant chat_memory.
/// Call after both user and assistant message persistence.
fn spawn_index_message(
    memory_store: &SharedMemoryStore,
    embedding_client: &SharedEmbeddingClient,
    params: &SpawnIndexParams,
) {
    let ms = memory_store.clone();
    let ec = embedding_client.clone();
    let collection_name = ms.collection_name.clone();
    let embedding_model_name = params.embedding_model_name.clone();
    let session_id = params.session_id;
    let tenant_id = params.tenant_id;
    let msg_id = params.msg_id;
    let role = params.role.clone();
    let content = params.content.clone();
    let has_material = params.has_material;
    let turn_index = params.turn_index;

    tokio::spawn(async move {
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
    });
}

/// Build a text block from recent chat history to include in the system prompt.
/// TODO(Phase 4): Retained for reference — replaced by `build_chat_history()` + `stream_chat()`.
#[allow(dead_code)]
fn build_conversation_context(
    history: &[chat_messages::Model],
    max_messages: usize,
    max_chars_per_msg: usize,
) -> String {
    if history.is_empty() {
        return String::new();
    }

    let start = history.len().saturating_sub(max_messages);
    let recent = &history[start..];
    let mut parts = vec!["\n--- 对话历史 ---".to_string()];
    for msg in recent {
        let role_label = match msg.role.as_str() {
            "user" => "用户",
            "assistant" => "助手",
            _ => continue,
        };
        let truncated: String = msg.content.chars().take(max_chars_per_msg).collect();
        let ellipsis = if msg.content.chars().count() > max_chars_per_msg {
            "…"
        } else {
            ""
        };
        parts.push(format!("{role_label}: {truncated}{ellipsis}"));
    }
    parts.push("--- 对话历史结束 ---\n".to_string());
    parts.join("\n")
}

// ---------------------------------------------------------------------------
// Token estimation & budget-aware history truncation
// ---------------------------------------------------------------------------

/// Estimate token count for text.
/// JSON uses chars/1.2 (structural tokens are expensive);
/// Natural language uses chars/2 (conservative for Chinese-heavy content).
pub(crate) fn estimate_text_tokens(text: &str, is_json: bool) -> usize {
    let char_count = text.chars().count();
    if is_json {
        (char_count as f64 / 1.2).ceil() as usize
    } else {
        (char_count / 2).max(1)
    }
}

const TOOL_OVERHEAD_TOKENS: usize = 10000;

/// Estimate token count for a message, including content + tool call records.
/// JSON content (tool arguments) uses chars/1.2; natural language uses chars/2.
pub(crate) fn estimate_message_tokens(msg: &chat_messages::Model) -> usize {
    let content_tokens = estimate_text_tokens(&msg.content, false);

    let tool_tokens = msg
        .token_usage
        .as_ref()
        .and_then(|u| u.get("toolCalls").and_then(|v| v.as_array()))
        .map_or(0, |arr| {
            arr.iter()
                .map(|tc| {
                    let args_tokens = tc
                        .get("arguments")
                        .map_or(0, |a| estimate_text_tokens(&a.to_string(), true));
                    let preview_tokens = tc
                        .get("resultPreview")
                        .and_then(|v| v.as_str())
                        .map_or(0, |s| estimate_text_tokens(s, false));
                    args_tokens + preview_tokens + 20
                })
                .sum::<usize>()
        });

    content_tokens + tool_tokens
}

/// Build structured chat history from DB messages as rig Messages.
///
/// Converts the flat DB message history into structured `rig::Message` objects
/// that properly represent user messages, assistant messages (with tool calls),
/// and tool results — ready for `stream_chat()`.
fn build_chat_history(
    history: &[chat_messages::Model],
) -> Vec<rig::completion::message::Message> {
    use rig::completion::message::{AssistantContent, Message};
    use rig::OneOrMany;

    let mut messages = Vec::new();

    for msg in history {
        match msg.role.as_str() {
            "user" => {
                messages.push(Message::user(&msg.content));
            }
            "assistant" => {
                if let Some(ref usage) = msg.token_usage {
                    if let Some(tool_calls) =
                        usage.get("toolCalls").and_then(|v| v.as_array())
                    {
                        if !tool_calls.is_empty() {
                            let mut assistant_parts: Vec<AssistantContent> = Vec::new();
                            let mut tool_results: Vec<(String, String, String)> =
                                Vec::new();

                            if !msg.content.is_empty() {
                                assistant_parts
                                    .push(AssistantContent::text(&msg.content));
                            }

                            for (idx, tc) in tool_calls.iter().enumerate() {
                                let tool_name = tc
                                    .get("toolName")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                let result_preview = tc
                                    .get("resultPreview")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let tool_call_id = tc
                                    .get("toolCallId")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    .map_or_else(
                                        || format!("hist-{idx}"),
                                        |s| s.to_string(),
                                    );

                                // Use tool_call_with_call_id to satisfy Ollama's
                                // OpenAI Responses API requirement (call_id is mandatory).
                                assistant_parts.push(
                                    AssistantContent::tool_call_with_call_id(
                                        &tool_call_id,
                                        tool_call_id.clone(),
                                        tool_name,
                                        tc.get("arguments")
                                            .cloned()
                                            .unwrap_or_else(|| serde_json::json!({})),
                                    ),
                                );

                                tool_results.push((
                                    tool_call_id.clone(),
                                    tool_call_id,
                                    format!("[历史工具调用结果] {result_preview}"),
                                ));
                            }

                            if !assistant_parts.is_empty() {
                                messages.push(Message::Assistant {
                                    id: None,
                                    content: OneOrMany::many(assistant_parts)
                                        .unwrap_or_else(|_| {
                                            OneOrMany::one(AssistantContent::text(""))
                                        }),
                                });
                            }

                            for (id, call_id, result_text) in tool_results {
                                messages.push(Message::tool_result_with_call_id(
                                    id,
                                    Some(call_id),
                                    result_text,
                                ));
                            }

                            continue;
                        }
                    }
                }
                messages.push(Message::assistant(&msg.content));
            }
            _ => {}
        }
    }

    messages
}

/// Token-aware history truncation.
///
/// Estimates token count with JSON-aware coefficients,
/// truncates oldest messages to fit within budget.
/// Ensures complete user+assistant pairs (no dangling user at head or tail).
fn build_chat_history_with_budget(
    history: &[chat_messages::Model],
    max_context_tokens: i32,
    response_reserve_tokens: i32,
    system_prompt_tokens: usize,
) -> Vec<rig::completion::message::Message> {
    let budget = (max_context_tokens as usize)
        .saturating_sub(system_prompt_tokens)
        .saturating_sub(response_reserve_tokens as usize)
        .saturating_sub(TOOL_OVERHEAD_TOKENS);

    // Guard against zero budget (system prompt too large, nothing left for history)
    if budget == 0 {
        return build_chat_history(&[]);
    }

    let mut used_tokens = 0usize;
    let mut selected_indices: Vec<usize> = Vec::new();

    for (i, msg) in history.iter().enumerate().rev() {
        let estimated_tokens = estimate_message_tokens(msg);
        if used_tokens + estimated_tokens > budget {
            break;
        }
        used_tokens += estimated_tokens;
        selected_indices.push(i);
    }

    selected_indices.reverse();

    // Head alignment: if first selected is assistant (missing preceding user), skip it
    if let Some(&first) = selected_indices.first() {
        if history[first].role == "assistant" && first + 1 < history.len() {
            selected_indices.remove(0);
        }
    }

    // Tail alignment: remove trailing user messages (incomplete turn from crash)
    while let Some(&last) = selected_indices.last() {
        if history[last].role == "user" {
            selected_indices.pop();
        } else {
            break;
        }
    }

    let selected: Vec<chat_messages::Model> = selected_indices
        .iter()
        .map(|&i| history[i].clone())
        .collect();

    build_chat_history(&selected)
}

/// Register a document from the DB into the registry.
fn register_doc_from_model(registry: &mut MaterialRegistry, doc: &kb_documents::Model) {
    let content = doc.full_text.clone().unwrap_or_default();
    registry.register_document(DocumentContent {
        id: doc.id,
        title: doc.title.clone(),
        content,
        doc_type: doc.source_type.clone(),
        total_lines: doc.full_text.as_deref().map_or(0, |t| t.lines().count()),
    });
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

/// v3 streaming QA pipeline: tool-augmented multi-turn agent.
///
/// 6-step pipeline:
/// 0. Session management
/// 1. Material registration
/// 2. Tool construction
/// 3. Agent building (with PromptHook)
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
    // Destructure params for convenient access
    let request = &params.request;
    let tenant_id = params.tenant_id;
    let user_id = params.user_id;
    let config = &params.config;
    let embedding_model_name = &params.embedding_model_name;
    let tx = &params.tx;
    let search_provider = &params.search_provider;
    let memory_store = &params.memory_store;
    let session_locks = &params.session_locks;
    let compaction_guard = &params.compaction_guard;
    let broker = &params.broker;
    // ── Step 0: Session management ──────────────────────────────────
    let session_span = tracing::info_span!("qa.session");
    let session_guard = session_span.enter();
    let session = match request.session_id {
        Some(sid) => chat_service::get_session(db, sid, tenant_id, user_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to get session");
            })
            .ok(),
        None => chat_service::create_session(db, tenant_id, user_id, None)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to create session");
            })
            .ok(),
    };

    let Some(session) = session else {
        let _ = send_event(
            tx,
            QaEvent::Error {
                message: "Failed to create or retrieve session".to_string(),
            },
        )
        .await;
        return Err(());
    };

    session_span.record("session_id", session.id.to_string());

    let session_id_str = session.id.to_string();

    // Acquire per-session lock to serialise concurrent requests for the same session.
    let session_guard_lock = {
        let mut locks = session_locks.lock().await;

        // Periodic cleanup: remove entries when the map grows beyond threshold.
        // Only removes entries with no active holders (try_lock succeeds).
        if locks.len() > 100 {
            locks.retain(|_, arc| arc.try_lock().is_err());
        }

        let lock = locks
            .entry(session.id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())));
        lock.clone()
    };
    let session_guard_lock = session_guard_lock.lock().await;

    // Load session history
    let history = chat_service::get_session_messages(db, session.id, tenant_id, user_id)
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

    drop(session_guard);
    drop(session_guard_lock);

    // ── Step 1: Material registration ───────────────────────────────
    let material_span = tracing::info_span!("qa.material");
    let material_guard = material_span.enter();
    send_event(
        tx,
        QaEvent::PhaseChanged {
            phase: QaPhase::MaterialProcessing {
                strategy: "v3_registry".to_string(),
                total_chunks: None,
            },
        },
    )
    .await?;

    let mut registry = MaterialRegistry::default();

    // Track current-turn material summaries for injection into user prompt
    let mut current_turn_materials: Vec<String> = Vec::new();

    // Track inline material ID for material_refs persistence
    let mut inline_material_id: Option<String> = None;

    // Register inline material
    if let Some(ref inline_text) = request.material.inline {
        // M11: Enforce inline material size limit
        let content = if inline_text.len() > config.max_inline_chars {
            tracing::warn!(
                len = inline_text.len(),
                max = config.max_inline_chars,
                "Inline material exceeds size limit, truncating"
            );
            inline_text
                .chars()
                .take(config.max_inline_chars)
                .collect::<String>()
        } else {
            inline_text.clone()
        };
        let id = format!("inline-{}", Uuid::now_v7().simple());
        let total_lines = content.lines().count();
        inline_material_id = Some(id.clone());
        registry.register_inline(InlineText {
            id: id.clone(),
            label: "用户粘贴文本".to_string(),
            content,
            total_lines,
        });
        current_turn_materials.push(format!("{id} 用户粘贴文本 ({total_lines}行)"));
    }

    // Register document materials
    if !request.material.document_ids.is_empty() {
        let requested_ids: std::collections::HashSet<Uuid> =
            request.material.document_ids.iter().copied().collect();
        let docs = kb_documents::Entity::find()
            .filter(kb_documents::Column::TenantId.eq(tenant_id))
            .filter(kb_documents::Column::Id.is_in(request.material.document_ids.clone()))
            .all(db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to fetch documents by ID");
                let _ = send_event_blocking(
                    tx,
                    QaEvent::Error {
                        message: format!("Failed to fetch documents: {e}"),
                    },
                );
            })?;

        // M12: Warn about requested documents not found or not accessible for this tenant
        let found_ids: std::collections::HashSet<Uuid> =
            docs.iter().map(|d| d.id).collect();
        let missing_ids: Vec<&Uuid> = requested_ids.difference(&found_ids).collect();
        if !missing_ids.is_empty() {
            tracing::warn!(
                ?missing_ids,
                "Some requested documents not found or not accessible for this tenant"
            );
        }

        for doc in &docs {
            register_doc_from_model(&mut registry, doc);
            let total_lines = doc.full_text.as_deref().map_or(0, |t| t.lines().count());
            current_turn_materials
                .push(format!("{} {} ({}行)", doc.id, doc.title, total_lines));
        }
    }

    // Resolve file_ids → documents
    if !request.material.file_ids.is_empty() {
        let docs = kb_documents::Entity::find()
            .filter(kb_documents::Column::TenantId.eq(tenant_id))
            .filter(kb_documents::Column::FileId.is_in(request.material.file_ids.clone()))
            .all(db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to resolve file_ids to documents");
                let _ = send_event_blocking(
                    tx,
                    QaEvent::Error {
                        message: format!("Failed to resolve file_ids: {e}"),
                    },
                );
            })?;

        for doc in &docs {
            register_doc_from_model(&mut registry, doc);
            let total_lines = doc.full_text.as_deref().map_or(0, |t| t.lines().count());
            current_turn_materials
                .push(format!("{} {} ({}行)", doc.id, doc.title, total_lines));
        }
    }

    // Recover materials from history messages
    for msg in &history {
        let Some(ref refs) = msg.material_refs else {
            continue;
        };

        // Extract document_ids from material_refs
        if let Some(doc_ids) = refs.get("documentIds").and_then(|v| v.as_array()) {
            let ids: Vec<Uuid> = doc_ids
                .iter()
                .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect();
            if !ids.is_empty() {
                let docs = kb_documents::Entity::find()
                    .filter(kb_documents::Column::TenantId.eq(tenant_id))
                    .filter(kb_documents::Column::Id.is_in(ids))
                    .all(db)
                    .await
                    .unwrap_or_default();
                for doc in &docs {
                    register_doc_from_model(&mut registry, doc);
                }
            }
        }

        // Extract file_ids from material_refs
        if let Some(file_ids) = refs.get("fileIds").and_then(|v| v.as_array()) {
            let ids: Vec<Uuid> = file_ids
                .iter()
                .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect();
            if !ids.is_empty() {
                let docs = kb_documents::Entity::find()
                    .filter(kb_documents::Column::TenantId.eq(tenant_id))
                    .filter(kb_documents::Column::FileId.is_in(ids))
                    .all(db)
                    .await
                    .unwrap_or_default();
                for doc in &docs {
                    register_doc_from_model(&mut registry, doc);
                }
            }
        }

        // Extract inline text from material_refs
        if let Some(inline_obj) = refs.get("inline").and_then(|v| v.as_object()) {
            if let Some(content) = inline_obj.get("content").and_then(|v| v.as_str()) {
                let id = inline_obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("recovered-inline")
                    .to_string();
                // Avoid duplicate registration
                if registry.get_inline(&id).is_none() {
                    registry.register_inline(InlineText {
                        id,
                        label: inline_obj
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("历史粘贴文本")
                            .to_string(),
                        content: content.to_string(),
                        total_lines: content.lines().count(),
                    });
                }
            }
        }
    }

    let registry = Arc::new(registry);

    // Count materials for tracing
    let material_count = registry.all_materials().len();
    material_span.record("material_count", material_count);
    tracing::info!(
        material_count,
        "Materials registered: count={}",
        material_count,
    );
    drop(material_guard);

    // Build material_refs JSON for the user message
    let material_refs_json: Option<serde_json::Value> =
        if request.material.inline.is_some()
            || !request.material.file_ids.is_empty()
            || !request.material.document_ids.is_empty()
        {
            let mut refs = serde_json::json!({});
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
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>());
            }
            if !request.material.document_ids.is_empty() {
                refs["documentIds"] = serde_json::json!(request
                    .material
                    .document_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>());
            }
            Some(refs)
        } else {
            None
        };

    let has_material = material_refs_json.is_some();

    // Save user message
    let user_msg = chat_service::create_message(
        db,
        &CreateMessageParams {
            session_id: session.id,
            tenant_id,
            user_id,
            role: "user".to_string(),
            content: request.instruction.clone(),
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

    // Index user message to chat_memory (fire-and-forget)
    spawn_index_message(
        memory_store,
        embedding_client,
        &SpawnIndexParams {
            embedding_model_name: embedding_model_name.to_string(),
            session_id: session.id,
            tenant_id,
            msg_id: user_msg.id,
            role: "user".to_string(),
            content: request.instruction.clone(),
            has_material,
            turn_index: (history.len() as i32 + 1) / 2,
        },
    );

    // ── Step 2: Construct tools ─────────────────────────────────────
    let list_tool = ListMaterialsTool {
        registry: registry.clone(),
    };
    let read_tool = ReadMaterialTool {
        registry: registry.clone(),
    };
    let search_material_tool = SearchMaterialTool {
        registry: registry.clone(),
    };
    let search_kb_tool = SearchKnowledgeBaseTool {
        embedding_client: embedding_client.clone(),
        search_provider: search_provider.clone(),
        embedding_model_name: embedding_model_name.to_string(),
        tenant_id,
        user_id,
    };
    let conversation_db: std::sync::Arc<sea_orm::DatabaseConnection> =
        std::sync::Arc::new(db.clone());
    let list_history_tool = ListConversationHistoryTool {
        db: conversation_db.clone(),
        session_id: session.id,
        tenant_id,
        user_id,
    };
    let read_turn_tool = ReadConversationTurnTool {
        db: conversation_db,
        session_id: session.id,
        tenant_id,
        user_id,
    };

    // ── Step 2b: Register frontend tool stubs ──────────────────────
    let page_tool_stubs: Vec<FrontendToolStub> = request
        .page_tools
        .iter()
        .map(|def| FrontendToolStub {
            name: def.name.clone(),
            description: def.description.clone(),
            parameters: def.parameters.clone(),
            broker: broker.clone(),
            sse_tx: tx.clone(),
        })
        .collect();

    // ── Step 3: Compaction ──────────────────────────────────────────
    let compaction_span = tracing::info_span!("qa.compaction");
    let compaction_span_guard = compaction_span.enter();

    // Token-based trigger check
    let history_tokens: usize = history.iter().map(estimate_message_tokens).sum();
    let token_threshold =
        config.max_context_tokens as usize - config.compaction_reserve_tokens;
    let needs_compaction =
        history_tokens > token_threshold && history.len() > config.compaction_threshold;

    // Fixed recent window
    let recent_turns = config.compaction_recent_turns;
    let recent_start = if needs_compaction {
        history.len().saturating_sub(recent_turns)
    } else {
        0
    };
    let recent_history: &[chat_messages::Model] = if needs_compaction {
        &history[recent_start..]
    } else {
        &history[..]
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
    drop(compaction_span_guard);

    // Read cached summary (non-blocking)
    let summary = if needs_compaction && recent_start > 0 {
        super::qa_compaction_service::get_cached_summary(db, session.id, tenant_id)
            .await
            .ok()
            .flatten()
            .map(|c| c.summary)
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Spawn background compaction when needed (fire-and-forget).
    // Uses CompactionGuard DashMap to prevent duplicate runs for the same session.
    if needs_compaction && recent_start > 0 {
        let guard_key = session.id;
        let mut should_spawn = compaction_guard.entry(guard_key).or_insert(false);
        if !should_spawn.value() {
            *should_spawn.value_mut() = true;

            let bg_db = db.clone();
            let bg_qa_client = qa_client.clone();
            let bg_session_id = session.id;
            let bg_tenant_id = tenant_id;
            let bg_history: Vec<chat_messages::Model> = history.clone();
            let bg_model = config.model.clone();
            let bg_threshold = config.compaction_threshold;
            let bg_recent_turns = config.compaction_recent_turns;
            let bg_max_ctx = config.max_context_tokens;
            let bg_reserve = config.compaction_reserve_tokens;
            let bg_guard = compaction_guard.clone();

            tokio::spawn(async move {
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
                    Ok(summary) => {
                        tracing::info!(
                            session_id = %bg_session_id,
                            summary_len = summary.len(),
                            "Background compaction completed"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            session_id = %bg_session_id,
                            error = %e,
                            "Background compaction failed"
                        );
                    }
                }

                // Remove guard entry regardless of success/failure
                bg_guard.remove(&bg_session_id);
            });
        }
    }

    // ── Step 3b: Build agent with PromptHook ─────────────────────────
    let hook = QaV3Hook::new(tx.clone());
    let records_hook = hook.clone(); // Keep a clone to extract tool records after streaming

    let material_hint = {
        let mats = registry.all_materials();
        if mats.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n## 当前会话材料状态\n当前会话中有 **{}** 份可用材料。请始终先调用 `list_materials` 查看详情，再根据需要调用 `read_material` 读取内容。",
                mats.len()
            )
        }
    };

    // Build system_prompt with summary
    let mut system_prompt = format!("{BASE_SYSTEM_PROMPT}{material_hint}\n\n");
    if !summary.is_empty() {
        system_prompt.push_str(&format!("\n\n[对话历史摘要]\n{summary}\n"));
    }

    // Inject page context when present
    if !request.page_context.is_empty() {
        let active_ctx = request.page_context.iter().find(|c| c.active);
        let page_list: Vec<String> = request
            .page_context
            .iter()
            .map(|c| {
                let marker = if c.active { " ← 当前" } else { "" };
                format!(
                    "- 「{}」路由: {} (意图: {}){}",
                    c.title, c.route, c.intent, marker
                )
            })
            .collect();

        let active_title = active_ctx.map_or("未知", |c| c.title.as_str());
        let active_route = active_ctx.map_or("未知", |c| c.route.as_str());

        system_prompt.push_str(&format!(
            "\n\n## 已注册页面上下文\n\
             当前活跃页面：「{}」（路由: {}）\n\n\
             所有已注册页面：\n{}\n\n\
             你可以使用 page_ 前缀的工具来查询和操作页面数据。工具的 targetPage 参数用于指定目标页面（默认当前活跃页面）。\n\
             使用原则：不猜测字段名、先确认再操作、高风险操作需确认。\n\
             你可以调用 list_available_pages 查看系统所有可访问页面，调用 navigate_to_page 帮助用户切换页面。\n\n\
             ## 重要：工具调用纪律\n\
             - 如果需要执行操作（创建、编辑、删除等），**必须直接调用对应的 tool**，不要用文字描述你打算做什么。\n\
             - 错误示例：\"现在我将执行编辑操作：\"（然后停止）→ 用户什么都没得到。\n\
             - 正确示例：直接调用 page_execute_action，参数齐全。\n\
             - 如果缺少必要参数，**直接向用户询问缺失的参数**，不要假装要执行然后停下来。",
            active_title, active_route,
            page_list.join("\n")
        ));
    }

    // ── Step 3c: Semantic recall with dedup ────────────────────────
    let recall_span =
        tracing::info_span!("qa.recall", strategy = %config.history_strategy);
    let recall_guard = recall_span.enter();
    let relevant_context = if config.history_strategy == "none" {
        None
    } else {
        let strategy = match config.history_strategy.as_str() {
            "retrieve" => Some(memory_service::HistoryStrategy::RetrieveRelevant {
                top_k: config.semantic_top_k,
            }),
            "original" => Some(memory_service::HistoryStrategy::ReadOriginalMaterial),
            "hybrid" => Some(memory_service::HistoryStrategy::Hybrid {
                recent_turns: config.compaction_recent_turns,
                top_k: config.semantic_top_k,
            }),
            other => {
                tracing::warn!(
                    strategy = other,
                    "Unknown history strategy, skipping recall"
                );
                None
            }
        };

        if let Some(strategy) = strategy {
            let recalled = memory_service::recall_history(
                &embedding_client.0,
                &memory_store.client,
                &memory_store.collection_name,
                embedding_model_name,
                &strategy,
                &RecallHistoryParams {
                    session_id: session.id,
                    tenant_id,
                    query: &request.instruction,
                    history_db: &history,
                },
            )
            .await
            .unwrap_or_default();

            // Dedup: exclude messages already in recent_history
            let recent_msg_ids: std::collections::HashSet<Uuid> =
                recent_history.iter().map(|m| m.id).collect();

            let mut deduped = recalled;
            deduped
                .relevant_messages
                .retain(|m| !recent_msg_ids.contains(&m.message_id));

            let ctx_text = memory_service::format_recalled_context(&deduped);
            if ctx_text.is_empty() {
                None
            } else {
                Some(ctx_text)
            }
        } else {
            None
        }
    };

    recall_span.record("has_recall", relevant_context.is_some());
    drop(recall_guard);

    // Inject semantic recall context into system_prompt
    if let Some(ref ctx) = relevant_context {
        system_prompt.push_str(ctx);
    }

    // Token budget uses system_prompt_tokens AFTER summary + semantic recall injection
    let system_prompt_tokens = estimate_text_tokens(&system_prompt, false);

    // ── Step 4: Build agent + stream ─────────────────────────────────
    let agent_span = tracing::info_span!(
        "qa.agent",
        model = %config.model,
        provider = %config.provider,
        system_prompt_tokens = system_prompt_tokens,
    );
    let agent_guard = agent_span.enter();

    send_event(
        tx,
        QaEvent::PhaseChanged {
            phase: QaPhase::GeneratingAnswer,
        },
    )
    .await?;

    // Wrap user instruction with current-turn material hint
    let user_prompt = if current_turn_materials.is_empty() {
        request.instruction.clone()
    } else {
        format!(
            "[本轮新提交材料: {}]\n\n{}",
            current_turn_materials.join(", "),
            request.instruction
        )
    };

    // Use recent_history (compaction-aware) instead of full history
    let chat_history = build_chat_history_with_budget(
        recent_history,
        config.max_context_tokens,
        config.response_reserve_tokens,
        system_prompt_tokens,
    );

    tracing::info!(
        system_prompt_len = system_prompt.len(),
        system_prompt_tokens,
        chat_history_messages = chat_history.len(),
        user_prompt_len = user_prompt.len(),
        model = %config.model,
        provider = %config.provider,
        max_context_tokens = config.max_context_tokens,
        "Agent: model={} provider={} prompt_tokens={} history_msgs={} user_len={}",
        config.model,
        config.provider,
        system_prompt_tokens,
        chat_history.len(),
        user_prompt.len(),
    );

    // Build debug context for this turn
    {
        use rig::completion::message::{AssistantContent, Message};
        let debug_context = serde_json::json!({
            "system_prompt": system_prompt,
            "system_prompt_tokens": system_prompt_tokens,
            "user_prompt": user_prompt,
            "chat_history_summary": {
                "message_count": chat_history.len(),
                "messages": chat_history.iter().map(|msg| {
                    match msg {
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
                        Message::System { .. } => serde_json::json!({ "role": "other" })
                    }
                }).collect::<Vec<_>>()
            },
            "config_snapshot": {
                "model": config.model,
                "provider": config.provider,
                "max_context_tokens": config.max_context_tokens,
                "temperature": config.temperature,
                "history_strategy": config.history_strategy,
                "compaction_threshold": config.compaction_threshold,
                "compaction_recent_turns": config.compaction_recent_turns,
            },
            "compaction": {
                "triggered": needs_compaction,
                "summary_length": summary.len(),
                "recent_start": recent_start,
                "history_total": history.len(),
                "history_tokens": history_tokens,
            },
            "semantic_recall": {
                "strategy": config.history_strategy,
                "context_length": relevant_context.as_ref().map_or(0, |c| c.len()),
                "has_recall": relevant_context.is_some(),
            }
        });
        records_hook.set_debug_context(debug_context);
    }

    let mut agent_builder = qa_client
        .0
        .agent(&config.model)
        .preamble(&system_prompt)
        .hook(hook)
        .tool(list_tool)
        .tool(read_tool)
        .tool(search_material_tool)
        .tool(search_kb_tool)
        .tool(list_history_tool)
        .tool(read_turn_tool);

    // Register frontend page tool stubs
    for stub in page_tool_stubs {
        agent_builder = agent_builder.tool(stub);
    }

    let mut agent_builder = agent_builder.default_max_turns(15);

    // Ollama defaults num_ctx=4096, silently truncating prompts that exceed it.
    // Only set for Ollama provider; DeepSeek/OpenAI ignore this parameter.
    if config.provider == "ollama" {
        agent_builder = agent_builder.additional_params(serde_json::json!({
            "options": { "num_ctx": config.max_context_tokens }
        }));
    }

    let agent = agent_builder.build();

    let mut stream = agent
        .stream_chat(&user_prompt, chat_history)
        .multi_turn(15)
        .await;

    let mut final_answer = String::new();
    let mut tool_call_count: u32 = 0;
    let mut captured_usage: Option<rig::completion::Usage> = None;
    let mut client_connected = true;

    while let Some(item) = stream.next().await {
        match item {
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::Text(text),
            )) => {
                final_answer.push_str(&text.text);
                if send_event(tx, QaEvent::AnswerToken { token: text.text })
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
                // ToolCallStarted / ToolCallCompleted events are emitted by QaV3Hook
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
            Ok(MultiTurnStreamItem::StreamUserItem(_)) => {
                // Tool results flowing back — already handled by hook
            }
            Err(e) => {
                // Always persist error information to DB so it survives page refresh.
                let tool_records = records_hook.take_tool_records();
                let content_parts = records_hook.take_content_parts();
                let mut debug_context = records_hook.take_debug_context();
                let tool_rounds = records_hook.take_tool_rounds();

                // Inject tool_rounds into debug_context
                if !tool_rounds.is_empty() {
                    if let Some(ctx) = debug_context.as_mut() {
                        if let Some(obj) = ctx.as_object_mut() {
                            obj.insert(
                                "toolRounds".to_string(),
                                serde_json::json!(tool_rounds),
                            );
                        }
                    }
                }

                let tool_usage_json = if tool_records.is_empty()
                    && content_parts.is_empty()
                    && debug_context.is_none()
                {
                    None
                } else {
                    let mut obj = serde_json::Map::new();
                    if !tool_records.is_empty() {
                        obj.insert(
                            "toolCalls".to_string(),
                            serde_json::json!(tool_records),
                        );
                    }
                    if !content_parts.is_empty() {
                        obj.insert(
                            "contentParts".to_string(),
                            serde_json::json!(content_parts),
                        );
                    }
                    if let Some(ctx) = debug_context {
                        obj.insert("debugContext".to_string(), ctx);
                    }
                    Some(serde_json::Value::Object(obj))
                };
                let error_content = if final_answer.is_empty() {
                    format!("⚠️ 回答生成失败：{e}")
                } else {
                    format!("{final_answer}\n\n⚠️ 后续生成失败：{e}")
                };
                let _ = chat_service::create_message(
                    db,
                    &CreateMessageParams {
                        session_id: session.id,
                        tenant_id,
                        user_id,
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

                let _ = send_event(
                    tx,
                    QaEvent::Error {
                        message: e.to_string(),
                    },
                )
                .await;
                client_connected = false;
                break;
            }
            _ => {
                // Reasoning, ToolCallDelta, etc. — forward silently
            }
        }
    }

    agent_span.record("answer_len", final_answer.len());
    agent_span.record("tool_call_count", tool_call_count);
    drop(agent_guard);

    // ── Step 5: Persistence ─────────────────────────────────────────
    let persist_span = tracing::info_span!("qa.persist");
    let persist_guard = persist_span.enter();

    if client_connected {
        send_event(
            tx,
            QaEvent::PhaseChanged {
                phase: QaPhase::Persisting,
            },
        )
        .await?;
    }

    // Collect tool call records and ordered content parts from the hook
    let tool_records = records_hook.take_tool_records();
    let content_parts = records_hook.take_content_parts();
    let mut debug_context = records_hook.take_debug_context();
    let tool_rounds = records_hook.take_tool_rounds();

    // Inject tool_rounds into debug_context (captured during streaming loop)
    if !tool_rounds.is_empty() {
        if let Some(ctx) = debug_context.as_mut() {
            if let Some(obj) = ctx.as_object_mut() {
                obj.insert("toolRounds".to_string(), serde_json::json!(tool_rounds));
            }
        }
    }
    let tool_usage_json =
        if tool_records.is_empty() && content_parts.is_empty() && debug_context.is_none()
        {
            None
        } else {
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
        };

    // Collect citations from search_knowledge_base tool calls
    let citations = records_hook.take_citations();

    let (prompt_tokens, completion_tokens, total_tokens) =
        captured_usage.map_or((0, 0, 0), |u| {
            (
                u.input_tokens as i32,
                u.output_tokens as i32,
                u.total_tokens as i32,
            )
        });

    let assistant_msg = chat_service::create_message(
        db,
        &CreateMessageParams {
            session_id: session.id,
            tenant_id,
            user_id,
            role: "assistant".to_string(),
            content: final_answer.clone(),
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

    // Index assistant message to chat_memory (fire-and-forget)
    spawn_index_message(
        memory_store,
        embedding_client,
        &SpawnIndexParams {
            embedding_model_name: embedding_model_name.to_string(),
            session_id: session.id,
            tenant_id,
            msg_id: assistant_msg.id,
            role: "assistant".to_string(),
            content: final_answer.clone(),
            has_material: false,
            turn_index: ((history.len() + 1) as i32 + 1) / 2,
        },
    );

    // Update session title if first message
    if session.title.is_none() {
        let title: String = request.instruction.chars().take(50).collect();
        let _ = chat_service::update_session_title(
            db, session.id, tenant_id, user_id, &title,
        )
        .await;
    }

    // ── Step 6: Complete ────────────────────────────────────────────
    persist_span.record("answer_len", final_answer.len());
    persist_span.record("tool_call_count", tool_records.len());
    drop(persist_guard);

    let usage = TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    };

    if client_connected {
        send_event(
            tx,
            QaEvent::Completed {
                response: QaStreamResponse {
                    answer: final_answer,
                    session_id: session_id_str,
                    citations,
                    intent: "v3_agent".to_string(),
                    output_format: "free_text".to_string(),
                    strategy: "agent_tool_calling".to_string(),
                    mode: "v3".to_string(),
                    usage,
                },
            },
        )
        .await?;
    }

    Ok(())
}
