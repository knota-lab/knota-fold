use async_trait::async_trait;
use loco_rs::{
    app::AppContext,
    task::{self, Task, TaskInfo},
    Result,
};

use crate::services::task_scheduler_service;

pub struct SchedulerDispatch;

#[async_trait]
impl Task for SchedulerDispatch {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "scheduler_dispatch".to_string(),
            detail: "元任务：查询到期调度计划并分发到 Worker 队列".to_string(),
        }
    }

    async fn run(&self, app_context: &AppContext, _vars: &task::Vars) -> Result<()> {
        if let Err(e) = task_scheduler_service::tick(app_context).await {
            tracing::error!(error = %e, "scheduler_dispatch tick failed");
        }
        Ok(())
    }
}
