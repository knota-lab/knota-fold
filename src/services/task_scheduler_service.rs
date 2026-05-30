use axum::http::StatusCode;
use chrono::{Duration, Utc};
use cron::Schedule;
use loco_rs::{
    app::AppContext, bgworker::BackgroundWorker, controller::ErrorDetail, Error, Result,
};
use sea_orm::{DatabaseConnection, EntityTrait};

use crate::config::ConfigExt;
use crate::models::{
    scheduled_worker_definitions, scheduled_worker_executions,
    scheduled_worker_schedules, scheduled_worker_tenant_grants, tenants,
};
use crate::views::errors::err_custom;

/// Main tick function called every minute by `scheduler_dispatch` task.
pub async fn tick(ctx: &AppContext) -> Result<()> {
    if !is_scheduler_enabled(ctx) {
        tracing::debug!("task scheduler disabled; skipping tick");
        return Ok(());
    }

    let now = Utc::now().fixed_offset();
    let db = &ctx.db;

    recover_zombies(db).await?;

    let due_schedules = scheduled_worker_schedules::Model::find_due(db, now).await?;
    let max_concurrent = get_max_concurrent(ctx);

    for schedule in due_schedules {
        if let Err(e) = process_schedule(ctx, db, &schedule, max_concurrent).await {
            tracing::error!(schedule_id = %schedule.id, error = %e, "failed to process schedule");
        }
    }

    Ok(())
}

async fn process_schedule(
    ctx: &AppContext,
    db: &DatabaseConnection,
    schedule: &scheduled_worker_schedules::Model,
    max_concurrent: i32,
) -> Result<()> {
    let worker_def = match scheduled_worker_definitions::Entity::find_by_id(
        schedule.worker_def_id,
    )
    .one(db)
    .await?
    {
        Some(d) if d.status == "active" => d,
        _ => {
            tracing::info!(schedule_id = %schedule.id, "skipping: worker def not found or inactive");
            return Ok(());
        }
    };

    if !validate_schedule_grant(db, schedule).await? {
        return Ok(());
    }

    let tenant_id = schedule.tenant_id;

    if !worker_def.allow_concurrent {
        let running = scheduled_worker_executions::Model::find_running_for_schedule(
            db,
            schedule.id,
        )
        .await?;
        if !running.is_empty() {
            tracing::warn!(schedule_id = %schedule.id, "skipping: concurrent execution in progress");
            return Ok(());
        }
    }

    let tenant_running =
        scheduled_worker_executions::Model::count_running_for_tenant(db, tenant_id)
            .await?;
    if tenant_running >= max_concurrent as u64 {
        tracing::warn!(tenant_id = %tenant_id, limit = max_concurrent, "skipping: tenant concurrent limit reached");
        return Ok(());
    }

    let execution = scheduled_worker_executions::Model::create_pending(
        db,
        &scheduled_worker_executions::CreateExecutionParams {
            schedule_id: schedule.id,
            worker_def_id: worker_def.id,
            tenant_id,
            trigger_type: "scheduled".to_string(),
            params_json: schedule.params_json.clone(),
            triggered_by: None,
            traceparent: None,
            parent_span_id: None,
        },
    )
    .await?;

    advance_next_run(db, schedule).await?;

    dispatch_or_skip(
        ctx,
        db,
        &worker_def,
        execution,
        tenant_id,
        schedule.params_json.as_ref(),
    )
    .await
}

async fn validate_schedule_grant(
    db: &DatabaseConnection,
    schedule: &scheduled_worker_schedules::Model,
) -> Result<bool> {
    let grant = scheduled_worker_tenant_grants::Model::find_granted(
        db,
        schedule.worker_def_id,
        schedule.tenant_id,
    )
    .await?;
    if grant.is_none() {
        tracing::info!(schedule_id = %schedule.id, "skipping: no grant");
        return Ok(false);
    }
    let tenant_active = match tenants::Model::find_by_id(db, schedule.tenant_id).await {
        Ok(tenant) => tenant.status == "active",
        Err(e) => {
            tracing::warn!(schedule_id = %schedule.id, error = %e, "skipping: tenant query failed");
            return Ok(false);
        }
    };
    if !tenant_active {
        tracing::info!(schedule_id = %schedule.id, "skipping: tenant inactive");
        return Ok(false);
    }
    Ok(true)
}

