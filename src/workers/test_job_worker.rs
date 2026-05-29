use std::time::Duration;

use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::Instrument;
use uuid::Uuid;

use crate::services::task_execution_service;

pub struct TestJobWorker {
    pub ctx: AppContext,
}

#[derive(Deserialize, Debug, Serialize, Clone)]
pub struct TestJobWorkerArgs {
    pub execution_id: Uuid,
    pub worker_code: String,
    pub tenant_id: Uuid,
    pub params_json: Option<String>,
    pub retry_count: i32,
    pub trace_id: Option<String>,
    pub parent_span_id: Option<String>,
}

#[async_trait]
impl BackgroundWorker<TestJobWorkerArgs> for TestJobWorker {
    fn build(ctx: &AppContext) -> Self {
        Self { ctx: ctx.clone() }
    }

    async fn perform(&self, args: TestJobWorkerArgs) -> Result<()> {
        let trace_id_str = args.trace_id.as_deref().unwrap_or("untraced");
        let parent_span_id_str = args.parent_span_id.as_deref().unwrap_or("");
        let span = tracing::info_span!(
            "test_job_worker",
            execution_id = %args.execution_id,
            worker_code = %args.worker_code,
            trace_id = %trace_id_str,
            parent_span_id = %parent_span_id_str,
        );

        let ctx = self.ctx.clone();
        let execution_id = args.execution_id;
        let retry_count = args.retry_count;
        let worker_code = args.worker_code.clone();

        task_execution_service::run_with_status_tracking(
            &ctx,
            execution_id,
            retry_count,
            worker_code,
            || async move {
                tracing::info!(
                    execution_id = %args.execution_id,
                    worker_code = %args.worker_code,
                    tenant_id = %args.tenant_id,
                    params = ?args.params_json,
                    retry_count = args.retry_count,
                    trace_id = ?args.trace_id,
                    "test_job started"
                );

                tracing::info!(step = 1, "step one: initialization");
                tokio::time::sleep(Duration::from_secs(2)).await;

                tracing::warn!(step = 2, "step two: processing batch 1/3");
                tokio::time::sleep(Duration::from_secs(1)).await;

                tracing::warn!(step = 3, "step three: processing batch 2/3");
                tokio::time::sleep(Duration::from_secs(1)).await;

                tracing::error!(
                    step = 4,
                    "step four: simulated warning (error level log)"
                );
                tokio::time::sleep(Duration::from_secs(1)).await;

                tracing::info!(step = 5, "step five: finalization");

                Ok(format!(
                    "completed: execution_id={}, steps=5, batches=3",
                    args.execution_id
                ))
            },
        )
        .instrument(span)
        .await
    }
}
