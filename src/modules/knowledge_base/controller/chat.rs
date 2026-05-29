use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use axum::body::Body;
use axum::http::{header, HeaderMap};

use crate::extractors::TenantContext;
use crate::initializers::knowledge_base::SharedMemoryStore;
use crate::models::_entities::{chat_messages, chat_sessions};
use crate::modules::knowledge_base::service;
use crate::utils::error::{IntoLocoResult, IntoModelResult};
use crate::views::errors::parse_uuid;

/// Create a new chat session.
#[utoipa::path(
    post,
    path = "/api/chat/sessions",
    tag = "聊天",
    description = "创建聊天会话",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn create_session(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let session = service::create_session(&ctx.db, tc.tenant_id, tc.user_id, None)
        .await
        .model_err()?;

    format::json(serde_json::json!({
        "id": session.id.to_string(),
        "title": session.title,
        "createdAt": session.created_at.and_utc().to_rfc3339(),
        "updatedAt": session.updated_at.and_utc().to_rfc3339(),
    }))
}

/// List chat sessions for current user.
#[utoipa::path(
    get,
    path = "/api/chat/sessions",
    tag = "聊天",
    description = "列出聊天会话",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_sessions(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let sessions = service::list_sessions(&ctx.db, tc.tenant_id, tc.user_id, 50)
        .await
        .model_err()?;

    let items: Vec<serde_json::Value> = sessions
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id.to_string(),
                "title": s.title,
                "createdAt": s.created_at.and_utc().to_rfc3339(),
                "updatedAt": s.updated_at.and_utc().to_rfc3339(),
            })
        })
        .collect();

    format::json(items)
}

/// Get session details with messages.
#[utoipa::path(
    get,
    path = "/api/chat/sessions/{id}",
    tag = "聊天",
    description = "获取会话详情及消息",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_session(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let session_id = parse_uuid(id)?;

    let session = service::get_session(&ctx.db, session_id, tc.tenant_id, tc.user_id)
        .await
        .model_err()?;

    let messages =
        service::get_session_messages(&ctx.db, session_id, tc.tenant_id, tc.user_id)
            .await
            .model_err()?;

    let msg_values: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id.to_string(),
                "role": m.role,
                "content": m.content,
                "materialRefs": m.material_refs,
                "intent": m.intent,
                "strategy": m.strategy,
                "tokenUsage": m.token_usage,
                "createdAt": m.created_at.and_utc().to_rfc3339(),
            })
        })
        .collect();

    format::json(serde_json::json!({
        "id": session.id.to_string(),
        "title": session.title,
        "messages": msg_values,
        "createdAt": session.created_at.and_utc().to_rfc3339(),
        "updatedAt": session.updated_at.and_utc().to_rfc3339(),
    }))
}

/// Delete a chat session.
#[utoipa::path(
    delete,
    path = "/api/chat/sessions/{id}",
    tag = "聊天",
    description = "删除聊天会话",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn delete_session(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let session_id = parse_uuid(id)?;

    let memory_store = ctx.shared_store.get::<SharedMemoryStore>().ok_or_else(|| {
        crate::views::errors::err_internal(
            "knowledge_base.memory_store_not_initialized",
            "Memory store not initialized — is knowledge_base enabled?",
        )
    })?;

    service::delete_session(&ctx.db, &memory_store, session_id, tc.tenant_id, tc.user_id)
        .await
        .model_err()?;

    format::json(serde_json::json!({"success": true}))
}

/// Export a chat session as Markdown.
#[utoipa::path(
    get,
    path = "/api/chat/sessions/{id}/export",
    tag = "聊天",
    description = "导出对话为 Markdown",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn export_session(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<(HeaderMap, Body)> {
    let session_id = parse_uuid(id)?;

    let session = service::get_session(&ctx.db, session_id, tc.tenant_id, tc.user_id)
        .await
        .model_err()?;

    let messages =
        service::get_session_messages(&ctx.db, session_id, tc.tenant_id, tc.user_id)
            .await
            .model_err()?;

    let markdown = format_session_markdown(&session, &messages);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/markdown; charset=utf-8"
            .parse::<axum::http::HeaderValue>()
            .loco_err()?,
    );
    let filename = format!("chat-{}.md", session.created_at.format("%Y%m%d-%H%M%S"));
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"")
            .parse::<axum::http::HeaderValue>()
            .loco_err()?,
    );

    Ok((headers, Body::from(markdown)))
}