async fn advance_next_run(
    db: &DatabaseConnection,
    schedule: &scheduled_worker_schedules::Model,
) -> Result<()> {
    let next = compute_next_run(&schedule.cron_expr, Utc::now())?;
    scheduled_worker_schedules::Model::update_next_run_at(
        db,
        schedule.id,
        Some(next.fixed_offset()),
    )
    .await?;
    Ok(())
}

async fn dispatch_or_skip(
    ctx: &AppContext,
    db: &DatabaseConnection,
    worker_def: &scheduled_worker_definitions::Model,
    execution: scheduled_worker_executions::Model,
    tenant_id: uuid::Uuid,
    params_json: Option<&String>,
) -> Result<()> {
    let result = if worker_def.code.as_str() == "test_job" {
        use crate::workers::test_job_worker::{TestJobWorker, TestJobWorkerArgs};

        TestJobWorker::perform_later(
            ctx,
            TestJobWorkerArgs {
                execution_id: execution.id,
                worker_code: worker_def.code.clone(),
                tenant_id,
                params_json: params_json.cloned(),
                retry_count: 0,
                trace_id: None,
                parent_span_id: None,
            },
        )
        .await
    } else {
        tracing::error!(worker_code = %worker_def.code, "unknown worker code");
        mark_execution_skipped_unknown(db, execution.id, &worker_def.code).await?;
        return Ok(());
    };

    if let Err(e) = result {
        tracing::error!(execution_id = %execution.id, error = %e, "failed to enqueue worker");
        mark_execution_skipped_enqueue_failed(db, execution.id, &e).await?;
    }

    Ok(())
}

async fn mark_execution_skipped_unknown(
    db: &DatabaseConnection,
    execution_id: uuid::Uuid,
    worker_code: &str,
) -> Result<()> {
    scheduled_worker_executions::Model::update_status(
        db,
        execution_id,
        &scheduled_worker_executions::UpdateStatusParams {
            status: "skipped".to_string(),
            started_at: None,
            finished_at: None,
            duration_ms: None,
            output: None,
            error_message: Some(format!("unknown worker code: {worker_code}")),
            traceparent: None,
        },
    )
    .await?;
    Ok(())
}

async fn mark_execution_skipped_enqueue_failed(
    db: &DatabaseConnection,
    execution_id: uuid::Uuid,
    error: &loco_rs::Error,
) -> Result<()> {
    scheduled_worker_executions::Model::update_status(
        db,
        execution_id,
        &scheduled_worker_executions::UpdateStatusParams {
            status: "skipped".to_string(),
            started_at: None,
            finished_at: None,
            duration_ms: None,
            output: None,
            error_message: Some(format!("enqueue failed: {error}")),
            traceparent: None,
        },
    )
    .await?;
    Ok(())
}

/// Recover zombie executions stuck in 'running' state.
async fn recover_zombies(db: &DatabaseConnection) -> Result<()> {
    let cutoff = (Utc::now() - Duration::hours(2)).fixed_offset();
    let zombies = scheduled_worker_executions::Model::find_zombies(db, cutoff).await?;

    for zombie in zombies {
        tracing::warn!(execution_id = %zombie.id, "recovering zombie execution");
        scheduled_worker_executions::Model::update_status(
            db,
            zombie.id,
            &scheduled_worker_executions::UpdateStatusParams {
                status: "failed".to_string(),
                started_at: None,
                finished_at: Some(Utc::now().fixed_offset()),
                duration_ms: None,
                output: None,
                error_message: Some("worker crash suspected".to_string()),
                traceparent: None,
            },
        )
        .await?;
    }

    Ok(())
}

fn is_scheduler_enabled(ctx: &AppContext) -> bool {
    ctx.config
        .typed_settings()
        .ok()
        .flatten()
        .is_none_or(|s| s.scheduler_enabled())
}

fn get_max_concurrent(ctx: &AppContext) -> i32 {
    ctx.config
        .typed_settings()
        .ok()
        .flatten()
        .map_or(3, |s| s.max_concurrent_per_tenant())
}

pub fn compute_next_run(
    cron_expr: &str,
    after: chrono::DateTime<Utc>,
) -> Result<chrono::DateTime<Utc>> {
    let schedule: Schedule = cron_expr.parse::<Schedule>().map_err(|e| {
        let desc = e.to_string();
        Error::CustomError(
            StatusCode::BAD_REQUEST,
            ErrorDetail::new("worker.invalid_cron", &desc),
        )
    })?;

    schedule.after(&after).next().ok_or_else(|| {
        err_custom(
            StatusCode::BAD_REQUEST,
            "worker.no_future_runs",
            &format!("no future runs for cron: {cron_expr}"),
        )
    })
}
