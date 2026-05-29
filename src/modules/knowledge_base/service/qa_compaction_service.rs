//! Phase 4 Step 3: Compaction service with iterative merge.
//!
//! Provides token-based trigger, iterative summary merge, and async integration
//! for long-running conversation context management.

use chrono::Utc;
use rig::client::CompletionClient;
use rig::completion::Prompt;
use sea_orm::{ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::models::_entities::{chat_messages, chat_sessions};
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::models::chat_sessions as cs_models;
use crate::utils::error::IntoAppError;

use super::qa_v3_service::estimate_message_tokens;

// ---------------------------------------------------------------------------
// Summary cache struct
// ---------------------------------------------------------------------------

/// Cached summary stored in `chat_sessions.summary_cache`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SummaryCache {
    pub summary: String,
    pub last_compacted_msg_id: Uuid,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MIN_MESSAGES_FOR_RECOMPACT: usize = 4;
const SUMMARY_PREAMBLE: &str = "你是一个对话摘要助手。请简洁准确地总结对话要点。";
const INITIAL_SUMMARY_PROMPT: &str = "\
请用中文总结以下对话的关键信息，按以下结构输出：
1. **主要话题**：讨论的核心问题
2. **材料引用**：提到的材料名称和关键结论
3. **用户关注点**：用户反复追问或特别关心的方面
4. **已解决的问题**：得出了什么结论
5. **待跟进事项**：尚未解决或需要继续讨论的问题";
const MERGE_SUMMARY_PROMPT: &str = "\
以下是已有的对话摘要和新增的对话内容。请将新增内容合并进现有摘要中，保持原有结构不变：
- 如果新内容涉及已记录的话题，补充更新
- 如果新内容是全新话题，追加到相应章节
- 不要删除现有摘要中的任何信息
- 保持简洁，总长度不超过 1500 字";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format older messages for LLM summarization.
///
/// For assistant messages with tool_calls: show `result_preview` preferentially,
/// text content truncated to 200 chars.
/// For normal messages: truncate to 800 chars.
/// Labels: 用户 / 助手 / 系统.
fn format_history_for_summary(messages: &[chat_messages::Model]) -> String {
    let mut parts = Vec::new();

    for msg in messages {
        let role_label = match msg.role.as_str() {
            "user" => "用户",
            "assistant" => "助手",
            "system" => "系统",
            _ => continue,
        };

        // For assistant messages with tool_calls, show result_previews preferentially
        if msg.role == "assistant" {
            if let Some(ref usage) = msg.token_usage {
                if let Some(tool_calls) =
                    usage.get("toolCalls").and_then(|v| v.as_array())
                {
                    if !tool_calls.is_empty() {
                        let mut block = String::new();

                        // Truncate text content to 200 chars
                        if !msg.content.is_empty() {
                            let truncated: String =
                                msg.content.chars().take(200).collect();
                            let ellipsis = if msg.content.chars().count() > 200 {
                                "…"
                            } else {
                                ""
                            };
                            block.push_str(&format!("{role_label}: {truncated}"));
                            block.push_str(ellipsis);
                            block.push('\n');
                        }

                        // Show tool call result previews
                        for tc in tool_calls {
                            let tool_name = tc
                                .get("toolName")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            if let Some(preview) =
                                tc.get("resultPreview").and_then(|v| v.as_str())
                            {
                                block.push_str(&format!(
                                    "[工具调用 {tool_name}] {preview}\n"
                                ));
                            }
                        }

                        parts.push(block);
                        continue;
                    }
                }
            }
        }

        // Normal messages: truncate to 800 chars
        let truncated: String = msg.content.chars().take(800).collect();
        let ellipsis = if msg.content.chars().count() > 800 {
            "…"
        } else {
            ""
        };
        parts.push(format!("{role_label}: {truncated}{ellipsis}"));
    }

    parts.join("\n")
}

// ---------------------------------------------------------------------------
// Cache read / write
// ---------------------------------------------------------------------------

/// Read cached summary from `chat_sessions.summary_cache`.
#[tracing::instrument(skip(db))]
pub async fn get_cached_summary(
    db: &sea_orm::DatabaseConnection,
    session_id: Uuid,
    tenant_id: Uuid,
) -> Result<Option<SummaryCache>, KnowledgeBaseError> {
    let session = chat_sessions::Entity::find_by_id(session_id)
        .filter(chat_sessions::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .map_err(|e| KnowledgeBaseError::IndexingError(e.to_string()))?
        .ok_or(KnowledgeBaseError::NotFound)?;

    match session.summary_cache {
        Some(ref json_str) => {
            let cache: SummaryCache = serde_json::from_str(json_str)
                .map_err(|e| KnowledgeBaseError::ConfigError(e.to_string()))?;
            Ok(Some(cache))
        }
        None => Ok(None),
    }
}

/// Write summary cache to `chat_sessions.summary_cache`.
#[tracing::instrument(skip(db, summary))]
async fn cache_summary(
    db: &sea_orm::DatabaseConnection,
    session_id: Uuid,
    tenant_id: Uuid,
    summary: &str,
    last_compacted_msg_id: Uuid,
) -> Result<(), KnowledgeBaseError> {
    let cache = SummaryCache {
        summary: summary.to_string(),
        last_compacted_msg_id,
        created_at: Utc::now().naive_utc().to_string(),
    };

    let json_str = serde_json::to_string(&cache)
        .map_err(|e| KnowledgeBaseError::ConfigError(e.to_string()))?;

    // Fetch current session row
    let session = chat_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .map_err(|e| KnowledgeBaseError::IndexingError(e.to_string()))?
        .ok_or(KnowledgeBaseError::NotFound)?;

    let mut active: cs_models::ActiveModel = session.into();
    active.summary_cache = ActiveValue::Set(Some(json_str));
    active.update(db).await.db_err().map_err(|e| {
        tracing::error!(error = %e, "Failed to update summary_cache");
        KnowledgeBaseError::IndexingError(e.to_string())
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Core: compact_history
// ---------------------------------------------------------------------------

/// Parameters for [`compact_history`].
#[derive(Debug)]
pub struct CompactHistoryParams {
    pub session_id: Uuid,
    pub tenant_id: Uuid,
    pub max_context_tokens: i32,
    pub recent_turns: usize,
    pub compaction_reserve_tokens: usize,
}

/// Token-based trigger + iterative summary merge.
///
/// 1. Token-based trigger: total estimated tokens > (max_context_tokens - compaction_reserve_tokens)
///    AND history.len() > compaction_threshold (safety net).
/// 2. Fixed recent window: keep last `recent_turns` messages intact.
/// 3. Cache check: if cached summary exists and new messages < MIN_MESSAGES_FOR_RECOMPACT, return cache.
/// 4. Iterative merge:
///    - Cached summary exists → format only NEW messages, merge with MERGE_SUMMARY_PROMPT.
///    - No cache → format ALL older messages, use INITIAL_SUMMARY_PROMPT.
/// 5. Non-streaming LLM call via rig Prompt trait.
/// 6. Cache the result.
#[tracing::instrument(skip(db, qa_client, history, params))]
pub async fn compact_history(
    db: &sea_orm::DatabaseConnection,
    qa_client: &rig::providers::deepseek::Client,
    history: &[chat_messages::Model],
    summary_model: &str,
    compaction_threshold: usize,
    params: &CompactHistoryParams,
) -> Result<String, KnowledgeBaseError> {
    // ── 1. Token-based trigger ────────────────────────────────────
    let history_tokens: usize = history.iter().map(estimate_message_tokens).sum();
    let token_threshold = (params.max_context_tokens as usize)
        .saturating_sub(params.compaction_reserve_tokens);
    let needs_compaction =
        history_tokens > token_threshold && history.len() > compaction_threshold;

    if !needs_compaction {
        return Ok(String::new());
    }

    // ── 2. Fixed recent window ────────────────────────────────────
    let boundary_idx = history.len().saturating_sub(params.recent_turns);
    if boundary_idx == 0 {
        // Not enough messages to split
        return Ok(String::new());
    }

    let older_messages = &history[..boundary_idx];

    // ── 3. Cache check ────────────────────────────────────────────
    let cached = get_cached_summary(db, params.session_id, params.tenant_id).await?;

    // Count new messages since last compaction
    let new_msg_count = cached.as_ref().map_or(older_messages.len(), |c| {
        older_messages
            .iter()
            .rev()
            .take_while(|m| m.id != c.last_compacted_msg_id)
            .count()
    });

    // If cached summary exists and too few new messages, reuse cache
    if let Some(ref c) = cached {
        if new_msg_count < MIN_MESSAGES_FOR_RECOMPACT {
            tracing::debug!(
                new_msg_count,
                min_for_recompact = MIN_MESSAGES_FOR_RECOMPACT,
                "Reusing cached summary — too few new messages"
            );
            return Ok(c.summary.clone());
        }
    }

    // ── 4. Iterative merge ────────────────────────────────────────
    let (prompt, last_compacted_msg_id) = cached.as_ref().map_or_else(
        || {
            // Initial mode: format ALL older messages
            let text = format_history_for_summary(older_messages);
            let prompt = format!("{INITIAL_SUMMARY_PROMPT}\n\n{text}");
            let last_id = older_messages.last().map(|m| m.id).unwrap_or_default();
            (prompt, last_id)
        },
        |c| {
            // Merge mode: only format NEW messages since last compaction
            let new_messages: Vec<chat_messages::Model> = older_messages
                .iter()
                .rev()
                .take_while(|m| m.id != c.last_compacted_msg_id)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();

            let new_text = format_history_for_summary(&new_messages);

            let prompt = format!(
                "{}\n\n## 已有摘要\n{}\n\n## 新增对话内容\n{}",
                MERGE_SUMMARY_PROMPT, c.summary, new_text
            );

            // The last compacted msg is the last of the older_messages
            let last_id = older_messages
                .last()
                .map_or(c.last_compacted_msg_id, |m| m.id);

            (prompt, last_id)
        },
    );

    // ── 5. Non-streaming LLM call ────────────────────────────────
    tracing::info!(
        session_id = %params.session_id,
        prompt_len = prompt.len(),
        "Running compaction LLM call"
    );

    let summary: String = qa_client
        .agent(summary_model)
        .preamble(SUMMARY_PREAMBLE)
        .build()
        .prompt(&prompt)
        .await
        .map_err(|e: rig::completion::PromptError| {
            tracing::error!(error = %e, "Compaction LLM call failed");
            KnowledgeBaseError::ProviderError(e.to_string())
        })?;

    tracing::info!(
        session_id = %params.session_id,
        summary_len = summary.len(),
        "Compaction summary generated"
    );

    // ── 6. Cache the result ───────────────────────────────────────
    cache_summary(
        db,
        params.session_id,
        params.tenant_id,
        &summary,
        last_compacted_msg_id,
    )
    .await?;

    Ok(summary)
}
