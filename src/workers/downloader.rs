use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

pub struct DownloadWorker {
    pub ctx: AppContext,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct DownloadWorkerArgs {
    pub user_guid: String,
    /// Propagated trace_id from the enqueuing request for cross-process tracing.
    pub trace_id: String,
    /// Parent span ID from the enqueuing context (optional, for span linking).
    pub parent_span_id: Option<String>,
}

#[async_trait]
impl BackgroundWorker<DownloadWorkerArgs> for DownloadWorker {
    fn build(ctx: &AppContext) -> Self {
        Self { ctx: ctx.clone() }
    }
    async fn perform(&self, _args: DownloadWorkerArgs) -> Result<()> {
        // TODO: Some actual work goes here...

        Ok(())
    }
}
