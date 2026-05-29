//! File response/request DTOs (Wave 1).
//!
//! All field names use camelCase per project convention
//! ("前后端交互字段，使用小驼峰的形式").

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::_entities::files;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileResponse {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub mime_type: String,
    pub size: i64,
    pub content_hash: String,
    pub content_hash_algo: String,
    pub content_hash_fast: Option<String>,
    pub storage_backend: String,
    pub bucket: String,
    pub storage_key: String,
    pub status: String,
    pub status_reason: Option<String>,
    /// Source upload session for files completed via multipart;
    /// `None` for small-file direct uploads (设计文档 §8.3 L800).
    pub multipart_upload_id: Option<Uuid>,
    pub uploaded_by: Uuid,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
}

impl From<files::Model> for FileResponse {
    fn from(m: files::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            name: m.name,
            mime_type: m.mime_type,
            size: m.size,
            content_hash: m.content_hash,
            content_hash_algo: m.content_hash_algo,
            content_hash_fast: m.content_hash_fast,
            storage_backend: m.storage_backend,
            bucket: m.bucket,
            storage_key: m.storage_key,
            status: m.status,
            status_reason: m.status_reason,
            multipart_upload_id: None,
            uploaded_by: m.uploaded_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmallUploadRequest {
    pub name: String,
    pub mime_type_hint: Option<String>,
    /// Optional Wave 5 D4: bind the just-uploaded file to a business
    /// resource in the same logical operation. Sent as a JSON sidecar
    /// field in the multipart form (`attachTo` part with JSON body).
    pub attach_to: Option<crate::views::file_references::AttachReferenceRequest>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupCheckRequest {
    pub content_hash: String,
    pub size: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupCheckResponse {
    pub hit: bool,
    pub file: Option<FileResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadUrlResponse {
    pub url: String,
    pub expires_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoftDeleteRequest {
    pub reason: Option<String>,
}

/// `?disposition=inline|attachment` for `/files/{id}/download-url` (and the
/// sys mirror). Default `attachment` keeps backward-compat with the old
/// download button; UI surfaces that need in-browser preview (image / PDF
/// viewers) should pass `inline`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadUrlQuery {
    pub disposition: Option<String>,
}
