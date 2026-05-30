use serde::{Deserialize, Serialize};

use crate::models::_entities::{
    scheduled_worker_definitions, scheduled_worker_executions,
    scheduled_worker_schedules, scheduled_worker_tenant_grants,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerDefinitionResponse {
    pub id: String,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub category: String,
    pub params_schema: Option<String>,
    pub timeout_secs: i32,
    pub max_retries: i32,
    pub allow_concurrent: bool,
    pub is_system: bool,
    pub status: String,
    pub version: i32,
    pub created_at: String,
    pub updated_at: String,
}

impl WorkerDefinitionResponse {
    #[must_use]
    pub fn from_model(m: &scheduled_worker_definitions::Model) -> Self {
        Self {
            id: m.id.to_string(),
            code: m.code.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            category: m.category.clone(),
            params_schema: m.params_schema.clone(),
            timeout_secs: m.timeout_secs,
            max_retries: m.max_retries,
            allow_concurrent: m.allow_concurrent,
            is_system: m.is_system,
            status: m.status.clone(),
            version: m.version,
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkerDefinitionRequest {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub category: String,
    pub params_schema: Option<String>,
    pub timeout_secs: Option<i32>,
    pub max_retries: Option<i32>,
    pub allow_concurrent: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkerDefinitionRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub category: Option<String>,
    pub params_schema: Option<Option<String>>,
    pub timeout_secs: Option<i32>,
    pub max_retries: Option<i32>,
    pub allow_concurrent: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchStatusRequest {
    pub status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerGrantResponse {
    pub id: String,
    pub worker_def_id: String,
    pub tenant_id: String,
    pub granted_by: Option<String>,
    pub created_at: String,
}

impl WorkerGrantResponse {
    #[must_use]
    pub fn from_model(m: &scheduled_worker_tenant_grants::Model) -> Self {
        Self {
            id: m.id.to_string(),
            worker_def_id: m.worker_def_id.to_string(),
            tenant_id: m.tenant_id.to_string(),
            granted_by: m.granted_by.map(|v| v.to_string()),
            created_at: m.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchGrantsRequest {
    pub tenant_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantedTenantResponse {
    pub id: String,
    pub name: String,
    pub code: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerScheduleResponse {
    pub id: String,
    pub worker_def_id: String,
    pub tenant_id: String,
    pub name: String,
    pub cron_expr: String,
    pub params_json: Option<String>,
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub worker_name: Option<String>,
    pub worker_code: Option<String>,
}

impl WorkerScheduleResponse {
    #[must_use]
    pub fn from_model(m: &scheduled_worker_schedules::Model) -> Self {
        Self {
            id: m.id.to_string(),
            worker_def_id: m.worker_def_id.to_string(),
            tenant_id: m.tenant_id.to_string(),
            name: m.name.clone(),
            cron_expr: m.cron_expr.clone(),
            params_json: m.params_json.clone(),
            enabled: m.enabled,
            last_run_at: m.last_run_at.map(|dt| dt.to_rfc3339()),
            next_run_at: m.next_run_at.map(|dt| dt.to_rfc3339()),
            created_by: m.created_by.map(|v| v.to_string()),
            updated_by: m.updated_by.map(|v| v.to_string()),
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
            worker_name: None,
            worker_code: None,
        }
    }

    #[must_use]
    pub fn with_worker_info(mut self, name: &str, code: &str) -> Self {
        self.worker_name = Some(name.to_string());
        self.worker_code = Some(code.to_string());
        self
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkerScheduleRequest {
    pub worker_def_id: String,
    pub name: String,
    pub cron_expr: String,
    pub params_json: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkerScheduleRequest {
    pub name: Option<String>,
    pub cron_expr: Option<String>,
    pub params_json: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchEnabledRequest {
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerExecutionResponse {
    pub id: String,
    pub schedule_id: String,
    pub worker_def_id: String,
    pub tenant_id: String,
    pub trigger_type: String,
    pub triggered_by: Option<String>,
    pub params_json: Option<String>,
    pub status: String,
    pub retry_count: i32,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration_ms: Option<i32>,
    pub output: Option<String>,
    pub error_message: Option<String>,
    pub traceparent: Option<String>,
    pub created_at: String,
    pub worker_name: Option<String>,
    pub worker_code: Option<String>,
    pub schedule_name: Option<String>,
}

impl WorkerExecutionResponse {
    #[must_use]
    pub fn from_model(m: &scheduled_worker_executions::Model) -> Self {
        Self {
            id: m.id.to_string(),
            schedule_id: m.schedule_id.to_string(),
            worker_def_id: m.worker_def_id.to_string(),
            tenant_id: m.tenant_id.to_string(),
            trigger_type: m.trigger_type.clone(),
            triggered_by: m.triggered_by.map(|v| v.to_string()),
            params_json: m.params_json.clone(),
            status: m.status.clone(),
            retry_count: m.retry_count,
            started_at: m.started_at.map(|dt| dt.to_rfc3339()),
            finished_at: m.finished_at.map(|dt| dt.to_rfc3339()),
            duration_ms: m.duration_ms,
            output: m.output.clone(),
            error_message: m.error_message.clone(),
            traceparent: m.traceparent.clone(),
            created_at: m.created_at.to_rfc3339(),
            worker_name: None,
            worker_code: None,
            schedule_name: None,
        }
    }

    #[must_use]
    pub fn with_worker_info(mut self, name: &str, code: &str) -> Self {
        self.worker_name = Some(name.to_string());
        self.worker_code = Some(code.to_string());
        self
    }

    #[must_use]
    pub fn with_schedule_name(mut self, name: &str) -> Self {
        self.schedule_name = Some(name.to_string());
        self
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerResponse {
    pub execution_id: String,
}
