//! SSE-aware hook for rig-core's multi-turn streaming agent loop.
//!
//! Implements [`PromptHook`] to emit `ToolCallStarted` / `ToolCallCompleted`
//! events through an mpsc channel so the frontend can show live tool progress.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rig::agent::{HookAction, PromptHook, ToolCallHookAction};
use rig::completion::CompletionModel;
use tokio::sync::mpsc;

use super::super::qa_stream_types::QaEvent;
use super::super::qa_types::Citation;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Global frontend tool names (not using the `page_` prefix).
/// These tools execute in the browser just like `page_*` tools but are not
/// bound to the current page's capabilities.
const GLOBAL_FRONTEND_TOOLS: &[&str] = &["list_available_pages", "navigate_to_page"];

/// Check whether a tool is executed in the frontend browser (not backend).
///
/// All frontend tools either carry the `page_` prefix or are listed in
/// [`GLOBAL_FRONTEND_TOOLS`]. The Hook must skip emitting `ToolCallStarted`
/// for these because `FrontendToolStub` emits its own event.
fn is_frontend_tool(name: &str) -> bool {
    name.starts_with("page_") || GLOBAL_FRONTEND_TOOLS.contains(&name)
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

// ---------------------------------------------------------------------------
// Hook struct
// ---------------------------------------------------------------------------

/// Hook that captures tool-call lifecycle events and forwards them as SSE
/// [`QaEvent`] variants through an mpsc channel.
/// A completed tool call record, collected for persistence and export.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub tool_call_id: String,
    pub arguments: serde_json::Value,
    pub duration_ms: u64,
    pub result_preview: String,
    pub result_full: String,
}

/// An ordered content segment within an assistant message.
///
/// Captures the interleaving of text and tool calls as they happen during
/// the streaming multi-turn agent loop, preserving the true timeline.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    #[serde(rename_all = "camelCase")]
    Text { content: String, created_at: String },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        tool_name: String,
        tool_call_id: String,
        arguments: serde_json::Value,
        duration_ms: u64,
        result_preview: String,
        result_full: String,
        created_at: String,
    },
}

/// Pending call data stored between `on_tool_call` and `on_tool_result`.
#[derive(Debug, Clone)]
struct PendingCall {
    tool_call_id: String,
    arguments: serde_json::Value,
}

/// A single tool-call round captured for debug export.
///
/// Records the complete interaction: what the LLM requested and what the tool returned.
/// These are collected in order and stored in `debug_context.tool_rounds`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRound {
    /// The tool call round number (1-indexed, incrementing across the multi-turn loop).
    pub round: u32,
    /// Tool name requested by the LLM.
    pub tool_name: String,
    /// Call ID from the LLM's `tool_call` response.
    pub tool_call_id: String,
    /// Arguments the LLM passed to the tool.
    pub arguments: serde_json::Value,
    /// The full result returned by the tool (truncated to 50KB).
    pub result_full: String,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
}

#[derive(Clone)]
pub struct QaV3Hook {
    /// Channel that carries serialised [`QaEvent`] JSON strings to the SSE
    /// response stream.
    tx: mpsc::Sender<String>,
    /// Per-call timing data: `internal_call_id -> start Instant`.
    timings: Arc<Mutex<HashMap<String, Instant>>>,
    /// Completed tool call records for persistence/export (legacy flat list).
    tool_records: Arc<Mutex<Vec<ToolCallRecord>>>,
    /// Pending call data: `internal_call_id -> (tool_call_id, arguments)`.
    pending_calls: Arc<Mutex<HashMap<String, PendingCall>>>,
    /// Citations extracted from `search_knowledge_base` tool results.
    citations: Arc<Mutex<Vec<Citation>>>,
    /// Ordered content parts tracking the true interleaving of text and tool calls.
    content_parts: Arc<Mutex<Vec<ContentPart>>>,
    /// Text accumulated since the last tool call (or since the beginning).
    /// Flushed into `content_parts` as a Text part whenever a tool call starts/ends.
    pending_text: Arc<Mutex<String>>,
    /// Debug context snapshot captured before the streaming loop begins.
    debug_context: Arc<Mutex<Option<serde_json::Value>>>,
    /// Ordered tool rounds for debug export (each round = one tool call + result).
    tool_rounds: Arc<Mutex<Vec<ToolRound>>>,
    /// Monotonically increasing round counter for `tool_rounds`.
    tool_round_counter: Arc<Mutex<u32>>,
}

