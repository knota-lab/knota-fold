use std::future::Future;
use std::time::Duration;

use axum::http::StatusCode;
use chrono::Utc;
use loco_rs::{
    app::AppContext, bgworker::BackgroundWorker, controller::ErrorDetail, Error, Result,
};

use crate::config::ConfigExt;
use crate::models::{scheduled_worker_definitions, scheduled_worker_executions};
use crate::workers::test_job_worker::{TestJobWorker, TestJobWorkerArgs};

pub async fn run_with_status_tracking<F, Fut>(
    ctx: &AppContext,
    execution_id: uuid::Uuid,
    retry_count: i32,
    worker_code: String,
    work_fn: F,
) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let db = &ctx.db;

    let execution = scheduled_worker_executions::Model::find_by_id(db, execution_id)
        .await?
        .ok_or_else(|| {
            Error::CustomError(
                StatusCode::NOT_FOUND,
                ErrorDetail::new("worker.execution_not_found", "执行记录未找到"),
            )
        })?;

    if execution.retry_count != retry_count {
        let desc = format!(
            "retry_count mismatch for execution {execution_id}: db={}, args={retry_count}",
            execution.retry_count
        );
        return Err(Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("worker.retry_count_mismatch", &desc),
        ));
    }

    let started_at = Utc::now().fixed_offset();
    scheduled_worker_executions::Model::update_status(
        db,
        execution_id,
        &scheduled_worker_executions::UpdateStatusParams {
            status: "running".to_string(),
            started_at: Some(started_at),
            finished_at: None,
            duration_ms: None,
            output: None,
            error_message: None,
            traceparent: None,
        },
    )
    .await?;

    let worker_def = scheduled_worker_definitions::Model::find_by_code(db, &worker_code)
        .await?
        .ok_or_else(|| {
            let desc = format!("worker def not found: {worker_code}");
            Error::CustomError(
                StatusCode::NOT_FOUND,
                ErrorDetail::new("worker.def_not_found", &desc),
            )
        })?;

    let timeout_secs = worker_def.timeout_secs.max(1) as u64;
    let max_retries = worker_def.max_retries;

    let result = tokio::select! {
        r = work_fn() => r,
        _ = tokio::time::sleep(Duration::from_secs(timeout_secs)) => {
            let finished_at = Utc::now().fixed_offset();
            let duration_ms = (finished_at - started_at).num_milliseconds() as i32;
            scheduled_worker_executions::Model::update_status(
                db,
                execution_id,
                &scheduled_worker_executions::UpdateStatusParams {
                    status: "timeout".to_string(),
                    started_at: None,
                    finished_at: Some(finished_at),
                    duration_ms: Some(duration_ms),
                    output: None,
                    error_message: Some("execution timed out".to_string()),
                    traceparent: None,
                },
            ).await?;
            return Ok(());
        }
    };

    let finished_at = Utc::now().fixed_offset();
    let duration_ms = (finished_at - started_at).num_milliseconds() as i32;

    match result {
        Ok(output) => {
            let truncated = truncate_output(&output, get_output_max_bytes(ctx));
            scheduled_worker_executions::Model::update_status(
                db,
                execution_id,
                &scheduled_worker_executions::UpdateStatusParams {
                    status: "success".to_string(),
                    started_at: None,
                    finished_at: Some(finished_at),
                    duration_ms: Some(duration_ms),
                    output: Some(truncated),
                    error_message: None,
                    traceparent: None,
                },
            )
            .await?;
        }
        Err(e) => {
            handle_failure(
                ctx,
                execution_id,
                &e.to_string(),
                max_retries,
                &worker_code,
                retry_count,
            )
            .await?;
        }
    }

    Ok(())
}

fn get_output_max_bytes(ctx: &AppContext) -> usize {
    ctx.config
        .typed_settings()
        .ok()
        .flatten()
        .map_or(65_536, |s| s.output_max_bytes())
}

fn truncate_output(output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        output.to_string()
    } else {
        let mut end = max_bytes;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        let mut truncated = output[..end].to_string();
        truncated.push_str("\n... [truncated]");
        truncated
    }
}

async fn handle_failure(
    ctx: &AppContext,
    execution_id: uuid::Uuid,
    error: &str,
    max_retries: i32,
    worker_code: &str,
    retry_count: i32,
) -> Result<()> {
    let db = &ctx.db;
    let execution = scheduled_worker_executions::Model::find_by_id(db, execution_id)
        .await?
        .ok_or_else(|| {
            Error::CustomError(
                StatusCode::NOT_FOUND,
                ErrorDetail::new("worker.execution_not_found", "执行记录未找到"),
            )
        })?;
    let new_retry_count = retry_count + 1;

    scheduled_worker_executions::Model::set_retry_count(
        db,
        execution_id,
        new_retry_count,
    )
    .await?;

    if new_retry_count < max_retries {
        tracing::warn!(
            execution_id = %execution_id,
            worker_code = worker_code,
            retry_count = new_retry_count,
            "retrying execution"
        );

        if worker_code == "test_job" {
            TestJobWorker::perform_later(
                ctx,
                TestJobWorkerArgs {
                    execution_id,
                    worker_code: worker_code.to_string(),
                    tenant_id: execution.tenant_id,
                    params_json: execution.params_json.clone(),
                    retry_count: new_retry_count,
                    trace_id: execution.traceparent.clone(),
                    parent_span_id: execution.parent_span_id.clone(),
                },
            )
            .await?;
        } else {
            // TODO: match on worker_code when more workers are added
            let finished_at = Utc::now().fixed_offset();
            scheduled_worker_executions::Model::update_status(
                db,
                execution_id,
                &scheduled_worker_executions::UpdateStatusParams {
                    status: "failed".to_string(),
                    started_at: None,
                    finished_at: Some(finished_at),
                    duration_ms: None,
                    output: None,
                    error_message: Some(format!("unsupported retry worker: {worker_code}; original error: {error}")),
                    traceparent: None,
                },
            )
            .await?;
        }
    } else {
        let finished_at = Utc::now().fixed_offset();
        scheduled_worker_executions::Model::update_status(
            db,
            execution_id,
            &scheduled_worker_executions::UpdateStatusParams {
                status: "failed".to_string(),
                started_at: None,
                finished_at: Some(finished_at),
                duration_ms: None,
                output: None,
                error_message: Some(error.to_string()),
                traceparent: None,
            },
        )
        .await?;
    }

    Ok(())
}
