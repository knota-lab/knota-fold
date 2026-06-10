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
    use std::fmt::Write;
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
                write_markdown_user_message(&mut md, msg);
            }
            "assistant" => {
                write_markdown_assistant_message(&mut md, msg);
            }
            _ => {}
        }
    }

    let _ = writeln!(md, "> 对话结束 — 共 {round} 轮");
    md
}

fn write_markdown_user_message(md: &mut String, msg: &chat_messages::Model) {
    use std::fmt::Write;

    let _ = writeln!(md, "## 用户\n\n{}", msg.content);
    if let Some(ref refs) = msg.material_refs {
        let material_lines = markdown_material_lines(refs);
        if !material_lines.is_empty() {
            let _ = writeln!(md, "\n**附加材料:**\n{}", material_lines.join("\n"));
        }
    }
    md.push_str("\n---\n\n");
}

fn markdown_material_lines(refs: &serde_json::Value) -> Vec<String> {
    let mut material_lines = Vec::new();
    if let Some(inline_obj) = refs.get("inline").and_then(|v| v.as_object()) {
        let name = inline_obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("粘贴文本");
        if let Some(content) = inline_obj.get("content").and_then(|v| v.as_str()) {
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
    if let Some(library_id) = refs.get("libraryId").and_then(|v| v.as_str()) {
        material_lines.push(format!("- 知识库范围: {library_id}"));
    }
    if let Some(folder_id) = refs.get("folderId").and_then(|v| v.as_str()) {
        let scope_suffix = if refs
            .get("includeSubfolders")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            "（含子目录）"
        } else {
            "（仅当前目录）"
        };
        material_lines.push(format!("- 目录范围: {folder_id}{scope_suffix}"));
    }
    if refs.get("fileIds").is_some() {
        material_lines.push("- 文件".to_string());
    }
    material_lines
}

fn write_markdown_assistant_message(md: &mut String, msg: &chat_messages::Model) {
    use std::fmt::Write;

    let is_error = msg.content.starts_with("\u{26a0}\u{fe0f}")
        || msg.content.starts_with('\u{26a0}');
    md.push_str("## 助手\n\n");

    let has_content_parts = msg
        .token_usage
        .as_ref()
        .and_then(|tu| tu.get("contentParts")?.as_array().filter(|a| !a.is_empty()));
    if let Some(parts) = has_content_parts {
        write_markdown_content_parts(md, parts, is_error);
    } else {
        write_markdown_legacy_assistant(md, msg, is_error);
    }
    write_markdown_citations(md, msg.token_usage.as_ref());

    if msg.total_tokens > 0 {
        let _ = writeln!(
            md,
            "\n*Token 用量: prompt={}, completion={}, total={}",
            msg.prompt_tokens, msg.completion_tokens, msg.total_tokens
        );
    }

    md.push_str("\n---\n\n");
}

fn write_markdown_citations(md: &mut String, token_usage: Option<&serde_json::Value>) {
    use std::fmt::Write;

    let Some(citations) = token_usage
        .and_then(|tu| tu.get("citations"))
        .and_then(serde_json::Value::as_array)
        .filter(|items| !items.is_empty())
    else {
        return;
    };

    md.push_str("\n**引用来源:**\n");
    for citation in citations {
        let title = citation_title(citation);
        let heading = citation
            .get("headingPath")
            .and_then(serde_json::Value::as_str);
        let document_id = citation
            .get("documentId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let chunk_suffix = citation
            .get("chunkId")
            .and_then(serde_json::Value::as_str)
            .map_or_else(String::new, |id| format!(", 分块: {}", short_id(id)));
        let score = citation
            .get("score")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default();
        let location = citation_location(citation);

        let _ = writeln!(
            md,
            "- {title}（{location}, 文档: {document}{chunk_suffix}, 相关度: {score:.2}）",
            document = short_id(document_id),
        );
        if heading.is_some() && citation.get("documentTitle").is_some() {
            let _ = writeln!(md, "  - 章节: {}", heading.unwrap_or_default());
        }
    }
}

fn write_markdown_content_parts(
    md: &mut String,
    parts: &[serde_json::Value],
    is_error: bool,
) {
    for part in parts {
        let Some(part_type) = part.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        match part_type {
            "text" => write_markdown_text_part(md, part, is_error),
            "tool_call" => write_markdown_tool_call(md, part, true),
            _ => {}
        }
    }
}

fn write_markdown_text_part(md: &mut String, part: &serde_json::Value, is_error: bool) {
    use std::fmt::Write;

    let Some(content) = part.get("content").and_then(|v| v.as_str()) else {
        return;
    };
    if content.is_empty() {
        return;
    }
    if is_error {
        for line in content.lines() {
            let _ = writeln!(md, "> \u{26a0}\u{fe0f} {line}");
        }
    } else {
        md.push_str(content);
        md.push('\n');
    }
}

fn write_markdown_legacy_assistant(
    md: &mut String,
    msg: &chat_messages::Model,
    is_error: bool,
) {
    use std::fmt::Write;

    if let Some(tool_section) = markdown_legacy_tool_section(msg) {
        md.push_str(&tool_section);
        md.push_str("\n\n");
    }
    if is_error {
        for line in msg.content.lines() {
            let _ = writeln!(md, "> \u{26a0}\u{fe0f} {line}");
        }
    } else {
        let _ = writeln!(md, "{}", msg.content);
    }
}

fn markdown_legacy_tool_section(msg: &chat_messages::Model) -> Option<String> {
    let calls = msg.token_usage.as_ref()?.get("toolCalls")?.as_array()?;
    if calls.is_empty() {
        return None;
    }

    let mut lines = vec!["**工具调用:**".to_string()];
    for call in calls {
        let mut line = markdown_tool_call_summary(call, false)?;
        if let Some(args) = markdown_tool_arguments(call, "  ") {
            line.push_str(&args);
        }
        lines.push(line);
    }
    Some(lines.join("\n"))
}

fn markdown_tool_call_summary(call: &serde_json::Value, quoted: bool) -> Option<String> {
    let name = call.get("toolName")?.as_str()?;
    let duration = call.get("durationMs")?.as_u64()?;
    let preview = call.get("resultPreview")?.as_str().unwrap_or("");
    let time_str = format_duration(duration);
    let truncated_preview: String = preview.chars().take(500).collect();
    let ellipsis = if preview.chars().count() > 500 {
        "…"
    } else {
        ""
    };

    if quoted {
        Some(format!(
            "\n> **🔍 {name}** ({time_str})\n> `{truncated_preview}{ellipsis}`"
        ))
    } else {
        Some(format!(
            "- ✓ {name} ({time_str}) `{truncated_preview}{ellipsis}`"
        ))
    }
}

fn write_markdown_tool_call(md: &mut String, part: &serde_json::Value, quoted: bool) {
    if let Some(summary) = markdown_tool_call_summary(part, quoted) {
        md.push_str(&summary);
    }
    if let Some(args) = markdown_tool_arguments(part, if quoted { "> " } else { "  " }) {
        md.push_str(&args);
    }
    md.push_str("\n\n");
}

fn markdown_tool_arguments(call: &serde_json::Value, prefix: &str) -> Option<String> {
    let args = call.get("arguments")?;
    let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
    let body = args_str
        .lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    if prefix == "> " {
        Some(format!("\n> **参数:**\n> ```json\n{body}\n> ```"))
    } else {
        Some(format!("\n  参数:\n  ```json\n{body}\n  ```"))
    }
}

fn format_duration(duration_ms: u64) -> String {
    if duration_ms < 1000 {
        format!("{duration_ms}ms")
    } else {
        format!("{}.{:01}s", duration_ms / 1000, (duration_ms % 1000) / 100)
    }
}

fn citation_title(citation: &serde_json::Value) -> String {
    citation
        .get("documentTitle")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            citation
                .get("headingPath")
                .and_then(serde_json::Value::as_str)
        })
        .map_or_else(|| "未知文档".to_string(), ToString::to_string)
}