impl QaV3Hook {
    #[must_use]
    pub fn new(tx: mpsc::Sender<String>) -> Self {
        Self {
            tx,
            timings: Arc::new(Mutex::new(HashMap::new())),
            tool_records: Arc::new(Mutex::new(Vec::new())),
            pending_calls: Arc::new(Mutex::new(HashMap::new())),
            citations: Arc::new(Mutex::new(Vec::new())),
            content_parts: Arc::new(Mutex::new(Vec::new())),
            pending_text: Arc::new(Mutex::new(String::new())),
            debug_context: Arc::new(Mutex::new(None)),
            tool_rounds: Arc::new(Mutex::new(Vec::new())),
            tool_round_counter: Arc::new(Mutex::new(0)),
        }
    }

    /// Take all collected tool call records, clearing the internal buffer.
    pub fn take_tool_records(&self) -> Vec<ToolCallRecord> {
        std::mem::take(
            &mut self
                .tool_records
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Take all collected citations, clearing the internal buffer.
    pub fn take_citations(&self) -> Vec<Citation> {
        std::mem::take(
            &mut self
                .citations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Take all ordered content parts, flushing any remaining pending text first.
    pub fn take_content_parts(&self) -> Vec<ContentPart> {
        // Flush any remaining pending text
        if let Ok(mut text) = self.pending_text.lock() {
            if !text.is_empty() {
                if let Ok(mut parts) = self.content_parts.lock() {
                    parts.push(ContentPart::Text {
                        content: std::mem::take(&mut *text),
                        created_at: now_iso(),
                    });
                }
            }
        }
        std::mem::take(
            &mut self
                .content_parts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Set the debug context snapshot for this turn.
    pub fn set_debug_context(&self, ctx: serde_json::Value) {
        if let Ok(mut guard) = self.debug_context.lock() {
            *guard = Some(ctx);
        }
    }

    /// Take the debug context snapshot, clearing the internal buffer.
    #[must_use]
    pub fn take_debug_context(&self) -> Option<serde_json::Value> {
        self.debug_context
            .lock()
            .ok()
            .and_then(|mut guard| guard.take())
    }

    /// Take all collected tool rounds, clearing the internal buffer.
    pub fn take_tool_rounds(&self) -> Vec<ToolRound> {
        std::mem::take(
            &mut self
                .tool_rounds
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Flush pending text into `content_parts` as a Text part (if non-empty).
    fn flush_pending_text(parts: &mut Vec<ContentPart>, pending_text: &mut String) {
        if !pending_text.is_empty() {
            parts.push(ContentPart::Text {
                content: std::mem::take(pending_text),
                created_at: now_iso(),
            });
        }
    }

    /// Append a text delta to the pending text buffer.
    fn append_text_delta(&self, delta: &str) {
        if let Ok(mut text) = self.pending_text.lock() {
            text.push_str(delta);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Serialise a [`QaEvent`] to JSON and send it through the channel.
/// Silently drops the event if the receiver has been closed (frontend
/// disconnected) or serialisation fails — the hook never blocks the agent
/// loop.
async fn send_event(tx: &mpsc::Sender<String>, event: QaEvent) {
    let json = match serde_json::to_string(&event) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!(error = %e, "Failed to serialise QaEvent");
            return;
        }
    };
    if tx.send(json).await.is_err() {
        tracing::debug!("Event channel closed — frontend disconnected");
    }
}

// ---------------------------------------------------------------------------
// PromptHook implementation
// ---------------------------------------------------------------------------

impl<M> PromptHook<M> for QaV3Hook
where
    M: CompletionModel,
{
    /// Record the start time and emit a [`QaEvent::ToolCallStarted`] SSE event.
    async fn on_tool_call(
        &self,
        tool_name: &str,
        tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
    ) -> ToolCallHookAction {
        // LLMs like Ollama/qwen may not generate a tool_call ID.
        // Fall back to the internal_call_id so every event has a traceable ID.
        let call_id = tool_call_id.unwrap_or_else(|| internal_call_id.to_owned());

        // Record start time under the internal_call_id.
        if let Ok(mut map) = self.timings.lock() {
            map.insert(internal_call_id.to_owned(), Instant::now());
        }

        let arguments: serde_json::Value =
            serde_json::from_str(args).unwrap_or(serde_json::Value::Null);

        // Flush any pending text before the tool call starts
        if let (Ok(mut parts), Ok(mut text)) =
            (self.content_parts.lock(), self.pending_text.lock())
        {
            Self::flush_pending_text(&mut parts, &mut text);
        }

        // Save pending call data for use in on_tool_result
        if let Ok(mut map) = self.pending_calls.lock() {
            map.insert(
                internal_call_id.to_owned(),
                PendingCall {
                    tool_call_id: call_id.clone(),
                    arguments: arguments.clone(),
                },
            );
        }

        // For frontend tools: record timing + pending data but skip SSE emission.
        // The FrontendToolStub itself emits ToolCallStarted in its call() method.
        if !is_frontend_tool(tool_name) {
            let event = QaEvent::ToolCallStarted {
                tool_name: tool_name.to_owned(),
                tool_call_id: call_id,
                arguments,
            };
            send_event(&self.tx, event).await;
        }
        ToolCallHookAction::cont()
    }

    /// Calculate elapsed time and emit a [`QaEvent::ToolCallCompleted`] SSE event.
    async fn on_tool_result(
        &self,
        tool_name: &str,
        tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
        result: &str,
    ) -> HookAction {
        let duration_ms = self
            .timings
            .lock()
            .ok()
            .and_then(|mut map| map.remove(internal_call_id))
            .map_or(0, |start| {
                u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
            });

        let result_preview = truncate_str(result, 500).to_owned();
        let result_full = truncate_str(result, 50_000).to_owned();

        // Retrieve saved pending call data
        let pending = self
            .pending_calls
            .lock()
            .ok()
            .and_then(|mut map| map.remove(internal_call_id));

        let (saved_call_id, saved_args) = pending.map_or_else(
            || {
                let parsed_args: serde_json::Value =
                    serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
                (tool_call_id.clone().unwrap_or_default(), parsed_args)
            },
            |p| (p.tool_call_id, p.arguments),
        );

        // Record completed tool call for persistence/export (legacy flat list)
        if let Ok(mut records) = self.tool_records.lock() {
            records.push(ToolCallRecord {
                tool_name: tool_name.to_owned(),
                tool_call_id: saved_call_id.clone(),
                arguments: saved_args.clone(),
                duration_ms,
                result_preview: result_preview.clone(),
                result_full: result_full.clone(),
            });
        }

        // Record tool round for debug export (complete interaction trace)
        let round_num = self.tool_round_counter.lock().map_or(1, |mut g| {
            *g += 1;
            *g
        });
        if let Ok(mut rounds) = self.tool_rounds.lock() {
            rounds.push(ToolRound {
                round: round_num,
                tool_name: tool_name.to_owned(),
                tool_call_id: saved_call_id.clone(),
                arguments: saved_args.clone(),
                result_full: result_full.clone(),
                duration_ms,
            });
        }

        // Record completed tool call as an ordered content part
        if let Ok(mut parts) = self.content_parts.lock() {
            parts.push(ContentPart::ToolCall {
                tool_name: tool_name.to_owned(),
                tool_call_id: saved_call_id.clone(),
                arguments: saved_args.clone(),
                duration_ms,
                result_preview: result_preview.clone(),
                result_full,
                created_at: now_iso(),
            });
        }

        let event = QaEvent::ToolCallCompleted {
            tool_name: tool_name.to_owned(),
            tool_call_id: saved_call_id,
            result_preview,
            duration_ms,
        };

        send_event(&self.tx, event).await;

        // H6: Extract citations from search_knowledge_base results.
        if tool_name == "search_knowledge_base" {
            extract_citations_from_result(result, &self.citations);
        } else if tool_name == "read_knowledge_base_lines" {
            merge_line_read_citation(result, &self.citations);
        }

        HookAction::cont()
    }

    /// Track text deltas for `content_parts` ordering.
    async fn on_text_delta(
        &self,
        text_delta: &str,
        _aggregated_text: &str,
    ) -> HookAction {
        self.append_text_delta(text_delta);
        HookAction::cont()
    }

    /// NO-OP — tool call deltas are not needed for SSE progress.
    async fn on_tool_call_delta(
        &self,
        _tool_call_id: &str,
        _internal_call_id: &str,
        _tool_name: Option<&str>,
        _tool_call_delta: &str,
    ) -> HookAction {
        HookAction::cont()
    }
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

/// Truncate `s` to at most `max` byte length, respecting char boundaries.
fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    // Walk char boundaries so we never slice mid-byte.
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if i + c.len_utf8() > max {
            break;
        }
        end = i + c.len_utf8();
    }
    &s[..end]
}

// ---------------------------------------------------------------------------
// Citation extraction
// ---------------------------------------------------------------------------

/// Parse citations from `search_knowledge_base` tool result text.
///
/// The tool outputs lines like:
///   `1. [heading] (分数: 0.85, 文档ID: uuid, 分块ID: uuid)`
/// We extract `document_id`, `chunk_id`, content preview, and score.
fn extract_citations_from_result(result: &str, citations: &Arc<Mutex<Vec<Citation>>>) {
    if let Ok(mut list) = citations.lock() {
        for line in result.lines() {
            // Try to extract: (分数: X.XX, 文档ID: UUID, 分块ID: UUID)
            let Some(score) = extract_field_f64(line, "分数: ") else {
                continue;
            };
            let Some(doc_id_str) = extract_field(line, "文档ID: ") else {
                continue;
            };
            let Some(chunk_id_str) = extract_field(line, "分块ID: ") else {
                continue;
            };

            let Ok(doc_id) = uuid::Uuid::parse_str(doc_id_str) else {
                continue;
            };
            let chunk_id = uuid::Uuid::parse_str(chunk_id_str).ok();
            let heading_path = extract_heading_path(line).map(str::to_string);
            let (start_line, end_line) = extract_line_range(line);

            // Extract content after "相关内容:\n" on the next line
            // Since we're iterating line by line, we'll grab a truncated preview
            // from the tool result itself
            let content = extract_content_after_header(line);

            // Dedup: skip if we already have this chunk_id
            if let Some(cid) = chunk_id {
                if list.iter().any(|c| c.chunk_id == Some(cid)) {
                    continue;
                }
            }

            list.push(Citation {
                document_id: doc_id,
                chunk_id,
                document_title: None,
                heading_path,
                start_line,
                end_line,
                content,
                score,
            });
        }
    }
}

fn merge_line_read_citation(result: &str, citations: &Arc<Mutex<Vec<Citation>>>) {
    let Some((document_title, document_id, start_line, end_line)) =
        parse_read_lines_header(result)
    else {
        return;
    };

    if let Ok(mut list) = citations.lock() {
        if let Some(existing) = list
            .iter_mut()
            .rev()
            .find(|citation| citation.document_id == document_id)
        {
            existing.document_title = Some(document_title);
            existing.start_line = Some(start_line);
            existing.end_line = Some(end_line);
            return;
        }

        list.push(Citation {
            document_id,
            chunk_id: None,
            document_title: Some(document_title),
            heading_path: None,
            start_line: Some(start_line),
            end_line: Some(end_line),
            content: extract_line_preview(result),
            score: 0.0,
        });
    }
}

fn parse_read_lines_header(result: &str) -> Option<(String, uuid::Uuid, i32, i32)> {
    let first_line = result.lines().next()?;
    let title_start = first_line.find('《')? + '《'.len_utf8();
    let title_end = first_line[title_start..].find('》')? + title_start;
    let title = first_line[title_start..title_end].to_string();

    let id_start = first_line[title_end..].find('(')? + title_end + 1;
    let id_end = first_line[id_start..].find(')')? + id_start;
    let document_id = uuid::Uuid::parse_str(&first_line[id_start..id_end]).ok()?;

    let line_marker = "第 ";
    let line_start = first_line[id_end..].find(line_marker)? + id_end + line_marker.len();
    let line_end = first_line[line_start..].find('行')? + line_start;
    let raw = first_line[line_start..line_end].trim();
    let (start_line, end_line) = parse_line_range(raw);

    Some((title, document_id, start_line?, end_line?))
}

fn parse_line_range(raw: &str) -> (Option<i32>, Option<i32>) {
    if let Some((start, end)) = raw.split_once('-') {
        return (start.trim().parse().ok(), end.trim().parse().ok());
    }
    let line_num = raw.trim().parse().ok();
    (line_num, line_num)
}

fn extract_line_preview(result: &str) -> String {
    result
        .lines()
        .skip(1)
        .find(|line| !line.trim().is_empty())
        .map_or_else(String::new, |line| line.chars().take(120).collect())
}

fn extract_heading_path(line: &str) -> Option<&str> {
    let start = line.find('[')?;
    let rest = &line[start + 1..];
    let end = rest.find("] (")?;
    Some(&rest[..end])
}

fn extract_line_range(line: &str) -> (Option<i32>, Option<i32>) {
    let Some(raw) = extract_field(line, "位置: 第 ") else {
        return (None, None);
    };
    let trimmed = raw.trim_end_matches('行').trim();
    parse_line_range(trimmed)
}

/// Extract a field value between the marker and the next `,` or `)`.
fn extract_field<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let start = line.find(marker)?;
    let rest = &line[start + marker.len()..];
    let end = rest.find([',', ')']).unwrap_or(rest.len());
    Some(rest[..end].trim())
}

/// Extract f64 field value.
fn extract_field_f64(line: &str, marker: &str) -> Option<f64> {
    let val = extract_field(line, marker)?;
    val.parse().ok()
}

/// Extract content preview from a result line (the heading part).
fn extract_content_after_header(line: &str) -> String {
    // Format: "1. [heading] (分数: ...)\n相关内容:\n..."
    // We only have one line at a time, so extract the heading as content preview
    if let Some(start) = line.find("] (") {
        if let Some(heading_start) = line.find('[') {
            let heading = &line[heading_start + 1..start];
            return heading.to_string();
        }
    }
    line.chars().take(100).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_preserves_short_strings() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_truncates_at_char_boundary() {
        let s = "abcdefghij";
        assert_eq!(truncate_str(s, 5), "abcde");
    }

    #[test]
    fn truncate_handles_multibyte() {
        let s = "日本語テスト";
        // Each char is 3 bytes; limit of 6 bytes should give first 2 chars.
        assert_eq!(truncate_str(s, 6), "日本");
    }

    #[test]
    fn extract_line_range_parses_single_line() {
        let line = "1. [标题] (分数: 0.85, 位置: 第 12 行, 文档ID: 019ea257-0000-7000-8000-000000000001, 分块ID: 019ea257-0000-7000-8000-000000000002)";

        assert_eq!(extract_line_range(line), (Some(12), Some(12)));
    }

    #[test]
    fn extract_line_range_parses_line_range() {
        let line = "1. [标题] (分数: 0.85, 位置: 第 12-18 行, 文档ID: 019ea257-0000-7000-8000-000000000001, 分块ID: 019ea257-0000-7000-8000-000000000002)";

        assert_eq!(extract_line_range(line), (Some(12), Some(18)));
    }

    #[test]
    fn parse_read_lines_header_extracts_title_document_and_range() {
        let document_id = uuid::Uuid::now_v7();
        let result =
            format!("文档《前端开发指导》({document_id}) 第 10-20 行：\n10 | 内容");

        let (title, parsed_document_id, start_line, end_line) =
            parse_read_lines_header(&result).unwrap();

        assert_eq!(title, "前端开发指导");
        assert_eq!(parsed_document_id, document_id);
        assert_eq!(start_line, 10);
        assert_eq!(end_line, 20);
    }

    #[test]
    fn merge_line_read_citation_fills_existing_search_citation() {
        let document_id = uuid::Uuid::now_v7();
        let chunk_id = uuid::Uuid::now_v7();
        let citations = Arc::new(Mutex::new(vec![Citation {
            document_id,
            chunk_id: Some(chunk_id),
            document_title: None,
            heading_path: Some("3. 组件开发规范".to_string()),
            start_line: None,
            end_line: None,
            content: "3. 组件开发规范".to_string(),
            score: 0.7,
        }]));
        let result = format!(
            "文档《knota-studio 前端开发指导》({document_id}) 第 33-45 行：\n33 | 内容"
        );

        merge_line_read_citation(&result, &citations);

        let guard = citations.lock().unwrap();
        assert_eq!(guard.len(), 1);
        assert_eq!(
            guard[0].document_title.as_deref(),
            Some("knota-studio 前端开发指导")
        );
        assert_eq!(guard[0].start_line, Some(33));
        assert_eq!(guard[0].end_line, Some(45));
        assert_eq!(guard[0].chunk_id, Some(chunk_id));
    }
}
