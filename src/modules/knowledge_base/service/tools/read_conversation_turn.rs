use std::fmt;
use std::sync::Arc;

use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use rig::completion::ToolDefinition;
use rig::tool::Tool;

use crate::models::_entities::chat_messages;
use crate::modules::knowledge_base::service::chat_service;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ReadConversationTurnError(pub String);

impl fmt::Display for ReadConversationTurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "read conversation turn error: {}", self.0)
    }
}

impl std::error::Error for ReadConversationTurnError {}

// ---------------------------------------------------------------------------
// Args
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReadConversationTurnArgs {
    /// 起始轮次号（从 1 开始）
    pub start_turn: u32,
    /// 结束轮次号（含，与 `start_turn` 相同时只读一轮）— 默认等于 `start_turn`
    pub end_turn: Option<u32>,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_CHARS_PER_MESSAGE: usize = 2000;
const MAX_TURNS_PER_READ: u32 = 10;
const PREVIEW_CHARS_FOR_TOOL_RESULT: usize = 500;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Truncate `content` to at most `max` Unicode characters, appending `…` if
/// truncation occurred.
fn truncate_to_chars(content: &str, max: usize) -> String {
    if content.chars().count() <= max {
        content.to_string()
    } else {
        let truncated: String = content.chars().take(max).collect();
        format!("{truncated}…")
    }
}

// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReadConversationTurnTool {
    #[serde(skip)]
    pub db: Arc<DatabaseConnection>,
    #[serde(skip)]
    pub session_id: Uuid,
    #[serde(skip)]
    pub tenant_id: Uuid,
    #[serde(skip)]
    pub user_id: Uuid,
}

// ---------------------------------------------------------------------------
// Tool trait implementation
// ---------------------------------------------------------------------------

impl Tool for ReadConversationTurnTool {
    const NAME: &'static str = "read_conversation_turn";

    type Error = ReadConversationTurnError;
    type Args = ReadConversationTurnArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "读取指定轮次（或范围）的完整对话内容。单轮时 start_turn 与 end_turn 传相同值；多轮时自行控制范围。每条消息截断到 2000 字符。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "start_turn": {
                        "type": "integer",
                        "description": "起始轮次号（从 1 开始）"
                    },
                    "end_turn": {
                        "type": "integer",
                        "description": "结束轮次号（含，与 start_turn 相同时只读一轮）— 默认等于 start_turn"
                    }
                },
                "required": ["start_turn"]
            }),
        }
    }

    #[tracing::instrument(
        skip(self, args),
        fields(tool = "read_conversation_turn", start_turn = %args.start_turn)
    )]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let messages = chat_service::get_session_messages(
            &self.db,
            self.session_id,
            self.tenant_id,
            self.user_id,
        )
        .await
        .map_err(|e| ReadConversationTurnError(format!("加载会话消息失败: {e}")))?;

        let turns = pair_messages_into_turns(&messages);

        let total_turns = u32::try_from(turns.len()).unwrap_or(u32::MAX);

        if total_turns == 0 {
            return Ok("当前会话暂无对话历史。".to_string());
        }

        if args.start_turn < 1 {
            return Ok("轮次号从 1 开始".to_string());
        }

        if args.start_turn > total_turns {
            return Ok(format!(
                "当前会话共 {} 轮对话，请求的第 {} 轮不存在",
                total_turns, args.start_turn
            ));
        }

        let end_turn = args
            .end_turn
            .unwrap_or(args.start_turn)
            .min(args.start_turn + MAX_TURNS_PER_READ - 1)
            .min(total_turns);

        let output = render_turns(&turns, args.start_turn, end_turn);

        Ok(output)
    }
}

struct AssistantData {
    content: String,
    tool_calls: Option<serde_json::Value>,
}

struct Turn {
    user_content: String,
    assistant: Option<AssistantData>,
}

fn pair_messages_into_turns(messages: &[chat_messages::Model]) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "user" => {
                turns.push(Turn {
                    user_content: msg.content.clone(),
                    assistant: None,
                });
            }
            "assistant" => {
                if let Some(last) = turns.last_mut() {
                    if last.assistant.is_none() {
                        last.assistant = Some(AssistantData {
                            content: msg.content.clone(),
                            tool_calls: msg.token_usage.clone(),
                        });
                        continue;
                    }
                }
                turns.push(Turn {
                    user_content: String::new(),
                    assistant: Some(AssistantData {
                        content: msg.content.clone(),
                        tool_calls: msg.token_usage.clone(),
                    }),
                });
            }
            _ => {}
        }
    }

    turns
}

fn render_turns(turns: &[Turn], start_turn: u32, end_turn: u32) -> String {
    let mut output_parts: Vec<String> = Vec::new();

    for turn_idx in start_turn..=end_turn {
        let turn_index = usize::try_from(turn_idx - 1).unwrap_or(usize::MAX);
        let turn = &turns[turn_index];
        let mut parts: Vec<String> = Vec::new();

        parts.push(format!("=== 第 {turn_idx} 轮对话 ==="));
        parts.push(String::new());

        let user_char_count = turn.user_content.chars().count();
        parts.push(format!(
            "[用户] ({} 字符):\n{}",
            user_char_count,
            truncate_to_chars(&turn.user_content, MAX_CHARS_PER_MESSAGE)
        ));

        match &turn.assistant {
            Some(asst) => {
                let asst_char_count = asst.content.chars().count();

                let tool_calls_list: Vec<&serde_json::Value> = asst
                    .tool_calls
                    .as_ref()
                    .and_then(|tu| tu.get("toolCalls"))
                    .and_then(|tc| tc.as_array())
                    .map(|arr| arr.iter().collect())
                    .unwrap_or_default();

                let tool_call_count = tool_calls_list.len();

                let header = if tool_call_count > 0 {
                    format!(
                        "[助手] ({asst_char_count} 字符, 含 {tool_call_count} 次工具调用):"
                    )
                } else {
                    format!("[助手] ({asst_char_count} 字符):")
                };
                parts.push(header);

                for (i, tc) in tool_calls_list.iter().enumerate() {
                    let tool_name = tc
                        .get("toolName")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let duration_ms = tc
                        .get("duration_ms")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let result_preview = tc
                        .get("result_preview")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    parts.push(format!(
                        "[工具调用 {}: {}] 耗时 {}ms\n  结果预览: {}",
                        i + 1,
                        tool_name,
                        duration_ms,
                        truncate_to_chars(result_preview, PREVIEW_CHARS_FOR_TOOL_RESULT)
                    ));
                }

                parts.push(truncate_to_chars(&asst.content, MAX_CHARS_PER_MESSAGE));
            }
            None => {
                parts.push("[待回复]".to_string());
            }
        }

        output_parts.push(parts.join("\n"));
    }

    output_parts.join("\n\n")
}