fn citation_location(citation: &serde_json::Value) -> String {
    let start = citation
        .get("startLine")
        .and_then(serde_json::Value::as_i64);
    let end = citation.get("endLine").and_then(serde_json::Value::as_i64);
    match (start, end) {
        (Some(start), Some(end)) if start != end => format!("第 {start}-{end} 行"),
        (Some(start), _) => format!("第 {start} 行"),
        _ => "位置未知".to_string(),
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
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
    use std::fmt::Write;
    let title = session.title.as_deref().unwrap_or("未命名会话");
    let timestamp = chrono::Utc::now().naive_utc().format("%Y-%m-%d %H:%M:%S");

    let mut html = String::with_capacity(64 * 1024);
    write_debug_html_head(&mut html, title);
    write_debug_html_header(&mut html, title, &timestamp.to_string(), session);
    write_debug_toc(&mut html, messages);

    let mut round = 0u32;
    for (idx, msg) in messages.iter().enumerate() {
        let msg_num = idx + 1;
        match msg.role.as_str() {
            "user" => {
                round += 1;
                write_debug_user_message(&mut html, msg, round, msg_num);
            }
            "assistant" => write_debug_assistant_message(&mut html, msg, msg_num),
            _ => {}
        }
    }

    let _ = writeln!(
        html,
        "<hr class=\"round-sep\"><div class=\"meta\">对话结束 — 共 {round} 轮</div>"
    );
    html.push_str("</body>\n</html>\n");
    html
}

fn write_debug_html_head(html: &mut String, title: &str) {
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
}

fn write_debug_html_header(
    html: &mut String,
    title: &str,
    timestamp: &str,
    session: &chat_sessions::Model,
) {
    use std::fmt::Write;

    let _ = writeln!(html, "<h1>调试导出: {}</h1>", html_escape(title));
    let _ = writeln!(
        html,
        "<div class=\"meta\">导出时间: {} &nbsp;|&nbsp; 会话 ID: <code>{}</code></div>",
        timestamp, session.id
    );
    html.push_str(
        "<div class=\"warning\">⚠️ 本文档包含 LLM 调试数据，仅供开发者参考</div>\n",
    );
}

fn write_debug_toc(html: &mut String, messages: &[chat_messages::Model]) {
    use std::fmt::Write;

    let mut toc_entries: Vec<(u32, String)> = Vec::new();
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
            let _ = writeln!(html, "<a class=\"toc-link\" href=\"#round-{}\"><span class=\"toc-round\">第 {} 轮</span> {}</a>",
                    r, r, html_escape(preview));
        }
        html.push_str("</nav>\n</details>\n");
    }
}

