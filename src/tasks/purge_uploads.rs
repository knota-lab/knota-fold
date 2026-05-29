use async_trait::async_trait;
use loco_rs::{
    app::AppContext,
    task::{self, Task, TaskInfo},
    Result,
};

use crate::services::file_upload_service;

pub struct PurgeUploads;

#[async_trait]
impl Task for PurgeUploads {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "purge_uploads".to_string(),
            detail: "T+24h soft tombstone + T+7d hard purge for multipart uploads"
                .to_string(),
        }
    }

    async fn run(&self, app_context: &AppContext, _vars: &task::Vars) -> Result<()> {
        let outcome = file_upload_service::purge_uploads(app_context).await?;
        tracing::info!(
            soft_deleted = outcome.soft_deleted,
            hard_deleted = outcome.hard_deleted,
            "purge_uploads completed"
        );
        Ok(())
    }
}
