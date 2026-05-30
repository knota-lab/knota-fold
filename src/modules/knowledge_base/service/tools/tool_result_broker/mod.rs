//! Tool result broker for frontend-executed tools.
//!
//! Provides a rendezvous point where the backend's `FrontendToolStub` blocks
//! on a oneshot channel until the frontend POSTs the result back.

pub mod in_process;

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

// ---------------------------------------------------------------------------
// Resolve outcome
// ---------------------------------------------------------------------------

/// Outcome of a `resolve()` call, used for idempotent handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// First resolution — oneshot was consumed successfully.
    Fresh,
    /// Duplicate resolution — same `call_id` was already resolved.
    AlreadyResolved,
    /// The `call_id` was never registered (or has expired).
    NotFound,
}

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Result returned by the frontend after executing a page tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur when *registering* a pending tool call.
#[derive(Debug)]
pub enum BrokerError {
    /// A pending call with the same `call_id` already exists.
    Conflict(String),
    /// Internal broker failure.
    Internal(String),
}

impl fmt::Display for BrokerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(id) => write!(f, "tool call already pending: {id}"),
            Self::Internal(msg) => write!(f, "broker internal error: {msg}"),
        }
    }
}

impl std::error::Error for BrokerError {}

/// Errors surfaced to the `FrontendToolStub` when waiting for a result.
#[derive(Debug)]
pub enum ReceiverError {
    /// The timeout elapsed before the frontend responded.
    Timeout,
    /// The frontend channel was dropped (server shutdown / cancellation).
    Closed,
    /// Internal failure.
    Internal(String),
}

impl fmt::Display for ReceiverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(f, "tool result timed out"),
            Self::Closed => write!(f, "tool result channel closed"),
            Self::Internal(msg) => write!(f, "receiver internal error: {msg}"),
        }
    }
}

impl std::error::Error for ReceiverError {}

// ---------------------------------------------------------------------------
// Receiver wrapper
// ---------------------------------------------------------------------------

/// Thin wrapper around a `oneshot::Receiver` that maps channel errors to
/// [`ReceiverError`].
pub struct ToolResultReceiver {
    inner: oneshot::Receiver<Result<ToolResult, ReceiverError>>,
}

impl ToolResultReceiver {
    pub(crate) const fn new(
        inner: oneshot::Receiver<Result<ToolResult, ReceiverError>>,
    ) -> Self {
        Self { inner }
    }

    /// Await the tool result from the frontend.
    pub async fn recv(self) -> Result<ToolResult, ReceiverError> {
        match self.inner.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(ReceiverError::Closed),
        }
    }
}

// ---------------------------------------------------------------------------
// Broker trait
// ---------------------------------------------------------------------------

/// Async trait for registering and resolving frontend tool calls.
#[async_trait]
pub trait ToolResultBroker: Send + Sync + 'static {
    /// Register a pending tool call. Returns a [`ToolResultReceiver`] that will
    /// resolve when the frontend posts the result.
    ///
    /// Returns [`BrokerError::Conflict`] if the `call_id` is already pending.
    async fn register(&self, call_id: String) -> Result<ToolResultReceiver, BrokerError>;

    /// Resolve a pending tool call.
    ///
    /// Returns [`ResolveOutcome`] so callers can distinguish between a fresh
    /// resolve, an idempotent duplicate, and an unknown `call_id`.
    async fn resolve(&self, call_id: &str, result: ToolResult) -> ResolveOutcome;

    /// Remove a pending call without resolving it (e.g. on timeout).
    async fn cleanup(&self, call_id: &str);

    /// Number of currently pending (unresolved) tool calls.
    fn pending_count(&self) -> usize;
}
