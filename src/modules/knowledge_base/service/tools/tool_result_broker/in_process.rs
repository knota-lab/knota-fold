//! In-process broker backed by a `DashMap`.

use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::oneshot;

use super::{
    BrokerError, ResolveOutcome, ToolResult, ToolResultBroker, ToolResultReceiver,
};

/// How long a resolved entry is retained for idempotent duplicate detection.
const RESOLVED_TTL: Duration = Duration::from_secs(60);

/// Internal entry stored in the DashMap.
enum BrokerEntry {
    /// Waiting for the frontend to POST the result.
    Pending(oneshot::Sender<Result<ToolResult, super::ReceiverError>>),
    /// Already resolved — kept temporarily for idempotent duplicate detection.
    /// `result` is stored for potential future inspection but currently unused.
    #[allow(dead_code)]
    Resolved { at: Instant, result: ToolResult },
}

/// Concrete in-process broker using `DashMap<String, BrokerEntry>`.
///
/// Suitable for single-instance deployments. The map key is the `call_id`.
/// After a successful resolve the entry transitions from `Pending` to
/// `Resolved` and is retained for [`RESOLVED_TTL`] so that duplicate POSTs
/// from the frontend receive an idempotent 200 instead of a 404.
pub struct InProcessBroker {
    map: DashMap<String, BrokerEntry>,
}

impl Default for InProcessBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl InProcessBroker {
    pub fn new() -> Self {
        Self {
            map: DashMap::new(),
        }
    }
}

#[async_trait::async_trait]
impl ToolResultBroker for InProcessBroker {
    async fn register(&self, call_id: String) -> Result<ToolResultReceiver, BrokerError> {
        // Expire stale Resolved entries and reject Pending conflicts.
        if let Some(entry) = self.map.get_mut(&call_id) {
            match &*entry {
                BrokerEntry::Pending(_) => {
                    tracing::warn!(call_id = %call_id, "Broker register conflict — duplicate call_id");
                    return Err(BrokerError::Conflict(call_id));
                }
                BrokerEntry::Resolved { at, .. } => {
                    if at.elapsed() < RESOLVED_TTL {
                        tracing::warn!(call_id = %call_id, "Broker register conflict — recently resolved");
                        return Err(BrokerError::Conflict(call_id));
                    }
                    // Expired — fall through to insert below.
                }
            }
            drop(entry);
            self.map.remove(&call_id);
        }

        let (tx, rx) = oneshot::channel();
        self.map.insert(call_id.clone(), BrokerEntry::Pending(tx));
        tracing::debug!(
            call_id = %call_id,
            pending = self.pending_count(),
            "Broker registered call_id"
        );

        Ok(ToolResultReceiver::new(rx))
    }

    async fn resolve(&self, call_id: &str, result: ToolResult) -> ResolveOutcome {
        tracing::debug!(
            call_id = %call_id,
            is_error = result.is_error,
            pending = self.pending_count(),
            "Broker resolve attempt"
        );

        // Lazy TTL cleanup: remove any expired Resolved entries on access.
        if let Some(entry) = self.map.get_mut(call_id) {
            match &*entry {
                BrokerEntry::Pending(_) => { /* proceed to resolve below */ }
                BrokerEntry::Resolved { at, .. } if at.elapsed() >= RESOLVED_TTL => {
                    drop(entry);
                    self.map.remove(call_id);
                    tracing::debug!(call_id = %call_id, "Broker resolve — expired Resolved entry removed");
                    return ResolveOutcome::NotFound;
                }
                BrokerEntry::Resolved { .. } => {
                    tracing::debug!(call_id = %call_id, "Broker resolve — AlreadyResolved (idempotent)");
                    return ResolveOutcome::AlreadyResolved;
                }
            }
        }

        if let Some((_, entry)) = self.map.remove(call_id) {
            match entry {
                BrokerEntry::Pending(sender) => {
                    // If the receiver was already dropped, the send fails — that's fine.
                    let _ = sender.send(Ok(result.clone()));
                    // Re-insert as Resolved for duplicate detection.
                    self.map.insert(
                        call_id.to_owned(),
                        BrokerEntry::Resolved {
                            at: Instant::now(),
                            result,
                        },
                    );
                    tracing::debug!(call_id = %call_id, "Broker resolved successfully");
                    ResolveOutcome::Fresh
                }
                BrokerEntry::Resolved { .. } => {
                    // Should not reach here (handled above), but be safe.
                    ResolveOutcome::AlreadyResolved
                }
            }
        } else {
            tracing::warn!(
                call_id = %call_id,
                pending_keys = ?self.map.iter()
                    .filter_map(|e| matches!(e.value(), BrokerEntry::Pending(_)).then(|| e.key().clone()))
                    .collect::<Vec<_>>(),
                "Broker resolve FAILED — call_id not found"
            );
            ResolveOutcome::NotFound
        }
    }

    async fn cleanup(&self, call_id: &str) {
        tracing::debug!(call_id = %call_id, "Broker cleanup");
        self.map.remove(call_id);
    }

    fn pending_count(&self) -> usize {
        self.map
            .iter()
            .filter(|e| matches!(e.value(), BrokerEntry::Pending(_)))
            .count()
    }
}
