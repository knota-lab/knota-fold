use std::fmt::Write;

use crate::models::_entities::chat_messages;
use crate::modules::knowledge_base::service::qa_types::QaRequest;
use crate::modules::knowledge_base::service::tools::{
    FrontendToolStub, MaterialRegistry,
};

use super::QaStreamCtx;

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

/// Estimate token count for text.
/// JSON uses chars/1.2 (structural tokens are expensive);
/// Natural language uses chars/2 (conservative for Chinese-heavy content).
pub(super) fn estimate_text_tokens(text: &str, is_json: bool) -> usize {
    let char_count = text.chars().count();
    if is_json {
        char_count.saturating_mul(5).div_ceil(6)
    } else {
        (char_count / 2).max(1)
    }
}

const TOOL_OVERHEAD_TOKENS: usize = 10000;

/// Estimate token count for a message, including content + tool call records.
/// JSON content (tool arguments) uses chars/1.2; natural language uses chars/2.
pub fn estimate_message_tokens(msg: &chat_messages::Model) -> usize {
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
                                        std::string::ToString::to_string,
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
pub(super) fn build_chat_history_with_budget(
    history: &[chat_messages::Model],
    max_context_tokens: i32,
    response_reserve_tokens: i32,
    system_prompt_tokens: usize,
) -> Vec<rig::completion::message::Message> {
    let budget = usize::try_from(max_context_tokens)
        .unwrap_or_default()
        .saturating_sub(system_prompt_tokens)
        .saturating_sub(usize::try_from(response_reserve_tokens).unwrap_or_default())
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

pub(super) fn build_system_prompt(
    request: &QaRequest,
    registry: &MaterialRegistry,
    summary: &str,
    relevant_context: Option<&str>,
) -> String {
    let material_hint = build_material_hint(registry);
    let mut system_prompt = format!("{BASE_SYSTEM_PROMPT}{material_hint}\n\n");
    if !summary.is_empty() {
        let _ = write!(system_prompt, "\n\n[对话历史摘要]\n{summary}\n");
    }
    append_page_context_prompt(&mut system_prompt, request);
    if let Some(ctx) = relevant_context {
        system_prompt.push_str(ctx);
    }
    system_prompt
}

fn build_material_hint(registry: &MaterialRegistry) -> String {
    let mats = registry.all_materials();
    if mats.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n## 当前会话材料状态\n当前会话中有 **{}** 份可用材料。请始终先调用 `list_materials` 查看详情，再根据需要调用 `read_material` 读取内容。",
            mats.len()
        )
    }
}

fn append_page_context_prompt(system_prompt: &mut String, request: &QaRequest) {
    if request.page_context.is_empty() {
        return;
    }
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

    let _ = write!(system_prompt, "\n\n## 已注册页面上下文\n\
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
        active_title, active_route, page_list.join("\n")
    );
}

pub(super) fn build_user_prompt(
    request: &QaRequest,
    current_turn_materials: &[String],
) -> String {
    if current_turn_materials.is_empty() {
        request.instruction.clone()
    } else {
        format!(
            "[本轮新提交材料: {}]\n\n{}",
            current_turn_materials.join(", "),
            request.instruction
        )
    }
}

pub(super) fn build_page_tool_stubs(ctx: &QaStreamCtx<'_>) -> Vec<FrontendToolStub> {
    ctx.request
        .page_tools
        .iter()
        .map(|def| FrontendToolStub {
            name: def.name.clone(),
            description: def.description.clone(),
            parameters: def.parameters.clone(),
            broker: ctx.broker.clone(),
            sse_tx: ctx.tx.clone(),
        })
        .collect()
}