fn write_debug_user_message(
    html: &mut String,
    msg: &chat_messages::Model,
    round: u32,
    msg_num: usize,
) {
    use std::fmt::Write;

    html.push_str("<div class=\"msg-card\">\n");
    let _ = writeln!(html, "<h2 id=\"round-{round}\"><span class=\"badge badge-user\">User</span> 消息 #{msg_num}</h2>");
    let _ = writeln!(
        html,
        "<div class=\"answer-block\">{}</div>",
        html_escape(&msg.content)
    );
    if let Some(ref refs) = msg.material_refs {
        let refs_str = serde_json::to_string_pretty(refs).unwrap_or_default();
        html.push_str("<div class=\"section-title\">附加材料</div>\n");
        let _ = writeln!(html, "<details><summary>material_refs JSON</summary><pre><code>{}</code></pre></details>",
                        html_escape(&refs_str));
    }
    html.push_str("</div>\n");
}

fn write_debug_assistant_message(
    html: &mut String,
    msg: &chat_messages::Model,
    msg_num: usize,
) {
    use std::fmt::Write;

    html.push_str("<div class=\"msg-card\">\n");
    let _ = writeln!(
        html,
        "<h2><span class=\"badge badge-assistant\">Assistant</span> 消息 #{msg_num}</h2>"
    );
    if msg.total_tokens > 0 {
        let _ = writeln!(html, "<div class=\"meta\">Token: prompt=<code>{}</code> completion=<code>{}</code> total=<code>{}</code></div>",
                        msg.prompt_tokens, msg.completion_tokens, msg.total_tokens);
    }

    let tu = msg.token_usage.as_ref();
    if let Some(debug_ctx) = tu.and_then(|v| v.get("debugContext")) {
        write_debug_context(html, debug_ctx);
    } else {
        html.push_str("<div class=\"meta\">无调试数据</div>\n");
    }

    write_debug_tool_records(html, tu);
    write_debug_citations(html, tu);
    write_debug_answer(html, msg);
    html.push_str("</div>\n");
}

fn write_debug_citations(html: &mut String, tu: Option<&serde_json::Value>) {
    use std::fmt::Write;

    let Some(citations) = tu
        .and_then(|v| v.get("citations"))
        .and_then(serde_json::Value::as_array)
        .filter(|items| !items.is_empty())
    else {
        return;
    };

    html.push_str("<div class=\"section-title\">引用来源</div>\n");
    html.push_str("<table class=\"kv-table\"><tr><td>文档</td><td>章节</td><td>位置</td><td>分块</td><td>相关度</td></tr>");
    for citation in citations {
        let title = citation_title(citation);
        let heading = citation
            .get("headingPath")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("—");
        let chunk = citation
            .get("chunkId")
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| "—".to_string(), short_id);
        let score = citation
            .get("score")
            .and_then(serde_json::Value::as_f64)
            .map_or_else(|| "—".to_string(), |value| format!("{value:.2}"));
        let _ = write!(
            html,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
            html_escape(&title),
            html_escape(heading),
            html_escape(&citation_location(citation)),
            html_escape(&chunk),
            score
        );
    }
    html.push_str("</table>\n");
}

