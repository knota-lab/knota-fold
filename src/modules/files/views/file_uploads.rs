use std::collections::HashMap;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::models::_entities::{file_upload_parts, file_uploads, files};
use crate::views::errors::err_bad_request;

const B3_FAST_PREFIX: &str = "b3fast:";
const B3_FAST_HEX_LEN: usize = 64;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitiateUploadRequest {
    pub file_name: String,
    pub expected_size: i64,
    /// Optional: small uploads / instant-confirm paths supply a known full
    /// hash up-front so the server can validate the streamed object on
    /// completion. Large multipart streams omit it - the hash computed
    /// while streaming is treated as authoritative.
    pub expected_hash: Option<String>,
    pub expected_hash_algo: String,
    pub part_size: i64,
    pub expected_hash_fast: Option<String>,
    pub mime_type_hint: Option<String>,
}

pub type InitUploadRequest = InitiateUploadRequest;

#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRequest {
    pub file_name: String,
    pub file_size: i64,
    pub content_hash_fast: String,
    /// Advisory only: logged at INFO and never used in matching logic.
    pub mime_type_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "match", rename_all = "snake_case")]
pub enum ProbeResponse {
    /// No same-tenant candidate exists for `(content_hash_fast, size)`.
    /// Client should proceed to multipart upload via the returned hint.
    Miss(ProbeMissResponse),
    /// At least one same-tenant candidate exists. Client should compute
    /// the full BLAKE3 hash and call `instant-upload` to either confirm
    /// the dedup match (no bytes uploaded) or fall back to multipart.
    Suspect(ProbeSuspectResponse),
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProbeMissResponse {
    pub upload_hint: ProbeUploadHint,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProbeUploadHint {
    pub endpoint: String,
    pub part_size: u64,
    pub parts_total: u32,
    pub concurrency_hint: u32,
    pub requires_full_hash: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProbeSuspectResponse {
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: chrono::DateTime<chrono::FixedOffset>,
    pub requires_full_hash_confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitiateUploadResponse {
    pub id: Uuid,
    pub status: String,
    pub part_size: i64,
    pub parts_total: i32,
    pub presigned_url_ttl_seconds: u64,
    pub expires_at: DateTime<FixedOffset>,
    pub temp_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignPartResponse {
    pub upload_id: Uuid,
    pub part_number: u32,
    pub url: String,
    pub method: String,
    pub required_headers: HashMap<String, String>,
    pub expires_at: DateTime<FixedOffset>,
    pub presigned_url_ttl_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterPartRequest {
    pub etag: String,
    pub size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterPartResponse {
    pub upload_id: Uuid,
    pub part_number: u32,
    pub parts_received: i32,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UploadFileSummary {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    pub name: String,
    pub mime_type: String,
    pub size: i64,
    pub content_hash: String,
    pub content_hash_algo: String,
    pub status: String,
}

/// Request body for `POST /api/file-uploads/{id}/complete`.
///
/// `attach_to` is optional; when present the upload is bound to a business
/// resource in the same DB transaction that finalizes the `files` row, so
/// the file is never visible without its reference. Mirrors
/// `InstantUploadRequest::attach_to` and the multipart `attachTo` sidecar
/// for `small_upload`.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompleteUploadRequest {
    #[serde(default)]
    pub attach_to: Option<crate::views::file_references::AttachReferenceRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteUploadResponse {
    pub file: UploadFileSummary,
    pub upload_id: Uuid,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbortUploadResponse {
    pub id: Uuid,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedPartResponse {
    pub part_number: i32,
    pub etag: String,
    pub size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeUploadResponse {
    pub id: Uuid,
    pub file_name: String,
    pub expected_size: i64,
    /// None when the upload was initiated without an up-front hash;
    /// gets populated by `complete_upload` once streamed hash is known.
    pub expected_hash: Option<String>,
    pub expected_hash_algo: String,
    pub part_size: i64,
    pub parts_total: i32,
    pub parts_received: i32,
    pub status: String,
    pub expires_at: DateTime<FixedOffset>,
    pub uploaded_parts: Vec<UploadedPartResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpiredUploadResponse {
    pub id: Uuid,
    pub status: String,
    pub expired_at: DateTime<FixedOffset>,
    pub status_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadPurgeOutcome {
    pub soft_deleted: u64,
    pub hard_deleted: u64,
}

/// Client-driven instant-upload (秒传) request.
///
/// Sent after a `/probe` Suspect verdict, once the client has computed
/// the full BLAKE3 hash. The server attempts to dedup against an existing
/// (active or soft-deleted) file row; on success no bytes are uploaded.
/// On miss, the response signals the client to fall back to the standard
/// multipart upload flow via the returned hint.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InstantUploadRequest {
    pub file_name: String,
    pub expected_size: i64,
    pub expected_hash: String,
    pub expected_hash_algo: String,
    pub expected_hash_fast: String,
    pub mime_type_hint: Option<String>,
    /// Wave 5 D4c: optional binding payload. When present, on a dedup
    /// hit the file is also attached to the supplied business
    /// resource (sequenced after revive/use of the winner row).
    #[serde(default)]
    pub attach_to: Option<crate::views::file_references::AttachReferenceRequest>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum InstantUploadResponse {
    /// Dedup hit: the client may use the returned file immediately.
    /// `revived` is true iff a soft-deleted file row was restored to
    /// satisfy the request; clients can use this to decide whether to
    /// surface a "restored" message in the UI.
    Confirmed(InstantUploadConfirmed),
    /// No matching file - client must fall back to multipart upload.
    Miss(InstantUploadMiss),
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InstantUploadConfirmed {
    pub file: UploadFileSummary,
    pub revived: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InstantUploadMiss {
    pub upload_hint: ProbeUploadHint,
}

impl From<&file_uploads::Model> for InitiateUploadResponse {
    fn from(value: &file_uploads::Model) -> Self {
        Self {
            id: value.id,
            status: value.status.clone(),
            part_size: value.part_size,
            parts_total: value.parts_total,
            presigned_url_ttl_seconds: 3600,
            expires_at: value.expires_at,
            temp_key: value.temp_key.clone(),
        }
    }
}

impl From<&file_upload_parts::Model> for UploadedPartResponse {
    fn from(value: &file_upload_parts::Model) -> Self {
        Self {
            part_number: value.part_number,
            etag: value.etag.clone(),
            size: value.size,
        }
    }
}

impl From<&files::Model> for UploadFileSummary {
    fn from(value: &files::Model) -> Self {
        Self {
            id: value.id,
            name: value.name.clone(),
            mime_type: value.mime_type.clone(),
            size: value.size,
            content_hash: value.content_hash.clone(),
            content_hash_algo: value.content_hash_algo.clone(),
            status: value.status.to_lowercase(),
        }
    }
}

pub fn validate_b3_fast_hash(hash: &str, field_name: &str) -> loco_rs::Result<()> {
    let stripped = hash.strip_prefix(B3_FAST_PREFIX).ok_or_else(|| {
        err_bad_request(
            "file.hash_fast_prefix_invalid",
            format!("{field_name} 必须以 {B3_FAST_PREFIX} 开头"),
        )
    })?;

    if stripped.len() != B3_FAST_HEX_LEN {
        return Err(err_bad_request(
            "file.hash_fast_length_invalid",
            format!("{field_name} 必须包含 64 个小写十六进制字符"),
        ));
    }

    if !stripped
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(err_bad_request(
            "file.hash_fast_hex_invalid",
            format!("{field_name} 只能包含小写十六进制字符"),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_b3_fast_hash;
    use crate::utils::file_hash::validate_b3_hash;

    #[test]
    fn validate_b3_fast_hash_rejects_malformed_value() {
        let err = validate_b3_fast_hash("b3fast:not-hex", "contentHashFast").unwrap_err();
        assert!(format!("{err:?}").contains("contentHashFast"));
    }

    #[test]
    fn validate_b3_hash_rejects_malformed_value() {
        let err = validate_b3_hash("b3:short").unwrap_err();
        assert!(format!("{err:?}").contains("contentHash"));
    }
}