fn format_session_markdown(
    session: &chat_sessions::Model,
    messages: &[chat_messages::Model],
) -> String {
    let title = session.title.as_deref().unwrap_or("未命名会话");
    let timestamp = chrono::Utc::now().naive_utc().format("%Y-%m-%d %H:%M:%S");

    let mut md = format!(
        "# {title}\n\n\
         > 导出时间: {timestamp}\n\
         > 会话 ID: {id}\n\n\
         ---\n\n",
        title = title,
        timestamp = timestamp,
        id = session.id,
    );

    let mut round = 0u32;
    for msg in messages {
        match msg.role.as_str() {
            "user" => {
                round += 1;
                md.push_str(&format!("## 用户\n\n{}\n", msg.content));

                if let Some(ref refs) = msg.material_refs {
                    let mut material_lines = Vec::new();

                    // Inline text with full content
                    if let Some(inline_obj) =
                        refs.get("inline").and_then(|v| v.as_object())
                    {
                        let name = inline_obj
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("粘贴文本");
                        if let Some(content) =
                            inline_obj.get("content").and_then(|v| v.as_str())
                        {
                            material_lines.push(format!(
                                "- {}（{}字）:\n{}\n",
                                name,
                                content.chars().count(),
                                content
                                    .lines()
                                    .map(|l| format!("  {l}"))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            ));
                        } else {
                            material_lines.push(format!("- {name}"));
                        }
                    }
                    if refs.get("documentIds").is_some() {
                        material_lines.push("- 文档（详见上方消息内容）".to_string());
                    }
                    if refs.get("fileIds").is_some() {
                        material_lines.push("- 文件".to_string());
                    }
                    if !material_lines.is_empty() {
                        md.push_str(&format!(
                            "\n**附加材料:**\n{}\n",
                            material_lines.join("\n")
                        ));
                    }
                }

                md.push_str("\n---\n\n");
            }
            "assistant" => {
                let is_error = msg.content.starts_with("\u{26a0}\u{fe0f}")
                    || msg.content.starts_with('\u{26a0}');
                md.push_str("## 助手\n\n");

                // Try content_parts first (ordered interleaving of text/tool_call)
                let has_content_parts = msg.token_usage.as_ref().and_then(|tu| {
                    tu.get("contentParts")?.as_array().filter(|a| !a.is_empty())
                });

                if let Some(parts) = has_content_parts {
                    for part in parts {
                        let Some(part_type) = part.get("type").and_then(|v| v.as_str())
                        else {
                            continue;
                        };
                        match part_type {
                            "text" => {
                                if let Some(content) =
                                    part.get("content").and_then(|v| v.as_str())
                                {
                                    if !content.is_empty() {
                                        if is_error {
                                            for line in content.lines() {
                                                md.push_str(&format!(
                                                    "> \u{26a0}\u{fe0f} {line}\n"
                                                ));
                                            }
                                        } else {
                                            md.push_str(content);
                                            md.push('\n');
                                        }
                                    }
                                }
                            }
                            "tool_call" => {
                                let name = part
                                    .get("toolName")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                let duration = part
                                    .get("durationMs")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let preview = part
                                    .get("resultPreview")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let time_str = if duration < 1000 {
                                    format!("{duration}ms")
                                } else {
                                    format!("{:.1}s", duration as f64 / 1000.0)
                                };
                                let truncated_preview: String =
                                    preview.chars().take(500).collect();
                                let ellipsis = if preview.chars().count() > 500 {
                                    "…"
                                } else {
                                    ""
                                };
                                md.push_str(&format!(
                                    "\n> **🔍 {name}** ({time_str})\n> `{truncated_preview}{ellipsis}`"
                                ));
                                // 完整工具调用参数
                                if let Some(args) = part.get("arguments") {
                                    let args_str = serde_json::to_string_pretty(args)
                                        .unwrap_or_default();
                                    md.push_str(&format!(
                                        "\n> **参数:**\n> ```json\n{}\n> ```",
                                        args_str
                                            .lines()
                                            .map(|l| format!("> {l}"))
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    ));
                                }
                                md.push_str("\n\n");
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Fallback: legacy tool_calls at top + text below
                    let tool_section = msg.token_usage.as_ref().and_then(|tu| {
                        let calls = tu.get("toolCalls")?.as_array()?;
                        if calls.is_empty() {
                            return None;
                        }
                        let mut lines = vec!["**工具调用:**".to_string()];
                        for call in calls {
                            let name = call.get("toolName")?.as_str()?;
                            let duration = call.get("durationMs")?.as_u64()?;
                            let preview =
                                call.get("resultPreview")?.as_str().unwrap_or("");
                            let time_str = if duration < 1000 {
                                format!("{duration}ms")
                            } else {
                                format!("{:.1}s", duration as f64 / 1000.0)
                            };
                            let truncated_preview: String =
                                preview.chars().take(500).collect();
                            let ellipsis = if preview.chars().count() > 500 {
                                "…"
                            } else {
                                ""
                            };
                            let mut line = format!(
                                "- ✓ {name} ({time_str}) `{truncated_preview}{ellipsis}`"
                            );
                            // 完整工具调用参数
                            if let Some(args) = call.get("arguments") {
                                let args_str = serde_json::to_string_pretty(args)
                                    .unwrap_or_default();
                                line.push_str(&format!(
                                    "\n  参数:\n  ```json\n{}\n  ```",
                                    args_str
                                        .lines()
                                        .map(|l| format!("  {l}"))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                ));
                            }
                            lines.push(line);
                        }
                        Some(lines.join("\n"))
                    });
                    if let Some(ref tools) = tool_section {
                        md.push_str(tools);
                        md.push_str("\n\n");
                    }
                    if is_error {
                        for line in msg.content.lines() {
                            md.push_str(&format!("> \u{26a0}\u{fe0f} {line}\n"));
                        }
                    } else {
                        md.push_str(&format!("{}\n", msg.content));
                    }
                }

                // Token 用量统计
                if msg.total_tokens > 0 {
                    md.push_str(&format!(
                        "\n*Token 用量: prompt={}, completion={}, total={}\n",
                        msg.prompt_tokens, msg.completion_tokens, msg.total_tokens
                    ));
                }

                md.push_str("\n---\n\n");
            }
            _ => {}
        }
    }

    md.push_str(&format!("> 对话结束 — 共 {round} 轮\n"));
    md
}

/// Export a chat session as debug Markdown (full tool results, debug context, etc.).
#[utoipa::path(
    get,
    path = "/api/chat/sessions/{id}/debug-export",
    tag = "聊天",
    description = "导出对话调试信息为 Markdown",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn debug_export_session(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<(HeaderMap, Body)> {
    let session_id = parse_uuid(id)?;

    let session = service::get_session(&ctx.db, session_id, tc.tenant_id, tc.user_id)
        .await
        .model_err()?;

    let messages =
        service::get_session_messages(&ctx.db, session_id, tc.tenant_id, tc.user_id)
            .await
            .model_err()?;

    let html = format_debug_html(&session, &messages);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/html; charset=utf-8"
            .parse::<axum::http::HeaderValue>()
            .loco_err()?,
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"debug-{session_id}.html\"")
            .parse::<axum::http::HeaderValue>()
            .loco_err()?,
    );

    Ok((headers, Body::from(html)))
}

fn format_debug_html(
    session: &chat_sessions::Model,
    messages: &[chat_messages::Model],
) -> String {
    let title = session.title.as_deref().unwrap_or("未命名会话");
    let timestamp = chrono::Utc::now().naive_utc().format("%Y-%m-%d %H:%M:%S");

    let mut html = String::with_capacity(64 * 1024);

    // ---- HTML head + inline CSS ----
    html.push_str(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>调试导出: "#,
    );
    html.push_str(&html_escape(title));
    html.push_str(r#"</title>
<style>
  :root { --bg: #fff; --surface: #f6f8fa; --border: #d0d7de; --text: #1f2328;
          --text2: #656d76; --accent: #0969da; --error: #d1242f; --tool: #1a7f37; }
  body { font-family: -apple-system, "Segoe UI", Helvetica, Arial, sans-serif;
         max-width: 960px; margin: 0 auto; padding: 24px; color: var(--text);
         line-height: 1.6; background: var(--bg); }
  h1 { border-bottom: 2px solid var(--border); padding-bottom: 8px; font-size: 20px; }
  h2 { font-size: 16px; margin: 24px 0 12px; display: flex; align-items: center; gap: 8px; }
  .badge { font-size: 11px; padding: 2px 8px; border-radius: 10px; font-weight: 600;
           text-transform: uppercase; letter-spacing: .5px; }
  .badge-user { background: #ddf4ff; color: #0969da; }
  .badge-assistant { background: #dafbe1; color: #1a7f37; }
  .meta { color: var(--text2); font-size: 13px; margin-bottom: 8px; }
  .meta code { background: var(--surface); padding: 1px 6px; border-radius: 4px;
               font-size: 12px; }
  .warning { background: #fff8c5; border: 1px solid #e3b341; border-radius: 6px;
             padding: 8px 12px; font-size: 13px; margin: 12px 0; }
  .msg-card { border: 1px solid var(--border); border-radius: 8px; padding: 16px;
              margin-bottom: 16px; background: var(--bg); }
  .msg-card + .msg-card { margin-top: -8px; }
  .section-title { font-size: 13px; font-weight: 600; color: var(--text2);
                   margin: 16px 0 8px; text-transform: uppercase; letter-spacing: .5px; }
  details { margin: 4px 0; }
  summary { cursor: pointer; font-weight: 500; font-size: 13px; padding: 4px 0;
            color: var(--accent); list-style: none; }
  summary::before { content: "▸ "; }
  details[open] > summary::before { content: "▾ "; }
  details > div, details > pre { margin: 8px 0 8px 16px; }
  .kv-table { border-collapse: collapse; font-size: 13px; width: 100%; }
  .kv-table td { padding: 3px 12px 3px 0; border-bottom: 1px solid var(--surface); }
  .kv-table td:first-child { color: var(--text2); white-space: nowrap; font-weight: 500; }
  pre { background: var(--surface); border: 1px solid var(--border); border-radius: 6px;
        padding: 12px; font-size: 12px; line-height: 1.5; overflow-x: auto;
        white-space: pre-wrap; word-break: break-all; margin: 8px 0; }
  code { font-family: "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; }
  .json-key { color: #0550ae; }
  .json-str { color: #0a3069; }
  .json-num { color: #0550ae; }
  .json-bool { color: #cf222e; }
  .tool-call { border-left: 3px solid var(--tool); padding-left: 12px; margin: 8px 0; }
  .tool-name { font-weight: 600; color: var(--tool); font-size: 13px; }
  .tool-duration { color: var(--text2); font-size: 12px; margin-left: 8px; }
  .answer-block { background: var(--surface); border-radius: 6px; padding: 12px;
                  margin: 8px 0; white-space: pre-wrap; word-break: break-word;
                  font-size: 14px; line-height: 1.6; }
  .error-block { background: #ffebe9; border-left: 3px solid var(--error); }
  .round-sep { border: none; border-top: 1px solid var(--border); margin: 8px 0; }
  .timeline { margin: 12px 0; padding-left: 24px; border-left: 3px solid var(--border); }
  .timeline-entry { margin: 12px 0; position: relative; }
  .timeline-entry::before { content: ''; position: absolute; left: -30px; top: 6px;
    width: 12px; height: 12px; border-radius: 50%; background: var(--border); }
  .user-entry::before { background: #0969da; }
  .tool-entry::before { background: #bf8700; }
  .timeline-label { font-weight: 600; font-size: 13px; margin-bottom: 4px; }
  .timeline-content { background: var(--surface); border-radius: 6px;
    padding: 10px; font-size: 12px; line-height: 1.5; white-space: pre-wrap;
    word-break: break-all; margin: 0; max-height: 400px; overflow-y: auto; }
  .toc { position: fixed; top: 16px; left: 16px; width: 280px; max-height: calc(100vh - 32px);
    border: 1px solid var(--border); border-radius: 6px; background: #fff;
    box-shadow: 0 2px 8px rgba(0,0,0,0.1); z-index: 100; overflow: hidden; }
  .toc summary { padding: 10px 14px; font-weight: 600; font-size: 14px;
    cursor: pointer; background: var(--surface); }
  .toc nav { padding: 8px 10px; max-height: calc(100vh - 80px); overflow-y: auto; }
  .toc-link { display: block; padding: 5px 8px; color: #0969da; text-decoration: none;
    font-size: 12px; border-radius: 4px; line-height: 1.4; }
  .toc-link:hover { background: #f0f4f8; }
  .toc-round { font-weight: 600; margin-right: 4px; color: var(--text2);
    font-size: 11px; white-space: nowrap; }
</style>
</head>
<body>
"#);

    // ---- Header ----
    html.push_str(&format!("<h1>调试导出: {}</h1>\n", html_escape(title)));
    html.push_str(&format!(
        "<div class=\"meta\">导出时间: {} &nbsp;|&nbsp; 会话 ID: <code>{}</code></div>\n",
        timestamp, session.id
    ));
    html.push_str(
        "<div class=\"warning\">⚠️ 本文档包含 LLM 调试数据，仅供开发者参考</div>\n",
    );

    // ---- TOC (目录) ----
    // Build a quick index of user messages for navigation
    {
        let mut toc_entries: Vec<(u32, String)> = Vec::new(); // (round, preview)
        let mut toc_round = 0u32;
        for msg in messages {
            if msg.role == "user" {
                toc_round += 1;
                let preview: String = msg.content.chars().take(80).collect();
                let ellipsis = if msg.content.chars().count() > 80 {
                    "…"
                } else {
                    ""
                };
                toc_entries.push((toc_round, format!("{preview}{ellipsis}")));
            }
        }
        if !toc_entries.is_empty() {
            html.push_str(
                "<details class=\"toc\" open><summary>📋 对话目录</summary>\n<nav>\n",
            );
            for (r, preview) in &toc_entries {
                html.push_str(&format!(
                    "<a class=\"toc-link\" href=\"#round-{}\"><span class=\"toc-round\">第 {} 轮</span> {}</a>\n",
                    r, r, html_escape(preview)
                ));
            }
            html.push_str("</nav>\n</details>\n");
        }
    }

    // ---- Messages ----
    let mut round = 0u32;
    for (idx, msg) in messages.iter().enumerate() {
        let msg_num = idx + 1;
        match msg.role.as_str() {
            "user" => {
                round += 1;
                html.push_str("<div class=\"msg-card\">\n");
                html.push_str(&format!(
                    "<h2 id=\"round-{round}\"><span class=\"badge badge-user\">User</span> 消息 #{msg_num}</h2>\n"
                ));
                html.push_str(&format!(
                    "<div class=\"answer-block\">{}</div>\n",
                    html_escape(&msg.content)
                ));
                if let Some(ref refs) = msg.material_refs {
                    let refs_str = serde_json::to_string_pretty(refs).unwrap_or_default();
                    html.push_str("<div class=\"section-title\">附加材料</div>\n");
                    html.push_str(&format!(
                        "<details><summary>material_refs JSON</summary><pre><code>{}</code></pre></details>\n",
                        html_escape(&refs_str)
                    ));
                }
                html.push_str("</div>\n");
            }
            "assistant" => {
                html.push_str("<div class=\"msg-card\">\n");
                html.push_str(&format!(
                    "<h2><span class=\"badge badge-assistant\">Assistant</span> 消息 #{msg_num}</h2>\n"
                ));

                // Token usage
                if msg.total_tokens > 0 {
                    html.push_str(&format!(
                        "<div class=\"meta\">Token: prompt=<code>{}</code> completion=<code>{}</code> total=<code>{}</code></div>\n",
                        msg.prompt_tokens, msg.completion_tokens, msg.total_tokens
                    ));
                }

                // Debug context
                let tu = msg.token_usage.as_ref();
                let debug_ctx = tu.and_then(|v| v.get("debugContext"));

                if let Some(dc) = debug_ctx {
                    html.push_str("<div class=\"section-title\">调试上下文</div>\n");

                    // System Prompt
                    if let Some(sp) = dc.get("system_prompt").and_then(|v| v.as_str()) {
                        html.push_str(&format!(
                            "<details><summary>System Prompt ({} 字符)</summary><pre>{}</pre></details>\n",
                            sp.chars().count(),
                            html_escape(sp)
                        ));
                    }

                    // User Prompt
                    if let Some(up) = dc.get("user_prompt").and_then(|v| v.as_str()) {
                        html.push_str(&format!(
                            "<details><summary>User Prompt ({} 字符)</summary><pre>{}</pre></details>\n",
                            up.chars().count(),
                            html_escape(up)
                        ));
                    }

                    // Config Snapshot
                    if let Some(cs) = dc.get("config_snapshot") {
                        let cs_str = serde_json::to_string_pretty(cs).unwrap_or_default();
                        html.push_str(&format!(
                            "<details><summary>配置快照</summary><pre><code>{}</code></pre></details>\n",
                            html_escape(&cs_str)
                        ));
                    }

                    // Compaction
                    if let Some(comp) = dc.get("compaction") {
                        let kv = |key: &str| -> String {
                            comp.get(key).map_or_else(
                                || "—".to_string(),
                                |v| match v {
                                    serde_json::Value::Bool(b) => b.to_string(),
                                    serde_json::Value::Number(n) => n.to_string(),
                                    serde_json::Value::String(s) => s.clone(),
                                    other => other.to_string(),
                                },
                            )
                        };
                        html.push_str("<details><summary>Compaction 状态</summary>\n<table class=\"kv-table\">");
                        for (k, label) in [
                            ("triggered", "触发"),
                            ("summary_length", "摘要长度"),
                            ("history_total", "历史消息总数"),
                            ("history_tokens", "历史估算 tokens"),
                            ("recent_start", "recent_start"),
                        ] {
                            html.push_str(&format!(
                                "<tr><td>{}</td><td>{}</td></tr>",
                                label,
                                kv(k)
                            ));
                        }
                        html.push_str("</table></details>\n");
                    }

                    // Semantic Recall
                    if let Some(sr) = dc.get("semantic_recall") {
                        let strategy =
                            sr.get("strategy").and_then(|v| v.as_str()).unwrap_or("—");
                        let ctx_len = sr
                            .get("context_length")
                            .and_then(|v| v.as_u64())
                            .map_or_else(|| "—".to_string(), |v| v.to_string());
                        let has_recall = sr
                            .get("has_recall")
                            .and_then(|v| v.as_bool())
                            .map_or_else(|| "—".to_string(), |b| b.to_string());
                        html.push_str("<details><summary>语义检索</summary>\n<table class=\"kv-table\">");
                        html.push_str(&format!(
                            "<tr><td>策略</td><td>{strategy}</td></tr>"
                        ));
                        html.push_str(&format!(
                            "<tr><td>上下文长度</td><td>{ctx_len}</td></tr>"
                        ));
                        html.push_str(&format!(
                            "<tr><td>有召回</td><td>{has_recall}</td></tr>"
                        ));
                        html.push_str("</table></details>\n");
                    }

                    // Chat History Summary
                    if let Some(chs) = dc.get("chat_history_summary") {
                        let mc = chs
                            .get("message_count")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        html.push_str(&format!(
                            "<details><summary>Chat History ({mc} 条消息)</summary>\n<table class=\"kv-table\"><tr><td>#</td><td>角色</td><td>内容/文本数</td><td>工具调用</td></tr>"
                        ));
                        if let Some(msgs) = chs.get("messages").and_then(|v| v.as_array())
                        {
                            for (i, entry) in msgs.iter().enumerate() {
                                let role = entry
                                    .get("role")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("—");
                                let content_len = entry
                                    .get("content_count")
                                    .or_else(|| entry.get("text_items"))
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let has_tools = entry
                                    .get("has_tool_calls")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                html.push_str(&format!(
                                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                                    i + 1, role, content_len, if has_tools { "✓" } else { "—" }
                                ));
                            }
                        }
                        html.push_str("</table></details>\n");
                    }

                    // Tool Rounds — 完整对话脉络
                    if let Some(rounds) = dc.get("toolRounds").and_then(|v| v.as_array())
                    {
                        if !rounds.is_empty() {
                            html.push_str("<div class=\"section-title\">对话脉络（LLM 完整交互时间线）</div>\n");
                            html.push_str("<div class=\"timeline\">\n");

                            // Step 1: User Prompt entry
                            if let Some(up) =
                                dc.get("user_prompt").and_then(|v| v.as_str())
                            {
                                html.push_str(
                                    "<div class=\"timeline-entry user-entry\">\n",
                                );
                                html.push_str(
                                    "<div class=\"timeline-label\">📥 用户输入</div>\n",
                                );
                                html.push_str(&format!(
                                    "<pre class=\"timeline-content\">{}</pre>\n",
                                    html_escape(up)
                                ));
                                html.push_str("</div>\n");
                            }

                            // Step 2: Each tool round
                            for round in rounds {
                                let round_num = round
                                    .get("round")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let tool_name = round
                                    .get("toolName")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                let duration_ms = round
                                    .get("durationMs")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let result_full = round
                                    .get("resultFull")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let args = round
                                    .get("arguments")
                                    .map(|v| {
                                        serde_json::to_string_pretty(v)
                                            .unwrap_or_default()
                                    })
                                    .unwrap_or_default();

                                html.push_str(
                                    "<div class=\"timeline-entry tool-entry\">\n",
                                );
                                html.push_str(&format!(
                                    "<div class=\"timeline-label\">🔧 工具调用 #{round_num} — {tool_name} ({duration_ms}ms)</div>\n"
                                ));

                                // Arguments
                                if !args.is_empty() {
                                    html.push_str(
                                        "<details><summary>参数</summary><pre><code>",
                                    );
                                    html.push_str(&html_escape(&args));
                                    html.push_str("</code></pre></details>\n");
                                }

                                // Result
                                html.push_str("<details><summary>完整返回结果");
                                if !result_full.is_empty() {
                                    html.push_str(&format!(
                                        " ({} 字符)",
                                        result_full.chars().count()
                                    ));
                                }
                                html.push_str("</summary><pre>");
                                html.push_str(&html_escape(result_full));
                                html.push_str("</pre></details>\n");

                                html.push_str("</div>\n");
                            }

                            html.push_str("</div>\n");
                        }
                    }
                } else {
                    html.push_str("<div class=\"meta\">无调试数据</div>\n");
                }

                // Tool call records
                let has_content_parts = tu.and_then(|v| {
                    v.get("content_parts")?.as_array().filter(|a| !a.is_empty())
                });
                let has_tool_calls = has_content_parts.is_some()
                    || tu
                        .and_then(|v| {
                            v.get("toolCalls")?.as_array().filter(|a| !a.is_empty())
                        })
                        .is_some();

                if has_tool_calls {
                    html.push_str("<div class=\"section-title\">工具调用记录</div>\n");
                    let parts_iter: Box<dyn Iterator<Item = &serde_json::Value>> =
                        if let Some(parts) = has_content_parts {
                            Box::new(parts.iter().filter(|p| {
                                p.get("type").and_then(|v| v.as_str())
                                    == Some("tool_call")
                            }))
                        } else if let Some(calls) =
                            tu.and_then(|v| v.get("toolCalls")?.as_array())
                        {
                            Box::new(calls.iter())
                        } else {
                            Box::new(std::iter::empty())
                        };

                    for part in parts_iter {
                        let name = part
                            .get("toolName")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let duration =
                            part.get("durationMs").and_then(|v| v.as_u64()).unwrap_or(0);
                        let time_str = if duration < 1000 {
                            format!("{duration}ms")
                        } else {
                            format!("{:.1}s", duration as f64 / 1000.0)
                        };

                        html.push_str("<div class=\"tool-call\">");
                        html.push_str(&format!("<span class=\"tool-name\">🔍 {}</span><span class=\"tool-duration\">{}</span>\n", html_escape(name), time_str));

                        // Arguments
                        if let Some(args) = part.get("arguments") {
                            let args_str =
                                serde_json::to_string_pretty(args).unwrap_or_default();
                            html.push_str(&format!(
                                "<details><summary>参数</summary><pre><code>{}</code></pre></details>\n",
                                html_escape(&args_str)
                            ));
                        }

                        // Result
                        let result_text = part
                            .get("resultFull")
                            .and_then(|v| v.as_str())
                            .or_else(|| {
                                part.get("resultPreview").and_then(|v| v.as_str())
                            })
                            .unwrap_or("");
                        html.push_str(&format!(
                            "<details><summary>完整结果 ({} 字符)</summary><pre>{}</pre></details>\n",
                            result_text.chars().count(),
                            html_escape(result_text)
                        ));
                        html.push_str("</div>\n");
                    }
                }

                // Answer content
                let is_error = msg.content.starts_with('\u{26a0}');
                html.push_str("<div class=\"section-title\">回答内容</div>\n");
                if !msg.content.is_empty() {
                    html.push_str(&format!(
                        "<div class=\"answer-block{}\">{}</div>\n",
                        if is_error { " error-block" } else { "" },
                        html_escape(&msg.content)
                    ));
                }

                html.push_str("</div>\n");
            }
            _ => {}
        }
    }

    html.push_str(&format!(
        "<hr class=\"round-sep\"><div class=\"meta\">对话结束 — 共 {round} 轮</div>\n"
    ));
    html.push_str("</body>\n</html>\n");
    html
}

/// Escape HTML special characters to prevent XSS in debug output.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