fn write_debug_context(html: &mut String, dc: &serde_json::Value) {
    use std::fmt::Write;

    html.push_str("<div class=\"section-title\">调试上下文</div>\n");
    if let Some(sp) = dc.get("system_prompt").and_then(|v| v.as_str()) {
        let _ = writeln!(
            html,
            "<details><summary>System Prompt ({} 字符)</summary><pre>{}</pre></details>",
            sp.chars().count(),
            html_escape(sp)
        );
    }

    if let Some(up) = dc.get("user_prompt").and_then(|v| v.as_str()) {
        let _ = writeln!(
            html,
            "<details><summary>User Prompt ({} 字符)</summary><pre>{}</pre></details>",
            up.chars().count(),
            html_escape(up)
        );
    }

    if let Some(cs) = dc.get("config_snapshot") {
        let cs_str = serde_json::to_string_pretty(cs).unwrap_or_default();
        let _ = writeln!(
            html,
            "<details><summary>配置快照</summary><pre><code>{}</code></pre></details>",
            html_escape(&cs_str)
        );
    }

    write_debug_compaction(html, dc.get("compaction"));
    write_debug_semantic_recall(html, dc.get("semantic_recall"));
    write_debug_chat_history(html, dc.get("chat_history_summary"));
    write_debug_tool_rounds(html, dc);
}

fn write_debug_compaction(html: &mut String, comp: Option<&serde_json::Value>) {
    use std::fmt::Write;

    let Some(comp) = comp else {
        return;
    };
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
    html.push_str(
        "<details><summary>Compaction 状态</summary>\n<table class=\"kv-table\">",
    );
    for (k, label) in [
        ("triggered", "触发"),
        ("summary_length", "摘要长度"),
        ("history_total", "历史消息总数"),
        ("history_tokens", "历史估算 tokens"),
        ("recent_start", "recent_start"),
    ] {
        let _ = write!(html, "<tr><td>{label}</td><td>{}</td></tr>", kv(k));
    }
    html.push_str("</table></details>\n");
}

fn write_debug_semantic_recall(html: &mut String, sr: Option<&serde_json::Value>) {
    use std::fmt::Write;

    let Some(sr) = sr else {
        return;
    };
    let strategy = sr.get("strategy").and_then(|v| v.as_str()).unwrap_or("—");
    let ctx_len = sr
        .get("context_length")
        .and_then(serde_json::Value::as_u64)
        .map_or_else(|| "—".to_string(), |v| v.to_string());
    let has_recall = sr
        .get("has_recall")
        .and_then(serde_json::Value::as_bool)
        .map_or_else(|| "—".to_string(), |b| b.to_string());
    html.push_str("<details><summary>语义检索</summary>\n<table class=\"kv-table\">");
    let _ = write!(html, "<tr><td>策略</td><td>{strategy}</td></tr>");
    let _ = write!(html, "<tr><td>上下文长度</td><td>{ctx_len}</td></tr>");
    let _ = write!(html, "<tr><td>有召回</td><td>{has_recall}</td></tr>");
    html.push_str("</table></details>\n");
}

fn write_debug_chat_history(html: &mut String, chs: Option<&serde_json::Value>) {
    use std::fmt::Write;

    let Some(chs) = chs else {
        return;
    };
    let mc = chs
        .get("message_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let _ = write!(html, "<details><summary>Chat History ({mc} 条消息)</summary>\n<table class=\"kv-table\"><tr><td>#</td><td>角色</td><td>内容/文本数</td><td>工具调用</td></tr>");
    if let Some(msgs) = chs.get("messages").and_then(|v| v.as_array()) {
        for (i, entry) in msgs.iter().enumerate() {
            write_debug_history_row(html, i, entry);
        }
    }
    html.push_str("</table></details>\n");
}

