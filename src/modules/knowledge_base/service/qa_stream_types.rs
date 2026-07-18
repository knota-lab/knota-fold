use serde::Serialize;
use utoipa::ToSchema;

/// SSE event payload sent from the streaming QA pipeline to the frontend.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "type", content = "data", rename_all_fields = "camelCase")]
pub enum QaEvent {
    Started {
        session_id: String,
    },
    PhaseChanged {
        phase: QaPhase,
    },
    AnswerToken {
        token: String,
    },
    Completed {
        response: QaStreamResponse,
    },
    Error {
        message: String,
    },
    ToolCallStarted {
        tool_name: String,
        tool_call_id: String,
        arguments: serde_json::Value,
    },
    ToolCallCompleted {
        tool_name: String,
        tool_call_id: String,
        result_preview: String,
        duration_ms: u64,
    },
}

/// Granular phase indicator so the frontend can render progress.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "type", content = "detail", rename_all_fields = "camelCase")]
pub enum QaPhase {
    MaterialProcessing {
        strategy: String,
        total_chunks: Option<usize>,
    },
    GeneratingAnswer,
    Persisting,
}

/// Final response returned in the `Completed` event.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct QaStreamResponse {
    pub answer: String,
    pub citations: Vec<super::qa_types::Citation>,
    pub intent: String,
    pub output_format: String,
    pub strategy: String,
    pub mode: String,
    pub usage: super::qa_types::TokenUsage,
    pub session_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qa_event_tool_call_started_serializes_camel_case_fields() {
        let event = QaEvent::ToolCallStarted {
            tool_name: "list_materials".into(),
            tool_call_id: "call_123".into(),
            arguments: serde_json::json!({}),
        };
        let json = serde_json::to_string(&event).unwrap();
        // Variant tag stays PascalCase
        assert!(
            json.contains("\"type\":\"ToolCallStarted\""),
            "tag must be PascalCase: {json}"
        );
        // Field names are camelCase
        assert!(
            json.contains("\"toolName\":\"list_materials\""),
            "tool_name → toolName: {json}"
        );
        assert!(
            json.contains("\"toolCallId\":\"call_123\""),
            "tool_call_id → toolCallId: {json}"
        );
    }

    #[test]
    fn qa_event_completed_variant_tag_preserved() {
        let event = QaEvent::Completed {
            response: QaStreamResponse {
                answer: "hi".into(),
                citations: vec![],
                intent: "answer".into(),
                output_format: "markdown".into(),
                strategy: "full".into(),
                mode: "strong".into(),
                usage: super::super::qa_types::TokenUsage {
                    prompt_tokens: 1,
                    completion_tokens: 2,
                    total_tokens: 3,
                },
                session_id: "s1".into(),
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(
            json.contains("\"type\":\"Completed\""),
            "tag must be PascalCase: {json}"
        );
    }

    #[test]
    fn qa_phase_rename_all_fields_camel_case() {
        let phase = QaPhase::MaterialProcessing {
            strategy: "v3".into(),
            total_chunks: Some(5),
        };
        let json = serde_json::to_string(&phase).unwrap();
        assert!(
            json.contains("\"type\":\"MaterialProcessing\""),
            "tag PascalCase: {json}"
        );
        assert!(
            json.contains("\"totalChunks\":5"),
            "total_chunks → totalChunks: {json}"
        );
    }
}
