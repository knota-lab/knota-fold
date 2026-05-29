use std::fmt;
use std::sync::Arc;

use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use rig::completion::ToolDefinition;
use rig::tool::Tool;

use crate::modules::knowledge_base::service::chat_service;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ListConversationHistoryError(pub String);

impl fmt::Display for ListConversationHistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "list conversation history error: {}", self.0)
    }
}

impl std::error::Error for ListConversationHistoryError {}

// ---------------------------------------------------------------------------
// Args (empty — no parameters needed)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ListConversationHistoryArgs {}

// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ListConversationHistoryTool {
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

impl Tool for ListConversationHistoryTool {
    const NAME: &'static str = "list_conversation_history";

    type Error = ListConversationHistoryError;
    type Args = ListConversationHistoryArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "浏览当前会话的对话历史概览。返回每轮对话的摘要（角色、内容前 100 字符、字符数、是否含工具调用），帮助你快速定位想回顾的轮次。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    #[tracing::instrument(skip(self, _args))]
    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let messages = chat_service::get_session_messages(
            &self.db,
            self.session_id,
            self.tenant_id,
            self.user_id,
        )
        .await
        .map_err(|e| ListConversationHistoryError(e.to_string()))?;

        if messages.is_empty() {
            return Ok("当前会话暂无对话历史。".to_string());
        }

        // Pair messages into turns: each user message starts a new turn,
        // the following assistant message (if any) completes it.
        let mut turns: Vec<(&str, Option<&str>, Option<&serde_json::Value>)> = Vec::new();

        let mut i = 0;
        while i < messages.len() {
            let msg = &messages[i];
            if msg.role == "user" {
                let user_content = &msg.content;
                let mut assistant_content: Option<&str> = None;
                let mut tool_usage: Option<&serde_json::Value> = None;

                // Check if the next message is an assistant reply
                if i + 1 < messages.len() && messages[i + 1].role == "assistant" {
                    assistant_content = Some(&messages[i + 1].content);
                    tool_usage = messages[i + 1].token_usage.as_ref();
                    i += 2;
                } else {
                    i += 1;
                }

                turns.push((user_content, assistant_content, tool_usage));
            } else {
                // Skip orphan assistant messages (shouldn't happen, but be safe)
                i += 1;
            }
        }

        if turns.is_empty() {
            return Ok("当前会话暂无对话历史。".to_string());
        }

        let mut lines = Vec::with_capacity(turns.len() * 6 + 2);
        lines.push(format!("当前会话共 {} 轮对话：", turns.len()));
        lines.push(String::new()); // blank line after header

        for (idx, (user_content, assistant_content, tool_usage)) in
            turns.iter().enumerate()
        {
            if idx > 0 {
                lines.push(String::new()); // blank line between turns
            }

            lines.push(format!("第 {} 轮", idx + 1));

            // User message
            let user_char_count = user_content.chars().count();
            lines.push(format!("  [用户] ({} 字符)", user_char_count));
            let user_preview = truncate_preview(user_content, 100);
            lines.push(format!("    {}", user_preview));

            // Assistant message
            if let Some(asst_content) = assistant_content {
                let asst_char_count = asst_content.chars().count();
                let tool_count = count_tool_calls(*tool_usage);
                if tool_count > 0 {
                    lines.push(format!(
                        "  [助手] ({} 字符, 含 {} 次工具调用)",
                        asst_char_count, tool_count
                    ));
                } else {
                    lines.push(format!("  [助手] ({} 字符)", asst_char_count));
                }
                let asst_preview = truncate_preview(asst_content, 100);
                lines.push(format!("    {}", asst_preview));
            } else {
                lines.push("  [待回复]".to_string());
            }
        }

        Ok(lines.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Truncate content to `max_chars` characters, appending `…` if truncated.
fn truncate_preview(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        content.to_string()
    } else {
        let truncated: String = content.chars().take(max_chars).collect();
        format!("{}…", truncated)
    }
}

/// Count the number of tool calls in a `token_usage` JSON value.
/// Expects `token_usage` to contain a `"tool_calls"` array.
fn count_tool_calls(token_usage: Option<&serde_json::Value>) -> usize {
    token_usage
        .and_then(|v| v.get("toolCalls"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0)
}