fn write_debug_history_row(html: &mut String, i: usize, entry: &serde_json::Value) {
    use std::fmt::Write;

    let role = entry.get("role").and_then(|v| v.as_str()).unwrap_or("—");
    let content_len = entry
        .get("content_count")
        .or_else(|| entry.get("text_items"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let has_tools = entry
        .get("has_tool_calls")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let _ = write!(
        html,
        "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
        i + 1,
        role,
        content_len,
        if has_tools { "✓" } else { "—" }
    );
}

fn write_debug_tool_rounds(html: &mut String, dc: &serde_json::Value) {
    let Some(rounds) = dc.get("toolRounds").and_then(|v| v.as_array()) else {
        return;
    };
    if rounds.is_empty() {
        return;
    }
    html.push_str("<div class=\"section-title\">对话脉络（LLM 完整交互时间线）</div>\n");
    html.push_str("<div class=\"timeline\">\n");
    if let Some(up) = dc.get("user_prompt").and_then(|v| v.as_str()) {
        write_debug_user_timeline_entry(html, up);
    }
    for round in rounds {
        write_debug_tool_timeline_entry(html, round);
    }
    html.push_str("</div>\n");
}

fn write_debug_user_timeline_entry(html: &mut String, user_prompt: &str) {
    use std::fmt::Write;

    html.push_str("<div class=\"timeline-entry user-entry\">\n");
    html.push_str("<div class=\"timeline-label\">📥 用户输入</div>\n");
    let _ = writeln!(
        html,
        "<pre class=\"timeline-content\">{}</pre>",
        html_escape(user_prompt)
    );
    html.push_str("</div>\n");
}

fn write_debug_tool_timeline_entry(html: &mut String, round: &serde_json::Value) {
    use std::fmt::Write;

    let round_num = round
        .get("round")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let tool_name = round
        .get("toolName")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let duration_ms = round
        .get("durationMs")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let result_full = round
        .get("resultFull")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let args = round
        .get("arguments")
        .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
        .unwrap_or_default();

    html.push_str("<div class=\"timeline-entry tool-entry\">\n");
    let _ = writeln!(html, "<div class=\"timeline-label\">🔧 工具调用 #{round_num} — {tool_name} ({duration_ms}ms)</div>");
    if !args.is_empty() {
        html.push_str("<details><summary>参数</summary><pre><code>");
        html.push_str(&html_escape(&args));
        html.push_str("</code></pre></details>\n");
    }
    html.push_str("<details><summary>完整返回结果");
    if !result_full.is_empty() {
        let _ = write!(html, " ({} 字符)", result_full.chars().count());
    }
    html.push_str("</summary><pre>");
    html.push_str(&html_escape(result_full));
    html.push_str("</pre></details>\n");
    html.push_str("</div>\n");
}

fn write_debug_tool_records(html: &mut String, tu: Option<&serde_json::Value>) {
    let has_content_parts =
        tu.and_then(|v| v.get("content_parts")?.as_array().filter(|a| !a.is_empty()));
    let has_tool_calls = has_content_parts.is_some()
        || tu
            .and_then(|v| v.get("toolCalls")?.as_array().filter(|a| !a.is_empty()))
            .is_some();
    if !has_tool_calls {
        return;
    }

    html.push_str("<div class=\"section-title\">工具调用记录</div>\n");
    if let Some(parts) = has_content_parts {
        for part in parts
            .iter()
            .filter(|p| p.get("type").and_then(|v| v.as_str()) == Some("tool_call"))
        {
            write_debug_tool_record(html, part);
        }
    } else if let Some(calls) = tu.and_then(|v| v.get("toolCalls")?.as_array()) {
        for call in calls {
            write_debug_tool_record(html, call);
        }
    }
}

fn write_debug_tool_record(html: &mut String, part: &serde_json::Value) {
    use std::fmt::Write;

    let name = part
        .get("toolName")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let duration = part
        .get("durationMs")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    html.push_str("<div class=\"tool-call\">");
    let _ = writeln!(
        html,
        "<span class=\"tool-name\">🔍 {}</span><span class=\"tool-duration\">{}</span>",
        html_escape(name),
        format_duration(duration)
    );
    if let Some(args) = part.get("arguments") {
        let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
        let _ = writeln!(
            html,
            "<details><summary>参数</summary><pre><code>{}</code></pre></details>",
            html_escape(&args_str)
        );
    }
    let result_text = part
        .get("resultFull")
        .and_then(|v| v.as_str())
        .or_else(|| part.get("resultPreview").and_then(|v| v.as_str()))
        .unwrap_or("");
    let _ = writeln!(
        html,
        "<details><summary>完整结果 ({} 字符)</summary><pre>{}</pre></details>",
        result_text.chars().count(),
        html_escape(result_text)
    );
    html.push_str("</div>\n");
}

fn write_debug_answer(html: &mut String, msg: &chat_messages::Model) {
    use std::fmt::Write;

    let is_error = msg.content.starts_with('\u{26a0}');
    html.push_str("<div class=\"section-title\">回答内容</div>\n");
    if !msg.content.is_empty() {
        let _ = writeln!(
            html,
            "<div class=\"answer-block{}\">{}</div>",
            if is_error { " error-block" } else { "" },
            html_escape(&msg.content)
        );
    }
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
