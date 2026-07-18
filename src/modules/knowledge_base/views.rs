use chrono::Utc;

use crate::models::_entities::{kb_documents, kb_folders, kb_libraries, worker_runs};
use crate::services::worker_run_service::{
    WorkerRunDefinition, KNOWLEDGE_BASE_INDEXING_RUN_DEFINITION, STATUS_CANCELLED,
    STATUS_FAILED, STATUS_SUCCEEDED,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

// ---- Request types ----

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateDocumentRequest {
    pub title: String,
    pub description: Option<String>,
    pub library_id: Option<uuid::Uuid>,
    pub folder_id: Option<uuid::Uuid>,
    /// MIME type observed by the business flow. For file-backed documents it is
    /// copied into the file reference snapshot instead of mutating `files`.
    /// Defaults to `text/plain` when inline `content` is present.
    pub source_type: Option<String>,
    /// Document visibility: "private" (only uploader) or "tenant" (shared in tenant).
    /// Defaults to "tenant" when omitted.
    pub scope: Option<String>,
    pub file_id: Option<uuid::Uuid>,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct DocumentListQuery {
    pub page: Option<u64>,
    pub page_size: Option<u64>,
    pub library_id: Option<uuid::Uuid>,
    pub folder_id: Option<uuid::Uuid>,
    pub status: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    pub limit: Option<usize>,
    pub library_id: Option<uuid::Uuid>,
    pub folder_id: Option<uuid::Uuid>,
    pub document_ids: Option<Vec<uuid::Uuid>>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PresignDocumentAssetsRequest {
    pub asset_keys: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateLibraryRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLibraryRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct FolderListQuery {
    pub library_id: uuid::Uuid,
    pub parent_id: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderRequest {
    pub library_id: uuid::Uuid,
    pub parent_id: Option<uuid::Uuid>,
    pub name: String,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFolderRequest {
    pub name: String,
    #[serde(default)]
    pub sort_order: i32,
}

// ---- Response types ----

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndexingProgressResponse {
    pub stage: String,
    pub label: String,
    pub message: Option<String>,
    pub current: Option<i32>,
    pub total: Option<i32>,
    pub stage_started_at: Option<String>,
    pub heartbeat_at: Option<String>,
    pub health: String,
    pub is_stale: bool,
    pub is_hard_timeout: bool,
    pub stale_reason: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentResponse {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub library_id: Option<String>,
    pub folder_id: Option<String>,
    pub source_type: String,
    pub scope: String,
    pub file_id: Option<String>,
    pub file_reference_id: Option<String>,
    pub status: String,
    pub chunk_count: i32,
    pub total_tokens: i32,
    pub indexing_progress: Option<IndexingProgressResponse>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reused_existing: Option<bool>,
}

impl DocumentResponse {
    #[must_use]
    pub fn from_model(m: &kb_documents::Model) -> Self {
        Self {
            id: m.id.to_string(),
            title: m.title.clone(),
            description: m.description.clone(),
            library_id: m.library_id.map(|id| id.to_string()),
            folder_id: m.folder_id.map(|id| id.to_string()),
            source_type: m.source_type.clone(),
            scope: m.scope.clone(),
            file_id: m.file_id.map(|id| id.to_string()),
            file_reference_id: m.file_reference_id.map(|id| id.to_string()),
            status: m.status.clone(),
            chunk_count: m.chunk_count,
            total_tokens: m.total_tokens,
            indexing_progress: indexing_progress_from_metadata(m.metadata.as_ref()),
            error_message: m.error_message.clone(),
            created_at: m.created_at.and_utc().to_rfc3339(),
            updated_at: m.updated_at.and_utc().to_rfc3339(),
            reused_existing: None,
        }
    }

    #[must_use]
    pub fn from_reused_model(m: &kb_documents::Model) -> Self {
        Self {
            reused_existing: Some(true),
            ..Self::from_model(m)
        }
    }

    #[must_use]
    pub fn from_model_with_worker_run(
        m: &kb_documents::Model,
        worker_run: Option<&worker_runs::Model>,
    ) -> Self {
        let mut response = Self::from_model(m);
        if let Some(run) = worker_run {
            response.indexing_progress = indexing_progress_from_worker_run(
                run,
                KNOWLEDGE_BASE_INDEXING_RUN_DEFINITION,
            );
        }
        response
    }
}

fn indexing_progress_from_metadata(
    metadata: Option<&serde_json::Value>,
) -> Option<IndexingProgressResponse> {
    let indexing = metadata?.get("indexing")?;
    let stage = indexing.get("stage")?.as_str()?.to_string();
    let label = indexing
        .get("label")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&stage)
        .to_string();
    let message = indexing
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(std::string::ToString::to_string);
    let stage_started_at = indexing
        .get("stageStartedAt")
        .and_then(serde_json::Value::as_str)
        .map(std::string::ToString::to_string);
    let current = indexing
        .get("current")
        .and_then(serde_json::Value::as_i64)
        .and_then(|value| i32::try_from(value).ok());
    let total = indexing
        .get("total")
        .and_then(serde_json::Value::as_i64)
        .and_then(|value| i32::try_from(value).ok());

    Some(IndexingProgressResponse {
        stage,
        label,
        message,
        current,
        total,
        stage_started_at,
        heartbeat_at: None,
        health: "normal".to_string(),
        is_stale: false,
        is_hard_timeout: false,
        stale_reason: None,
    })
}

fn indexing_progress_from_worker_run(
    run: &worker_runs::Model,
    definition: WorkerRunDefinition,
) -> Option<IndexingProgressResponse> {
    let stage = run.stage.clone()?;
    let stage_definition = definition.stage(&stage);
    let label = run
        .stage_label
        .clone()
        .or_else(|| stage_definition.map(|stage| stage.label.to_string()))
        .unwrap_or_else(|| stage.clone());
    let now = Utc::now().naive_utc();
    let derived = derive_worker_run_health(
        run.status.as_str(),
        stage_definition,
        run.heartbeat_at,
        run.stage_started_at,
        now,
    );
    let stale_reason = stale_reason(
        &label,
        derived.is_stale,
        derived.is_hard_timeout,
        stage_definition.and_then(|stage| stage.hard_timeout),
        stage_definition.map(|stage| stage.stale_after),
    );

    Some(IndexingProgressResponse {
        stage,
        label,
        message: run.message.clone(),
        current: run.current,
        total: run.total,
        stage_started_at: run.stage_started_at.map(|time| time.and_utc().to_rfc3339()),
        heartbeat_at: run.heartbeat_at.map(|time| time.and_utc().to_rfc3339()),
        health: derived.health.to_string(),
        is_stale: derived.is_stale,
        is_hard_timeout: derived.is_hard_timeout,
        stale_reason,
    })
}

struct DerivedWorkerRunHealth {
    health: &'static str,
    is_stale: bool,
    is_hard_timeout: bool,
}

fn derive_worker_run_health(
    status: &str,
    stage_definition: Option<
        &crate::services::worker_run_service::WorkerRunStageDefinition,
    >,
    heartbeat_at: Option<chrono::NaiveDateTime>,
    stage_started_at: Option<chrono::NaiveDateTime>,
    now: chrono::NaiveDateTime,
) -> DerivedWorkerRunHealth {
    let status_finished =
        matches!(status, STATUS_SUCCEEDED | STATUS_FAILED | STATUS_CANCELLED);
    let is_stale = !status_finished
        && stage_definition.is_some_and(|stage| {
            heartbeat_at.is_some_and(|heartbeat_at| {
                (now - heartbeat_at).num_seconds() > duration_secs(stage.stale_after)
            })
        });
    let is_hard_timeout = !status_finished
        && stage_definition.is_some_and(|stage| {
            stage.hard_timeout.is_some_and(|timeout| {
                stage_started_at.is_some_and(|stage_started_at| {
                    (now - stage_started_at).num_seconds() > duration_secs(timeout)
                })
            })
        });
    let health = if status_finished {
        "finished"
    } else if is_hard_timeout {
        "timeout"
    } else if is_stale {
        "stale"
    } else {
        "normal"
    };
    DerivedWorkerRunHealth {
        health,
        is_stale,
        is_hard_timeout,
    }
}

fn stale_reason(
    label: &str,
    is_stale: bool,
    is_hard_timeout: bool,
    hard_timeout: Option<std::time::Duration>,
    stale_after: Option<std::time::Duration>,
) -> Option<String> {
    if is_hard_timeout {
        return Some(format!(
            "{label}阶段超过{}分钟仍未完成",
            seconds_to_minutes(duration_secs(hard_timeout.unwrap_or_default()))
        ));
    }
    if is_stale {
        return Some(format!(
            "{label}阶段{}分钟无进展",
            seconds_to_minutes(duration_secs(stale_after.unwrap_or_default()))
        ));
    }
    None
}

fn duration_secs(duration: std::time::Duration) -> i64 {
    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
}

const fn seconds_to_minutes(seconds: i64) -> i64 {
    (seconds + 59) / 60
}

#[cfg(test)]
mod tests {
    use chrono::Duration as ChronoDuration;

    use super::*;
    use crate::services::worker_run_service::{
        KNOWLEDGE_BASE_INDEXING_RUN_DEFINITION, STATUS_RUNNING,
    };

    #[test]
    fn worker_run_health_is_normal_before_stale_threshold() {
        let now = Utc::now().naive_utc();
        let stage = KNOWLEDGE_BASE_INDEXING_RUN_DEFINITION
            .stage("embedding")
            .expect("embedding stage");
        let health = derive_worker_run_health(
            STATUS_RUNNING,
            Some(stage),
            Some(now - ChronoDuration::minutes(1)),
            Some(now - ChronoDuration::minutes(1)),
            now,
        );

        assert_eq!(health.health, "normal");
        assert!(!health.is_stale);
        assert!(!health.is_hard_timeout);
    }

    #[test]
    fn worker_run_health_marks_stale_after_heartbeat_threshold() {
        let now = Utc::now().naive_utc();
        let stage = KNOWLEDGE_BASE_INDEXING_RUN_DEFINITION
            .stage("embedding")
            .expect("embedding stage");
        let health = derive_worker_run_health(
            STATUS_RUNNING,
            Some(stage),
            Some(now - ChronoDuration::minutes(6)),
            Some(now - ChronoDuration::minutes(6)),
            now,
        );

        assert_eq!(health.health, "stale");
        assert!(health.is_stale);
        assert!(!health.is_hard_timeout);
    }

    #[test]
    fn worker_run_health_marks_timeout_after_stage_threshold() {
        let now = Utc::now().naive_utc();
        let stage = KNOWLEDGE_BASE_INDEXING_RUN_DEFINITION
            .stage("chunk")
            .expect("chunk stage");
        let health = derive_worker_run_health(
            STATUS_RUNNING,
            Some(stage),
            Some(now - ChronoDuration::minutes(1)),
            Some(now - ChronoDuration::minutes(11)),
            now,
        );

        assert_eq!(health.health, "timeout");
        assert!(!health.is_stale);
        assert!(health.is_hard_timeout);
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LibraryResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

impl LibraryResponse {
    #[must_use]
    pub fn from_model(m: &kb_libraries::Model) -> Self {
        Self {
            id: m.id.to_string(),
            name: m.name.clone(),
            description: m.description.clone(),
            sort_order: m.sort_order,
            created_by: m.created_by.to_string(),
            created_at: m.created_at.and_utc().to_rfc3339(),
            updated_at: m.updated_at.and_utc().to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FolderResponse {
    pub id: String,
    pub library_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub path: String,
    pub depth: i32,
    pub sort_order: i32,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

impl FolderResponse {
    #[must_use]
    pub fn from_model(m: &kb_folders::Model) -> Self {
        Self {
            id: m.id.to_string(),
            library_id: m.library_id.to_string(),
            parent_id: m.parent_id.map(|id| id.to_string()),
            name: m.name.clone(),
            path: m.path.clone(),
            depth: m.depth,
            sort_order: m.sort_order,
            created_by: m.created_by.to_string(),
            created_at: m.created_at.and_utc().to_rfc3339(),
            updated_at: m.updated_at.and_utc().to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultResponse {
    pub chunk_id: String,
    pub document_id: String,
    pub content: String,
    pub heading_path: Option<String>,
    pub score: f64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChunkResponse {
    pub id: String,
    pub document_id: String,
    pub chunk_index: i32,
    pub content: String,
    pub heading_path: Option<String>,
    pub token_count: i32,
    pub char_start: Option<i32>,
    pub char_end: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentAssetResponse {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub storage_key: String,
    pub size: usize,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPreviewResponse {
    pub document_id: String,
    pub title: String,
    pub markdown: String,
    pub assets: Vec<DocumentAssetResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PresignedDocumentAssetResponse {
    pub asset_key: String,
    pub url: String,
    pub expires_in: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PresignDocumentAssetsResponse {
    pub items: Vec<PresignedDocumentAssetResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MutationSuccessResponse {
    pub success: bool,
}
