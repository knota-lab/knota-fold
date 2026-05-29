use async_trait::async_trait;
use loco_rs::{
    app::AppContext,
    task::{self, Task, TaskInfo},
    Result,
};

use crate::services::file_service;

pub struct PurgeFiles;

#[async_trait]
impl Task for PurgeFiles {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "purge_files".to_string(),
            detail: "物理清理 status=DELETED + purge_at <= now()".to_string(),
        }
    }

    async fn run(&self, app_context: &AppContext, _vars: &task::Vars) -> Result<()> {
        let outcome = file_service::purge_files(app_context).await?;
        tracing::info!(
            purged = outcome.purged,
            errors = outcome.errors,
            "purge_files completed"
        );
        Ok(())
    }
}
