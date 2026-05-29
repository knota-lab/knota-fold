//! Frontend tool stub — registers with the broker and blocks until the
//! frontend POSTs the result via the HTTP endpoint.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::tool_result_broker::{ReceiverError, ToolResultBroker, ToolResultReceiver};
use crate::modules::knowledge_base::service::qa_stream_types::QaEvent;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum FrontendToolError {
    Timeout(String),
    Broker(String),
    Internal(String),
}

impl fmt::Display for FrontendToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout(msg) => write!(f, "frontend tool timeout: {msg}"),
            Self::Broker(msg) => {
                write!(f, "frontend tool broker error: {msg}")
            }
            Self::Internal(msg) => write!(f, "frontend tool error: {msg}"),
        }
    }
}

impl std::error::Error for FrontendToolError {}

impl From<ReceiverError> for FrontendToolError {
    fn from(e: ReceiverError) -> Self {
        match e {
            ReceiverError::Timeout => Self::Timeout("timed out".into()),
            ReceiverError::Closed => Self::Broker("channel closed".into()),
            ReceiverError::Internal(msg) => Self::Internal(msg),
        }
    }
}

// ---------------------------------------------------------------------------
// Args — accepts any JSON value
// ---------------------------------------------------------------------------

/// Arguments for frontend tools. The actual schema is defined by the frontend
/// and forwarded as-is; we just capture the raw JSON.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FrontendToolArgs(pub serde_json::Value);

// ---------------------------------------------------------------------------
// Timeout lookup
// ---------------------------------------------------------------------------

/// Returns the timeout for a given page tool name.
fn tool_timeout(name: &str) -> Duration {
    match name {
        "page_query_table" => Duration::from_secs(15),
        "page_list_actions" => Duration::from_secs(10),
        "page_get_action_detail" => Duration::from_secs(10),
        "page_get_form_values" => Duration::from_secs(15),
        "page_execute_action" => Duration::from_secs(60),
        _ => Duration::from_secs(30),
    }
}

// ---------------------------------------------------------------------------
// SSE helper
// ---------------------------------------------------------------------------

/// Fire-and-forget SSE event send.
async fn send_sse_event(tx: &mpsc::Sender<String>, event: QaEvent) {
    let json = match serde_json::to_string(&event) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!(error = %e, "Failed to serialise QaEvent");
            return;
        }
    };
    if tx.send(json).await.is_err() {
        tracing::debug!("SSE channel closed — frontend disconnected");
    }
}

// ---------------------------------------------------------------------------
// FrontendToolStub
// ---------------------------------------------------------------------------

/// A tool stub that delegates execution to the frontend via the
/// [`ToolResultBroker`]. When called:
/// 1. Generates a unique `call_id`
/// 2. Registers with the broker (creates a oneshot channel)
/// 3. Emits `ToolCallStarted` SSE event
/// 4. Blocks until the frontend POSTs the result or timeout elapses
#[derive(Clone)]
pub struct FrontendToolStub {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub broker: Arc<dyn ToolResultBroker>,
    pub sse_tx: mpsc::Sender<String>,
}

impl Tool for FrontendToolStub {
    const NAME: &'static str = "frontend_tool_stub";

    type Error = FrontendToolError;
    type Args = FrontendToolArgs;
    type Output = String;

    /// Override the default name-based routing with the dynamic name.
    fn name(&self) -> String {
        self.name.clone()
    }

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }

    #[tracing::instrument(skip(self, args))]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 1. Generate unique call_id
        let call_id = format!("page-{}", uuid::Uuid::now_v7().simple());

        tracing::info!(
            tool = %self.name,
            call_id = %call_id,
            args_len = args.0.to_string().len(),
            "FrontendToolStub call() START"
        );

        // 2. Register with broker
        let receiver: ToolResultReceiver =
            self.broker.register(call_id.clone()).await.map_err(|e| {
                tracing::error!(
                    tool = %self.name,
                    call_id = %call_id,
                    error = %e,
                    "FrontendToolStub broker register FAILED"
                );
                FrontendToolError::Broker(e.to_string())
            })?;

        tracing::info!(
            tool = %self.name,
            call_id = %call_id,
            "FrontendToolStub registered with broker, emitting ToolCallStarted SSE"
        );

        // 3. Emit ToolCallStarted SSE event with the actual arguments from the LLM
        send_sse_event(
            &self.sse_tx,
            QaEvent::ToolCallStarted {
                tool_name: self.name.clone(),
                tool_call_id: call_id.clone(),
                arguments: args.0,
            },
        )
        .await;

        // 4. Determine timeout
        let timeout = tool_timeout(&self.name);

        tracing::info!(
            tool = %self.name,
            call_id = %call_id,
            timeout_secs = timeout.as_secs(),
            "FrontendToolStub awaiting result..."
        );

        // 5. Await result with timeout
        match tokio::time::timeout(timeout, receiver.recv()).await {
            Ok(Ok(result)) => {
                if result.is_error {
                    tracing::warn!(
                        tool = %self.name,
                        call_id = %call_id,
                        "FrontendToolStub received ERROR result from frontend"
                    );
                    Err(FrontendToolError::Internal(result.output))
                } else {
                    tracing::info!(
                        tool = %self.name,
                        call_id = %call_id,
                        output_len = result.output.len(),
                        "FrontendToolStub received SUCCESS result"
                    );
                    Ok(result.output)
                }
            }
            Ok(Err(e)) => {
                // Receiver error (closed, etc.)
                tracing::error!(
                    tool = %self.name,
                    call_id = %call_id,
                    error = %e,
                    "FrontendToolStub receiver error"
                );
                self.broker.cleanup(&call_id).await;
                Err(e.into())
            }
            Err(_) => {
                // Timeout elapsed
                tracing::error!(
                    tool = %self.name,
                    call_id = %call_id,
                    timeout_secs = timeout.as_secs(),
                    "FrontendToolStub TIMEOUT"
                );
                self.broker.cleanup(&call_id).await;
                Err(FrontendToolError::Timeout(format!(
                    "tool {} timed out after {}s",
                    self.name,
                    timeout.as_secs()
                )))
            }
        }
    }
}
