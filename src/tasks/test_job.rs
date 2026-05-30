use async_trait::async_trait;
use loco_rs::{
    app::AppContext,
    task::{self, Task, TaskInfo},
    Result,
};

/// Demo loco Task for CLI testing. Scheduled worker execution lives
/// in `workers/test_job_worker.rs`.
pub struct TestJob;

#[async_trait]
impl Task for TestJob {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "test_job".to_string(),
            detail: "测试任务：模拟多步作业，输出日志并 sleep".to_string(),
        }
    }

    async fn run(&self, _ctx: &AppContext, _vars: &task::Vars) -> Result<()> {
        use std::time::Duration;

        tracing::info!(step = 1, "test_job started");
        tokio::time::sleep(Duration::from_secs(2)).await;

        tracing::info!(step = 2, "test_job processing batch 1/3");
        tokio::time::sleep(Duration::from_secs(1)).await;

        tracing::info!(step = 3, "test_job processing batch 2/3");
        tokio::time::sleep(Duration::from_secs(1)).await;

        tracing::info!(step = 4, "test_job processing batch 3/3");
        tokio::time::sleep(Duration::from_secs(1)).await;

        tracing::info!(step = 5, "test_job finished");
        Ok(())
    }
}
