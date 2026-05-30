use std::future::Future;
use std::pin::Pin;

#[cfg(debug_assertions)]
use std::sync::{Mutex, OnceLock};
use std::{collections::HashMap, ops::Range, time::Duration};

use aws_sdk_s3::{
    operation::upload_part::builders::UploadPartFluentBuilder,
    presigning::PresigningConfig,
    types::{CompletedMultipartUpload, CompletedPart, Delete, ObjectIdentifier},
};
use axum::http::StatusCode;
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, Utc};
use loco_rs::{app::AppContext, controller::ErrorDetail, prelude::*};
use sea_orm::{
    sea_query::{Expr, OnConflict},
    ActiveModelTrait, ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection,
    DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
    TransactionTrait, TryInsertResult,
};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::utils::error::{db_err_into, ErrInto, IntoAppError, OptionErrInto};
use crate::views::errors::err_bad_request;
use crate::{
    initializers::s3::{SharedS3Client, SharedS3Config},
    models::{
        _entities::{file_upload_idempotency, file_upload_parts, file_uploads, files},
        file_repo,
    },
    services::{audit_service, file_reference_service, file_service, partition_policy},
    utils::{
        file_hash::{format_b3_hash, validate_b3_hash},
        id::generate_id,
        mime::{detect_mime, is_blacklisted},
    },
    views::{
        audit_logs::{
            AuditAction, AuditContext, FileAuditSnapshot, FileUploadAuditSnapshot,
        },
        file_uploads::{
            validate_b3_fast_hash, AbortUploadResponse, CompleteUploadResponse,
            ExpiredUploadResponse, InitiateUploadRequest, InitiateUploadResponse,
            RegisterPartRequest, RegisterPartResponse, ResumeUploadResponse,
            SignPartResponse, UploadFileSummary, UploadPurgeOutcome,
            UploadedPartResponse,
        },
    },
};

const STATUS_INITIATED: &str = "Initiated";
const STATUS_IN_PROGRESS: &str = "InProgress";
const STATUS_COMPLETING: &str = "Completing";
const STATUS_COMPLETED: &str = "Completed";
const STATUS_ABORTED: &str = "Aborted";
const STATUS_EXPIRED: &str = "Expired";

const FILE_STATUS_ACTIVE: &str = "ACTIVE";
const STORAGE_BACKEND_MINIO: &str = "minio";
const CONTENT_HASH_ALGO_B3: &str = "b3";

const ENDPOINT_INITIATE: &str = "initiate";
const ENDPOINT_COMPLETE: &str = "complete";
const ENDPOINT_ABORT: &str = "abort";

const PURGE_STATUS_REASON: &str = "ttl_purged";
const ABORT_STATUS_REASON: &str = "aborted_by_user";
const HASH_MISMATCH_STATUS_REASON: &str = "hash_mismatch";
const FAST_HASH_MISMATCH_STATUS_REASON: &str = "fast_hash_mismatch";
const MIME_BLACKLIST_STATUS_REASON: &str = "mime_blacklisted";
const COMPLETE_RETRYABLE_STATUS_REASON: &str = "complete_failed_retryable";
const COMPLETE_OBJECT_READ_FAILED_REASON: &str = "complete_object_read_failed";
const SIZE_MISMATCH_STATUS_REASON: &str = "size_mismatch";
const COMPLETE_COPY_FAILED_REASON: &str = "complete_copy_failed";
const COMPLETE_DB_FAILED_REASON: &str = "complete_db_failed";
const COMPLETE_STALE_ABORTED_REASON: &str = "complete_stale_aborted";

#[cfg(debug_assertions)]
static REGISTER_PART_PRE_UPDATE_FLIP: OnceLock<Mutex<Option<Uuid>>> = OnceLock::new();

const MAX_FILE_NAME_LEN: usize = 512;
const MAX_PARTS: i64 = 10_000;
const MAX_TOTAL_SIZE: i64 = 100 * 1024 * 1024 * 1024;
const PRESIGNED_URL_TTL_SECONDS: u64 = 3600;
const UPLOAD_TTL_HOURS: i64 = 24;
const HARD_PURGE_RETENTION_DAYS: i64 = 7;
const COMPLETING_STALE_HOURS: i64 = 1;
const MIB: u64 = 1024 * 1024;
const FAST_HASH_THRESHOLD: u64 = 32 * MIB;
const FAST_HASH_WINDOW: u64 = 10 * MIB;
const MIME_SNIFF_BYTES: usize = 8192;

pub struct JsonEndpointResponse {
    pub status_code: StatusCode,
    pub body_bytes: Vec<u8>,
}

struct InitiatePersistArgs<'a> {
    ctx: &'a AppContext,
    s3_client: &'a SharedS3Client,
    bucket: &'a str,
    key_name: &'a str,
    s3_upload_id: String,
    upload_id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
    params: &'a InitiateUploadRequest,
    part_size: i64,
    parts_total: i32,
    expires_at: DateTime<FixedOffset>,
    idem_key: &'a str,
}

struct RegisterPartFinalizeArgs<'a> {
    db: &'a DatabaseConnection,
    upload_id: Uuid,
    part_number_i32: i32,
    user_id: Uuid,
    params: &'a RegisterPartRequest,
    endpoint: &'a str,
    idem_key: &'a str,
    current_status: &'a str,
}

struct CompleteUploadRequest<'a> {
    ctx: &'a AppContext,
    tenant_id: Uuid,
    user_id: Uuid,
    upload_id: Uuid,
    idempotency_key: Option<&'a str>,
    audit_ctx: &'a AuditContext,
    attach: Option<Box<file_reference_service::AttachRequest>>,
}

mod complete;

fn require_shared_s3_client(ctx: &AppContext) -> loco_rs::Result<SharedS3Client> {
    ctx.shared_store.get::<SharedS3Client>().ok_or_else(|| {
        Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("storage.not_initialized", "S3 存储客户端未初始化"),
        )
    })
}

fn require_shared_s3_config(ctx: &AppContext) -> loco_rs::Result<SharedS3Config> {
    ctx.shared_store.get::<SharedS3Config>().ok_or_else(|| {
        Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("storage.config_not_initialized", "S3 存储配置未初始化"),
        )
    })
}

fn ensure_idempotency_key(idempotency_key: Option<&str>) -> loco_rs::Result<&str> {
    idempotency_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            crate::views::errors::err_custom(
                StatusCode::BAD_REQUEST,
                "missing_idempotency_key",
                "Idempotency-Key header is required",
            )
        })
}

fn endpoint_register_part(part_number: u32) -> String {
    format!("register-part:{part_number}")
}

#[cfg(debug_assertions)]
pub fn set_register_part_pre_update_flip(upload_id: Option<Uuid>) {
    let slot = REGISTER_PART_PRE_UPDATE_FLIP.get_or_init(|| Mutex::new(None));
    *slot.lock().expect("register part test hook poisoned") = upload_id;
}

#[cfg(debug_assertions)]
async fn maybe_flip_register_part_pre_update_state(
    ctx: &AppContext,
    upload_id: Uuid,
    user_id: Uuid,
) -> loco_rs::Result<()> {
    let should_flip = {
        let slot = REGISTER_PART_PRE_UPDATE_FLIP.get_or_init(|| Mutex::new(None));
        *slot.lock().expect("register part test hook poisoned") == Some(upload_id)
    };

    if should_flip {
        set_register_part_pre_update_flip(None);
        file_uploads::Entity::update_many()
            .col_expr(file_uploads::Column::Status, Expr::value(STATUS_COMPLETING))
            .col_expr(file_uploads::Column::UpdatedAt, Expr::value(Utc::now()))
            .col_expr(file_uploads::Column::UpdatedBy, Expr::value(user_id))
            .filter(file_uploads::Column::Id.eq(upload_id))
            .exec(&ctx.db)
            .await
            .map_err(|e| {
                crate::views::errors::err_custom(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "test_hook_failed",
                    &format!("register_part test hook failed: {e}"),
                )
            })?;
    }

    Ok(())
}

fn validate_file_name(file_name: &str) -> loco_rs::Result<()> {
    if file_name.trim().is_empty() {
        return Err(err_bad_request("file.name_empty", "文件名不能为空"));
    }

    if file_name.len() > MAX_FILE_NAME_LEN {
        return Err(err_bad_request(
            "file.name_too_long",
            format!("文件名过长 (max {MAX_FILE_NAME_LEN} chars)"),
        ));
    }

    Ok(())
}

fn temp_key(upload_id: Uuid) -> String {
    format!("uploads/{upload_id}/multipart.bin")
}

fn final_storage_key(file_id: Uuid, content_hash: &str) -> String {
    let stripped = content_hash.trim_start_matches("b3:");
    format!("files/{file_id}/{stripped}.bin")
}

fn presigned_expiry() -> loco_rs::Result<PresigningConfig> {
    PresigningConfig::expires_in(Duration::from_secs(PRESIGNED_URL_TTL_SECONDS))
        .err_info(crate::error_info::common::DB_ERROR)
}

fn utc_plus_seconds(seconds: u64) -> DateTime<FixedOffset> {
    (Utc::now() + ChronoDuration::seconds(i64::try_from(seconds).unwrap_or(i64::MAX)))
        .fixed_offset()
}

fn upload_expiry() -> DateTime<FixedOffset> {
    (Utc::now() + ChronoDuration::hours(UPLOAD_TTL_HOURS)).fixed_offset()
}

fn status_code_to_i32(status_code: StatusCode) -> i32 {
    i32::from(status_code.as_u16())
}

fn i32_to_status_code(status_code: i32) -> loco_rs::Result<StatusCode> {
    let value = u16::try_from(status_code).map_err(|_| {
        Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new(
                "upload.status_code_overflow",
                "cached status code overflow",
            ),
        )
    })?;
    StatusCode::from_u16(value).err_info(crate::error_info::common::DB_ERROR)
}

fn json_bytes<T: serde::Serialize>(value: &T) -> loco_rs::Result<Vec<u8>> {
    serde_json::to_vec(value).err_info(crate::error_info::common::DB_ERROR)
}

fn json_endpoint_response<T: serde::Serialize>(
    status_code: StatusCode,
    value: &T,
) -> loco_rs::Result<JsonEndpointResponse> {
    Ok(JsonEndpointResponse {
        status_code,
        body_bytes: json_bytes(value)?,
    })
}

fn validate_initiate_request(params: &InitiateUploadRequest) -> loco_rs::Result<()> {
    validate_file_name(&params.file_name)?;

    if params.expected_size <= 0 {
        return Err(err_bad_request(
            "file.expected_size_must_be_positive",
            "expectedSize 必须大于 0",
        ));
    }

    if params.expected_size > MAX_TOTAL_SIZE {
        return Err(crate::views::errors::err_custom(
            StatusCode::UNPROCESSABLE_ENTITY,
            "expected_size_too_large",
            "expectedSize exceeds 100 GiB limit",
        ));
    }

    if params.expected_hash_algo != CONTENT_HASH_ALGO_B3 {
        return Err(crate::views::errors::err_custom(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsupported_hash_algo",
            "expectedHashAlgo must be b3",
        ));
    }

    if let Some(expected_hash) = params.expected_hash.as_deref() {
        validate_b3_hash(expected_hash)?;
    }

    if let Some(expected_hash_fast) = params.expected_hash_fast.as_deref() {
        validate_b3_fast_hash(expected_hash_fast, "expectedHashFast")?;
    }

    if let Some(mime_type_hint) = params.mime_type_hint.as_deref() {
        if is_blacklisted(mime_type_hint) {
            return Err(crate::views::errors::err_custom(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                "mimeTypeHint is blocked for upload",
            ));
        }
    }

    Ok(())
}

const fn fast_hash_sample_ranges(size: u64) -> Option<[Range<u64>; 3]> {
    if size < FAST_HASH_THRESHOLD {
        return None;
    }

    let first_range = 0..FAST_HASH_WINDOW;
    let middle_start = (size / 2).saturating_sub(FAST_HASH_WINDOW / 2);
    let middle_range = middle_start..(middle_start + FAST_HASH_WINDOW);
    let last_range = (size - FAST_HASH_WINDOW)..size;

    Some([first_range, middle_range, last_range])
}

struct StreamedHashes {
    full_hash: String,
    fast_hash: Option<String>,
    mime_sample: Vec<u8>,
    actual_size: u64,
}

async fn stream_object_hashes(
    client: &SharedS3Client,
    bucket: &str,
    key: &str,
    size: i64,
) -> loco_rs::Result<StreamedHashes> {
    let response = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, bucket, key, "failed to fetch uploaded object");
            crate::views::errors::err_custom(
                StatusCode::SERVICE_UNAVAILABLE,
                "storage_unavailable",
                "failed to read uploaded object",
            )
        })?;

    let mut reader = response.body.into_async_read();
    let mut full_hasher = blake3::Hasher::new();
    let mut fast_hasher = blake3::Hasher::new();
    let ranges = fast_hash_sample_ranges(size.try_into().unwrap_or_default());
    let mut range_index = 0usize;
    let mut offset = 0u64;
    let mut mime_sample = Vec::with_capacity(MIME_SNIFF_BYTES);
    let mut buffer = vec![0_u8; 64 * 1024];

    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .err_info(crate::error_info::common::DB_ERROR)?;
        if read == 0 {
            break;
        }

        let chunk = &buffer[..read];
        full_hasher.update(chunk);

        if mime_sample.len() < MIME_SNIFF_BYTES {
            let take = (MIME_SNIFF_BYTES - mime_sample.len()).min(chunk.len());
            mime_sample.extend_from_slice(&chunk[..take]);
        }

        if let Some(sample_ranges) = ranges.as_ref() {
            let chunk_start = offset;
            let chunk_end = offset + read as u64;

            while range_index < sample_ranges.len() {
                let range = &sample_ranges[range_index];
                if chunk_end <= range.start {
                    break;
                }
                if chunk_start >= range.end {
                    range_index += 1;
                    continue;
                }

                let start = range.start.max(chunk_start) - chunk_start;
                let end = range.end.min(chunk_end) - chunk_start;
                if start < end {
                    fast_hasher.update(&chunk[start as usize..end as usize]);
                }

                if chunk_end >= range.end {
                    range_index += 1;
                } else {
                    break;
                }
            }
        }

        offset += read as u64;
    }

    Ok(StreamedHashes {
        full_hash: format_b3_hash(&full_hasher.finalize()),
        fast_hash: ranges.map(|_| format!("b3fast:{}", fast_hasher.finalize().to_hex())),
        mime_sample,
        actual_size: offset,
    })
}

fn ensure_part_number(
    upload: &file_uploads::Model,
    part_number: u32,
) -> loco_rs::Result<i32> {
    if part_number == 0 || i64::from(part_number) > i64::from(upload.parts_total) {
        return Err(crate::views::errors::err_custom(
            StatusCode::BAD_REQUEST,
            "invalid_part_number",
            "part number is out of range",
        ));
    }

    i32::try_from(part_number)
        .map_err(|_| err_bad_request("file.invalid_part_number", "无效的分片编号"))
}

fn validate_registered_size(
    upload: &file_uploads::Model,
    part_number: i32,
    size: i64,
) -> loco_rs::Result<()> {
    if size <= 0 {
        return Err(crate::views::errors::err_custom(
            StatusCode::BAD_REQUEST,
            "invalid_part_size",
            "registered part size must be greater than 0",
        ));
    }

    if part_number < upload.parts_total && size != upload.part_size {
        return Err(crate::views::errors::err_custom(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_part_size",
            "non-final parts must match partSize",
        ));
    }

    if part_number == upload.parts_total {
        let prior_parts = i64::from(upload.parts_total - 1);
        let expected_last_size = upload.expected_size - (prior_parts * upload.part_size);
        if size != expected_last_size {
            return Err(crate::views::errors::err_custom(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_part_size",
                "final part size does not match expectedSize",
            ));
        }
    }

    Ok(())
}

async fn guard_register_part_state_update(
    txn: &DatabaseTransaction,
    upload_id: Uuid,
    user_id: Uuid,
    parts_received: i32,
    next_status: &str,
) -> loco_rs::Result<u64> {
    let update_result = file_uploads::Entity::update_many()
        .col_expr(
            file_uploads::Column::PartsReceived,
            Expr::value(parts_received),
        )
        .col_expr(file_uploads::Column::Status, Expr::value(next_status))
        .col_expr(file_uploads::Column::UpdatedAt, Expr::value(Utc::now()))
        .col_expr(file_uploads::Column::UpdatedBy, Expr::value(user_id))
        .filter(file_uploads::Column::Id.eq(upload_id))
        .filter(
            file_uploads::Column::Status
                .is_in([STATUS_INITIATED.to_string(), STATUS_IN_PROGRESS.to_string()]),
        )
        .exec(txn)
        .await
        .db_err()?;

    Ok(update_result.rows_affected)
}

async fn load_upload<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    upload_id: Uuid,
) -> loco_rs::Result<file_uploads::Model> {
    file_uploads::Entity::find()
        .filter(file_uploads::Column::Id.eq(upload_id))
        .filter(file_uploads::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)
}

async fn load_upload_optional<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    upload_id: Uuid,
) -> loco_rs::Result<Option<file_uploads::Model>> {
    file_uploads::Entity::find()
        .filter(file_uploads::Column::Id.eq(upload_id))
        .filter(file_uploads::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .db_err()
}

async fn load_parts<C: ConnectionTrait>(
    db: &C,
    upload_id: Uuid,
) -> loco_rs::Result<Vec<file_upload_parts::Model>> {
    file_upload_parts::Entity::find()
        .filter(file_upload_parts::Column::UploadId.eq(upload_id))
        .order_by_asc(file_upload_parts::Column::PartNumber)
        .all(db)
        .await
        .db_err()
}

async fn find_cached_success_by_upload(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    upload_id: Uuid,
    endpoint: &str,
    idempotency_key: &str,
) -> loco_rs::Result<Option<JsonEndpointResponse>> {
    if load_upload_optional(db, tenant_id, upload_id)
        .await?
        .is_none()
    {
        return Ok(None);
    }

    let cached = file_upload_idempotency::Entity::find_by_id((
        upload_id,
        endpoint.to_string(),
        idempotency_key.to_string(),
    ))
    .one(db)
    .await
    .db_err()?;

    cached
        .map(|value| {
            Ok(JsonEndpointResponse {
                status_code: i32_to_status_code(value.status_code)?,
                body_bytes: value.response_body,
            })
        })
        .transpose()
}

async fn find_cached_initiate_success(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    idempotency_key: &str,
) -> loco_rs::Result<Option<JsonEndpointResponse>> {
    let candidates = file_upload_idempotency::Entity::find()
        .filter(file_upload_idempotency::Column::Endpoint.eq(ENDPOINT_INITIATE))
        .filter(file_upload_idempotency::Column::IdempotencyKey.eq(idempotency_key))
        .order_by_desc(file_upload_idempotency::Column::CreatedAt)
        .all(db)
        .await
        .db_err()?;

    for candidate in candidates {
        let Some(upload) =
            load_upload_optional(db, tenant_id, candidate.upload_id).await?
        else {
            continue;
        };

        if upload.created_by != user_id {
            continue;
        }

        return Ok(Some(JsonEndpointResponse {
            status_code: i32_to_status_code(candidate.status_code)?,
            body_bytes: candidate.response_body,
        }));
    }

    Ok(None)
}

async fn cache_success<C: ConnectionTrait>(
    db: &C,
    upload_id: Uuid,
    endpoint: &str,
    idempotency_key: &str,
    response: &JsonEndpointResponse,
) -> loco_rs::Result<()> {
    file_upload_idempotency::Entity::insert(file_upload_idempotency::ActiveModel {
        upload_id: Set(upload_id),
        endpoint: Set(endpoint.to_string()),
        idempotency_key: Set(idempotency_key.to_string()),
        response_body: Set(response.body_bytes.clone()),
        status_code: Set(status_code_to_i32(response.status_code)),
        created_at: Set(Utc::now().fixed_offset()),
    })
    .on_conflict(
        OnConflict::columns([
            file_upload_idempotency::Column::UploadId,
            file_upload_idempotency::Column::Endpoint,
            file_upload_idempotency::Column::IdempotencyKey,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec(db)
    .await
    .db_err()?;

    Ok(())
}

async fn fetch_completed_file<C: ConnectionTrait>(
    db: &C,
    file_id: Uuid,
) -> loco_rs::Result<files::Model> {
    files::Entity::find_by_id(file_id)
        .one(db)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)
}

fn parse_completed_file_id(upload: &file_uploads::Model) -> loco_rs::Result<Uuid> {
    upload.completed_file_id.ok_or_else(|| {
        Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new(
                "upload.missing_completed_file_id",
                "completed upload is missing completed_file_id",
            ),
        )
    })
}

fn completed_response(file: &files::Model, upload_id: Uuid) -> CompleteUploadResponse {
    CompleteUploadResponse {
        file: UploadFileSummary::from(file),
        upload_id,
        status: STATUS_COMPLETED.to_string(),
    }
}

fn assert_active_upload(upload: &file_uploads::Model) -> loco_rs::Result<()> {
    match upload.status.as_str() {
        STATUS_INITIATED | STATUS_IN_PROGRESS => Ok(()),
        STATUS_COMPLETING => Err(crate::views::errors::err_custom(
            StatusCode::CONFLICT,
            "upload_busy",
            "upload cannot be signed while completing or completed",
        )),
        STATUS_COMPLETED => Err(crate::views::errors::err_custom(
            StatusCode::CONFLICT,
            "upload_completed",
            "upload is already completed",
        )),
        STATUS_ABORTED => Err(crate::views::errors::err_custom(
            StatusCode::CONFLICT,
            "upload_aborted",
            "upload has been aborted",
        )),
        STATUS_EXPIRED => Err(crate::views::errors::err_custom(
            StatusCode::GONE,
            "upload_expired",
            "upload has expired",
        )),
        _ => Err(crate::views::errors::err_custom(
            StatusCode::CONFLICT,
            "upload_invalid_state",
            "upload is not in an active state",
        )),
    }
}

fn assert_signable(
    upload: &file_uploads::Model,
    part_number: u32,
) -> loco_rs::Result<i32> {
    assert_active_upload(upload)?;
    ensure_part_number(upload, part_number)
}

fn ensure_parts_continuity(
    upload: &file_uploads::Model,
    parts: &[file_upload_parts::Model],
) -> loco_rs::Result<()> {
    if parts.len() != usize::try_from(upload.parts_total).unwrap_or_default() {
        return Err(crate::views::errors::err_custom(
            StatusCode::UNPROCESSABLE_ENTITY,
            "missing_parts",
            "registered parts are incomplete",
        ));
    }

    let mut total_size = 0_i64;
    for (expected_part_number, part) in (1_i32..=upload.parts_total).zip(parts.iter()) {
        if part.part_number != expected_part_number {
            return Err(crate::views::errors::err_custom(
                StatusCode::UNPROCESSABLE_ENTITY,
                "part_number_gap",
                "registered parts contain a gap",
            ));
        }

        validate_registered_size(upload, expected_part_number, part.size)?;
        total_size += part.size;
    }

    if total_size != upload.expected_size {
        return Err(crate::views::errors::err_custom(
            StatusCode::UNPROCESSABLE_ENTITY,
            "size_mismatch",
            "registered parts total size does not match expectedSize",
        ));
    }

    Ok(())
}

async fn delete_object_if_exists(client: &SharedS3Client, bucket: &str, key: &str) {
    if let Err(err) = client.delete_object().bucket(bucket).key(key).send().await {
        tracing::warn!(error = ?err, bucket, key, "best-effort delete_object failed");
    }
}

async fn delete_temp_prefix(client: &SharedS3Client, bucket: &str, upload_id: Uuid) {
    let prefix = format!("uploads/{upload_id}/");
    let listed = client
        .list_objects_v2()
        .bucket(bucket)
        .prefix(prefix)
        .send()
        .await;

    let Ok(output) = listed else {
        return;
    };

    let object_ids: Vec<ObjectIdentifier> = output
        .contents()
        .iter()
        .filter_map(|object| {
            object
                .key()
                .and_then(|key| ObjectIdentifier::builder().key(key).build().ok())
        })
        .collect();

    if object_ids.is_empty() {
        return;
    }

    if let Ok(delete) = Delete::builder().set_objects(Some(object_ids)).build() {
        let _ = client
            .delete_objects()
            .bucket(bucket)
            .delete(delete)
            .send()
            .await;
    }
}

async fn abort_multipart_if_possible(
    client: &SharedS3Client,
    bucket: &str,
    key: &str,
    upload_id: Option<&str>,
) {
    let Some(upload_id) = upload_id.filter(|value| !value.is_empty()) else {
        return;
    };

    if let Err(err) = client
        .abort_multipart_upload()
        .bucket(bucket)
        .key(key)
        .upload_id(upload_id)
        .send()
        .await
    {
        tracing::warn!(error = ?err, bucket, key, upload_id, "best-effort abort_multipart_upload failed");
    }
}

async fn complete_multipart_upload(
    client: &SharedS3Client,
    bucket: &str,
    key: &str,
    upload_id: &str,
    parts: &[file_upload_parts::Model],
) -> loco_rs::Result<()> {
    let completed_parts: Vec<CompletedPart> = parts
        .iter()
        .map(|part| {
            CompletedPart::builder()
                .e_tag(part.etag.clone())
                .part_number(part.part_number)
                .build()
        })
        .collect();

    client
        .complete_multipart_upload()
        .bucket(bucket)
        .key(key)
        .upload_id(upload_id)
        .multipart_upload(
            CompletedMultipartUpload::builder()
                .set_parts(Some(completed_parts))
                .build(),
        )
        .send()
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, bucket, key, upload_id, "failed to complete multipart upload");
            crate::views::errors::err_custom(
                StatusCode::SERVICE_UNAVAILABLE,
                "storage_unavailable",
                "failed to finalize multipart upload",
            )
        })?;

    Ok(())
}

async fn update_upload_state<C: ConnectionTrait>(
    db: &C,
    upload_id: Uuid,
    user_id: Uuid,
    status: &str,
    status_reason: Option<&str>,
    clear_s3_upload_id: bool,
) -> loco_rs::Result<()> {
    let mut update = file_uploads::Entity::update_many()
        .col_expr(
            file_uploads::Column::Status,
            Expr::value(status.to_string()),
        )
        .col_expr(file_uploads::Column::UpdatedAt, Expr::value(Utc::now()))
        .col_expr(file_uploads::Column::UpdatedBy, Expr::value(user_id));

    update = match status_reason {
        Some(value) => update.col_expr(
            file_uploads::Column::StatusReason,
            Expr::value(value.to_string()),
        ),
        None => update.col_expr(
            file_uploads::Column::StatusReason,
            Expr::value(sea_orm::Value::String(None)),
        ),
    };

    if clear_s3_upload_id {
        update = update.col_expr(
            file_uploads::Column::S3UploadId,
            Expr::value(sea_orm::Value::String(None)),
        );
    }

    update
        .filter(file_uploads::Column::Id.eq(upload_id))
        .exec(db)
        .await
        .db_err()?;

    Ok(())
}

async fn best_effort_terminalize_upload_row(
    db: &DatabaseConnection,
    upload_id: Uuid,
    user_id: Uuid,
    status: &str,
    reason: &str,
    clear_s3_upload_id: bool,
) {
    if let Err(err) = update_upload_state(
        db,
        upload_id,
        user_id,
        status,
        Some(reason),
        clear_s3_upload_id,
    )
    .await
    {
        tracing::warn!(error = ?err, upload_id = %upload_id, status, reason, "failed to best-effort terminalize upload row");
    }
}

async fn cleanup_complete_failure(
    client: &SharedS3Client,
    bucket: &str,
    upload_id: Uuid,
    temp_key: &str,
    final_key: Option<&str>,
) {
    if let Some(final_key) = final_key {
        delete_object_if_exists(client, bucket, final_key).await;
    }
    delete_object_if_exists(client, bucket, temp_key).await;
    delete_temp_prefix(client, bucket, upload_id).await;
}

async fn insert_file_row(
    txn: &DatabaseTransaction,
    upload: &file_uploads::Model,
    file_id: Uuid,
    user_id: Uuid,
    mime_type: &str,
    content_hash_fast: Option<String>,
    final_key: &str,
) -> Result<TryInsertResult<sea_orm::InsertResult<files::ActiveModel>>, sea_orm::DbErr> {
    let active_model = files::ActiveModel {
        id: ActiveValue::Set(file_id),
        tenant_id: ActiveValue::Set(upload.tenant_id),
        name: ActiveValue::Set(upload.file_name.clone()),
        mime_type: ActiveValue::Set(mime_type.to_string()),
        size: ActiveValue::Set(upload.expected_size),
        content_hash: ActiveValue::Set(
            upload
                .expected_hash
                .clone()
                .expect("upload.expected_hash must be set before insert_file_row"),
        ),
        content_hash_algo: ActiveValue::Set(upload.expected_hash_algo.clone()),
        content_hash_fast: ActiveValue::Set(content_hash_fast),
        storage_backend: ActiveValue::Set(STORAGE_BACKEND_MINIO.to_string()),
        bucket: ActiveValue::Set(upload.bucket.clone()),
        storage_key: ActiveValue::Set(final_key.to_string()),
        multipart_upload_id: ActiveValue::Set(Some(upload.id.to_string())),
        status: ActiveValue::Set(FILE_STATUS_ACTIVE.to_string()),
        status_reason: ActiveValue::Set(None),
        deleted_at: ActiveValue::Set(None),
        purge_at: ActiveValue::Set(None),
        deleted_by: ActiveValue::Set(None),
        uploaded_by: ActiveValue::Set(user_id),
        created_by: ActiveValue::Set(user_id),
        updated_by: ActiveValue::Set(user_id),
        ..Default::default()
    };

    files::Entity::insert(active_model)
        .on_conflict(
            OnConflict::columns([
                files::Column::TenantId,
                files::Column::ContentHash,
                files::Column::Size,
            ])
            .do_nothing()
            .to_owned(),
        )
        .do_nothing()
        .exec(txn)
        .await
}

async fn finalize_inserted_file(
    txn: &DatabaseTransaction,
    upload: &file_uploads::Model,
    inserted_file: &files::Model,
    user_id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    file_uploads::Entity::update_many()
        .col_expr(file_uploads::Column::Status, Expr::value(STATUS_COMPLETED))
        .col_expr(
            file_uploads::Column::CompletedFileId,
            Expr::value(inserted_file.id),
        )
        .col_expr(
            file_uploads::Column::ExpectedHashFast,
            Expr::value(upload.expected_hash_fast.clone()),
        )
        .col_expr(
            file_uploads::Column::StatusReason,
            Expr::value(sea_orm::Value::String(None)),
        )
        .col_expr(
            file_uploads::Column::S3UploadId,
            Expr::value(sea_orm::Value::String(None)),
        )
        .col_expr(file_uploads::Column::UpdatedAt, Expr::value(Utc::now()))
        .col_expr(file_uploads::Column::UpdatedBy, Expr::value(user_id))
        .filter(file_uploads::Column::Id.eq(upload.id))
        .exec(txn)
        .await
        .db_err()?;

    let upload_snapshot = FileUploadAuditSnapshot::from(upload);
    let file_snapshot = FileAuditSnapshot::from(inserted_file);
    audit_service::log(
        txn,
        audit_ctx,
        AuditAction::UploadComplete,
        "file_upload",
        &upload.id.to_string(),
        Some(&upload_snapshot),
        Some(&file_snapshot),
    )
    .await?;

    Ok(())
}

pub async fn initiate_upload(
    ctx: &AppContext,
    tenant_id: Uuid,
    user_id: Uuid,
    params: &InitiateUploadRequest,
    idempotency_key: Option<&str>,
) -> loco_rs::Result<JsonEndpointResponse> {
    let key = ensure_idempotency_key(idempotency_key)?;
    if let Some(cached) =
        find_cached_initiate_success(&ctx.db, tenant_id, user_id, key).await?
    {
        return Ok(cached);
    }

    validate_initiate_request(params)?;
    let policy = partition_policy::load_policy_config(&ctx.db, tenant_id).await?;
    let upload_hint = partition_policy::compute(
        params.expected_size.try_into().unwrap_or_default(),
        &policy,
    )?;
    let part_size = i64::try_from(upload_hint.part_size).map_err(|_| {
        Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("upload.part_size_overflow", "part size overflow"),
        )
    })?;
    if !(1..=MAX_PARTS).contains(&i64::from(upload_hint.parts_total)) {
        return Err(crate::views::errors::err_custom(
            StatusCode::UNPROCESSABLE_ENTITY,
            "too_many_parts",
            "partsTotal exceeds 10000",
        ));
    }
    let parts_total = i32::try_from(upload_hint.parts_total).map_err(|_| {
        Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("upload.parts_total_overflow", "parts total overflow"),
        )
    })?;

    let s3_client = require_shared_s3_client(ctx)?;
    let s3_config = require_shared_s3_config(ctx)?;
    let upload_id = generate_id();
    let key_name = temp_key(upload_id);
    let expires_at = upload_expiry();
    let s3_upload_id = create_s3_multipart(
        &s3_client,
        &s3_config.bucket,
        &key_name,
        params.mime_type_hint.as_ref(),
        upload_id,
    )
    .await?;

    persist_upload_and_respond(InitiatePersistArgs {
        ctx,
        s3_client: &s3_client,
        bucket: &s3_config.bucket,
        key_name: &key_name,
        s3_upload_id,
        upload_id,
        tenant_id,
        user_id,
        params,
        part_size,
        parts_total,
        expires_at,
        idem_key: key,
    })
    .await
}

async fn create_s3_multipart(
    s3_client: &SharedS3Client,
    bucket: &str,
    key_name: &str,
    mime_type_hint: Option<&String>,
    upload_id: Uuid,
) -> loco_rs::Result<String> {
    let create_output = s3_client
        .create_multipart_upload()
        .bucket(bucket.to_string())
        .key(key_name.to_string())
        .set_content_type(mime_type_hint.cloned())
        .send()
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, upload_id = %upload_id, "failed to initialize multipart upload");
            crate::views::errors::err_custom(
                StatusCode::SERVICE_UNAVAILABLE,
                "storage_unavailable",
                "failed to initialize multipart upload",
            )
        })?;
    create_output
        .upload_id()
        .ok_or_else(|| {
            Error::CustomError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetail::new(
                    "upload.missing_s3_upload_id",
                    "CreateMultipartUpload response missing upload id",
                ),
            )
        })
        .map(std::string::ToString::to_string)
}

async fn persist_upload_and_respond(
    args: InitiatePersistArgs<'_>,
) -> loco_rs::Result<JsonEndpointResponse> {
    let txn = match args.ctx.db.begin().await {
        Ok(txn) => txn,
        Err(e) => {
            abort_multipart_if_possible(
                args.s3_client,
                args.bucket,
                args.key_name,
                Some(&args.s3_upload_id),
            )
            .await;
            return Err(db_err_into(&e));
        }
    };

    let active_model = file_uploads::ActiveModel {
        id: Set(args.upload_id),
        tenant_id: Set(args.tenant_id),
        file_name: Set(args.params.file_name.clone()),
        mime_type_hint: Set(args.params.mime_type_hint.clone()),
        expected_size: Set(args.params.expected_size),
        expected_hash: Set(args.params.expected_hash.clone()),
        expected_hash_algo: Set(args.params.expected_hash_algo.clone()),
        part_size: Set(args.part_size),
        parts_total: Set(args.parts_total),
        parts_received: Set(0),
        expected_hash_fast: Set(args.params.expected_hash_fast.clone()),
        storage_backend: Set(STORAGE_BACKEND_MINIO.to_string()),
        bucket: Set(args.bucket.to_string()),
        temp_key: Set(args.key_name.to_string()),
        s3_upload_id: Set(Some(args.s3_upload_id.clone())),
        status: Set(STATUS_INITIATED.to_string()),
        status_reason: Set(None),
        expires_at: Set(args.expires_at),
        expired_at: Set(None),
        completed_file_id: Set(None),
        created_by: Set(args.user_id),
        updated_by: Set(args.user_id),
        ..Default::default()
    };

    let upload = match active_model.insert(&txn).await {
        Ok(upload) => upload,
        Err(e) => {
            let _ = txn.rollback().await;
            abort_multipart_if_possible(
                args.s3_client,
                args.bucket,
                args.key_name,
                Some(args.s3_upload_id.as_str()),
            )
            .await;
            return Err(db_err_into(&e));
        }
    };

    let payload = InitiateUploadResponse::from(&upload);
    let response = json_endpoint_response(StatusCode::CREATED, &payload)?;
    if let Err(err) =
        cache_success(&txn, upload.id, ENDPOINT_INITIATE, args.idem_key, &response).await
    {
        let _ = txn.rollback().await;
        abort_multipart_if_possible(
            args.s3_client,
            args.bucket,
            args.key_name,
            Some(args.s3_upload_id.as_str()),
        )
        .await;
        return Err(err);
    }

    if let Err(err) = txn.commit().await {
        abort_multipart_if_possible(
            args.s3_client,
            args.bucket,
            args.key_name,
            Some(args.s3_upload_id.as_str()),
        )
        .await;
        return Err(db_err_into(&err));
    }

    Ok(response)
}

pub async fn sign_part(
    ctx: &AppContext,
    tenant_id: Uuid,
    upload_id: Uuid,
    part_number: u32,
) -> loco_rs::Result<SignPartResponse> {
    let upload = load_upload(&ctx.db, tenant_id, upload_id).await?;
    let part_number_i32 = assert_signable(&upload, part_number)?;

    let s3_upload_id = upload.s3_upload_id.clone().ok_or_else(|| {
        crate::views::errors::err_custom(
            StatusCode::GONE,
            "upload_expired",
            "multipart upload is no longer active",
        )
    })?;

    let s3_client = require_shared_s3_client(ctx)?;
    let mut builder: UploadPartFluentBuilder = s3_client
        .upload_part()
        .bucket(upload.bucket.clone())
        .key(upload.temp_key.clone())
        .part_number(part_number_i32)
        .upload_id(s3_upload_id);

    let content_length = if part_number_i32 == upload.parts_total {
        upload.expected_size - (i64::from(upload.parts_total - 1) * upload.part_size)
    } else {
        upload.part_size
    };
    builder = builder.content_length(content_length);

    let presigned = builder.presigned(presigned_expiry()?).await.map_err(|err| {
        tracing::error!(error = ?err, upload_id = %upload.id, part_number, "failed to presign upload part");
        crate::views::errors::err_custom(
            StatusCode::SERVICE_UNAVAILABLE,
            "storage_unavailable",
            "failed to generate presigned part url",
        )
    })?;

    let mut required_headers = HashMap::new();
    required_headers.insert("content-length".to_string(), content_length.to_string());

    Ok(SignPartResponse {
        upload_id,
        part_number,
        url: presigned.uri().to_string(),
        method: "PUT".to_string(),
        required_headers,
        expires_at: utc_plus_seconds(PRESIGNED_URL_TTL_SECONDS),
        presigned_url_ttl_seconds: PRESIGNED_URL_TTL_SECONDS,
    })
}

pub async fn register_part(
    ctx: &AppContext,
    tenant_id: Uuid,
    user_id: Uuid,
    upload_id: Uuid,
    part_number: u32,
    params: &RegisterPartRequest,
    idempotency_key: Option<&str>,
) -> loco_rs::Result<JsonEndpointResponse> {
    register_part_inner(
        ctx,
        tenant_id,
        user_id,
        upload_id,
        part_number,
        params,
        idempotency_key,
    )
    .await
}

async fn register_part_inner(
    ctx: &AppContext,
    tenant_id: Uuid,
    user_id: Uuid,
    upload_id: Uuid,
    part_number: u32,
    params: &RegisterPartRequest,
    idempotency_key: Option<&str>,
) -> loco_rs::Result<JsonEndpointResponse> {
    let key = ensure_idempotency_key(idempotency_key)?;
    let endpoint = endpoint_register_part(part_number);
    if let Some(cached) =
        find_cached_success_by_upload(&ctx.db, tenant_id, upload_id, &endpoint, key)
            .await?
    {
        return Ok(cached);
    }

    let upload = load_upload(&ctx.db, tenant_id, upload_id).await?;
    let part_number_i32 = assert_signable(&upload, part_number)?;
    validate_registered_size(&upload, part_number_i32, params.size)?;

    if let Some(existing) = file_upload_parts::Entity::find()
        .filter(file_upload_parts::Column::UploadId.eq(upload_id))
        .filter(file_upload_parts::Column::PartNumber.eq(part_number_i32))
        .one(&ctx.db)
        .await
        .db_err()?
    {
        if existing.etag != params.etag || existing.size != params.size {
            return Err(crate::views::errors::err_custom(
                StatusCode::CONFLICT,
                "part_conflict",
                "part already registered with different payload",
            ));
        }
    }

    #[cfg(debug_assertions)]
    maybe_flip_register_part_pre_update_state(ctx, upload_id, user_id).await?;

    upsert_part_and_finalize(RegisterPartFinalizeArgs {
        db: &ctx.db,
        upload_id,
        part_number_i32,
        user_id,
        params,
        endpoint: &endpoint,
        idem_key: key,
        current_status: &upload.status,
    })
    .await
}

async fn upsert_part_and_finalize(
    args: RegisterPartFinalizeArgs<'_>,
) -> loco_rs::Result<JsonEndpointResponse> {
    let txn = args.db.begin().await.db_err()?;

    file_upload_parts::Entity::insert(file_upload_parts::ActiveModel {
        id: Set(generate_id()),
        upload_id: Set(args.upload_id),
        part_number: Set(args.part_number_i32),
        etag: Set(args.params.etag.clone()),
        size: Set(args.params.size),
        ..Default::default()
    })
    .on_conflict(
        OnConflict::columns([
            file_upload_parts::Column::UploadId,
            file_upload_parts::Column::PartNumber,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec(&txn)
    .await
    .db_err()?;

    let existing = file_upload_parts::Entity::find()
        .filter(file_upload_parts::Column::UploadId.eq(args.upload_id))
        .filter(file_upload_parts::Column::PartNumber.eq(args.part_number_i32))
        .one(&txn)
        .await
        .db_err()?
        .ok_or_else(|| {
            crate::views::errors::err_custom(
                StatusCode::CONFLICT,
                "part_conflict",
                "part registration conflicted with a concurrent request",
            )
        })?;

    if existing.etag != args.params.etag || existing.size != args.params.size {
        let _ = txn.rollback().await;
        return Err(crate::views::errors::err_custom(
            StatusCode::CONFLICT,
            "part_conflict",
            "part already registered with different payload",
        ));
    }

    let parts_received = file_upload_parts::Entity::find()
        .filter(file_upload_parts::Column::UploadId.eq(args.upload_id))
        .count(&txn)
        .await
        .db_err()? as i32;

    let next_status = if parts_received > 0 {
        STATUS_IN_PROGRESS
    } else {
        args.current_status
    };

    if guard_register_part_state_update(
        &txn,
        args.upload_id,
        args.user_id,
        parts_received,
        next_status,
    )
    .await?
        == 0
    {
        let _ = txn.rollback().await;
        return Err(crate::views::errors::err_custom(
            StatusCode::CONFLICT,
            "upload_busy",
            "upload cannot be completed while completing or completed",
        ));
    }

    let payload = RegisterPartResponse {
        upload_id: args.upload_id,
        part_number: args.part_number_i32.try_into().unwrap_or_default(),
        parts_received,
        status: next_status.to_string(),
    };
    let response = json_endpoint_response(StatusCode::OK, &payload)?;
    cache_success(
        &txn,
        args.upload_id,
        args.endpoint,
        args.idem_key,
        &response,
    )
    .await?;
    txn.commit().await.db_err()?;
    Ok(response)
}

#[must_use]
pub fn complete_upload<'a>(
    ctx: &'a AppContext,
    tenant_id: Uuid,
    user_id: Uuid,
    upload_id: Uuid,
    idempotency_key: Option<&'a str>,
    audit_ctx: &'a AuditContext,
    attach: Option<file_reference_service::AttachRequest>,
) -> Pin<Box<dyn Future<Output = loco_rs::Result<JsonEndpointResponse>> + Send + 'a>> {
    Box::pin(complete::complete_upload_inner(CompleteUploadRequest {
        ctx,
        tenant_id,
        user_id,
        upload_id,
        idempotency_key,
        audit_ctx,
        attach: attach.map(Box::new),
    }))
}
pub async fn abort_upload(
    ctx: &AppContext,
    tenant_id: Uuid,
    user_id: Uuid,
    upload_id: Uuid,
    idempotency_key: Option<&str>,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<JsonEndpointResponse> {
    let key = ensure_idempotency_key(idempotency_key)?;
    if let Some(cached) =
        find_cached_success_by_upload(&ctx.db, tenant_id, upload_id, ENDPOINT_ABORT, key)
            .await?
    {
        return Ok(cached);
    }

    let upload = load_upload(&ctx.db, tenant_id, upload_id).await?;
    if matches!(upload.status.as_str(), STATUS_ABORTED | STATUS_EXPIRED) {
        return cache_abort_response(&ctx.db, &upload, key).await;
    }

    let response = transition_abort_state(
        &ctx.db, tenant_id, user_id, upload_id, &upload, audit_ctx, key,
    )
    .await?;

    let s3_client = require_shared_s3_client(ctx)?;
    cleanup_abort_s3_objects(&s3_client, &upload).await;

    Ok(response)
}

async fn cache_abort_response(
    db: &DatabaseConnection,
    upload: &file_uploads::Model,
    idem_key: &str,
) -> loco_rs::Result<JsonEndpointResponse> {
    let payload = AbortUploadResponse {
        id: upload.id,
        status: upload.status.clone(),
    };
    let response = json_endpoint_response(StatusCode::OK, &payload)?;
    let txn = db.begin().await.db_err()?;
    cache_success(&txn, upload.id, ENDPOINT_ABORT, idem_key, &response).await?;
    txn.commit().await.db_err()?;
    Ok(response)
}

async fn transition_abort_state(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    upload_id: Uuid,
    upload: &file_uploads::Model,
    audit_ctx: &AuditContext,
    idem_key: &str,
) -> loco_rs::Result<JsonEndpointResponse> {
    let txn = db.begin().await.db_err()?;
    let update_result = file_uploads::Entity::update_many()
        .col_expr(file_uploads::Column::Status, Expr::value(STATUS_ABORTED))
        .col_expr(
            file_uploads::Column::StatusReason,
            Expr::value(ABORT_STATUS_REASON),
        )
        .col_expr(
            file_uploads::Column::S3UploadId,
            Expr::value(sea_orm::Value::String(None)),
        )
        .col_expr(file_uploads::Column::UpdatedAt, Expr::value(Utc::now()))
        .col_expr(file_uploads::Column::UpdatedBy, Expr::value(user_id))
        .filter(file_uploads::Column::Id.eq(upload_id))
        .filter(file_uploads::Column::TenantId.eq(tenant_id))
        .filter(
            file_uploads::Column::Status
                .is_in([STATUS_INITIATED.to_string(), STATUS_IN_PROGRESS.to_string()]),
        )
        .exec(&txn)
        .await
        .db_err()?;

    if update_result.rows_affected == 0 {
        let _ = txn.rollback().await;
        let current = load_upload(db, tenant_id, upload_id).await?;
        return match current.status.as_str() {
            STATUS_COMPLETING | STATUS_COMPLETED => {
                Err(crate::views::errors::err_custom(
                    StatusCode::CONFLICT,
                    "upload_busy",
                    "upload cannot be aborted while completing or completed",
                ))
            }
            STATUS_ABORTED | STATUS_EXPIRED => {
                cache_abort_response(db, &current, idem_key).await
            }
            _ => Err(crate::views::errors::err_custom(
                StatusCode::CONFLICT,
                "upload_invalid_state",
                "upload cannot be aborted in its current state",
            )),
        };
    }

    let snapshot = FileUploadAuditSnapshot::from(upload);
    audit_service::log(
        &txn,
        audit_ctx,
        AuditAction::UploadAbort,
        "file_upload",
        &upload.id.to_string(),
        Some(&snapshot),
        None::<&FileUploadAuditSnapshot>,
    )
    .await?;

    let payload = AbortUploadResponse {
        id: upload.id,
        status: STATUS_ABORTED.to_string(),
    };
    let response = json_endpoint_response(StatusCode::OK, &payload)?;
    cache_success(&txn, upload.id, ENDPOINT_ABORT, idem_key, &response).await?;
    txn.commit().await.db_err()?;

    Ok(response)
}

async fn cleanup_abort_s3_objects(
    s3_client: &SharedS3Client,
    upload: &file_uploads::Model,
) {
    abort_multipart_if_possible(
        s3_client,
        &upload.bucket,
        &upload.temp_key,
        upload.s3_upload_id.as_deref(),
    )
    .await;
    delete_temp_prefix(s3_client, &upload.bucket, upload.id).await;
    delete_object_if_exists(s3_client, &upload.bucket, &upload.temp_key).await;
}

pub async fn resume_upload(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    upload_id: Uuid,
) -> loco_rs::Result<Result<ResumeUploadResponse, ExpiredUploadResponse>> {
    let upload = load_upload(db, tenant_id, upload_id).await?;
    if upload.status == STATUS_EXPIRED {
        return Ok(Err(ExpiredUploadResponse {
            id: upload.id,
            status: "expired".to_string(),
            expired_at: upload.expired_at.ok_or_else(|| {
                Error::CustomError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetail::new(
                        "upload.missing_expired_at",
                        "expired upload missing expired_at tombstone timestamp",
                    ),
                )
            })?,
            status_reason: upload
                .status_reason
                .clone()
                .unwrap_or_else(|| PURGE_STATUS_REASON.to_string()),
        }));
    }

    if upload.status == STATUS_ABORTED || upload.status == STATUS_COMPLETED {
        return Err(crate::views::errors::err_not_found(
            "file_upload.not_found",
            "上传任务不存在或已完成",
        ));
    }

    let parts = load_parts(db, upload.id).await?;
    Ok(Ok(ResumeUploadResponse {
        id: upload.id,
        file_name: upload.file_name.clone(),
        expected_size: upload.expected_size,
        expected_hash: upload.expected_hash.clone(),
        expected_hash_algo: upload.expected_hash_algo.clone(),
        part_size: upload.part_size,
        parts_total: upload.parts_total,
        parts_received: upload.parts_received,
        status: upload.status.clone(),
        expires_at: upload.expires_at,
        uploaded_parts: parts.iter().map(UploadedPartResponse::from).collect(),
    }))
}

pub async fn purge_uploads(ctx: &AppContext) -> loco_rs::Result<UploadPurgeOutcome> {
    let s3_client = require_shared_s3_client(ctx)?;
    let now = Utc::now().fixed_offset();
    let hard_cutoff = now - ChronoDuration::days(HARD_PURGE_RETENTION_DAYS);
    let completing_cutoff = now - ChronoDuration::hours(COMPLETING_STALE_HOURS);

    let soft_targets = file_uploads::Entity::find()
        .filter(
            file_uploads::Column::Status
                .is_in([STATUS_INITIATED.to_string(), STATUS_IN_PROGRESS.to_string()]),
        )
        .filter(file_uploads::Column::ExpiresAt.lt(now))
        .all(&ctx.db)
        .await
        .db_err()?;

    let hard_targets = file_uploads::Entity::find()
        .filter(file_uploads::Column::Status.eq(STATUS_EXPIRED))
        .filter(file_uploads::Column::ExpiredAt.lt(hard_cutoff))
        .all(&ctx.db)
        .await
        .db_err()?;

    let stale_completing = file_uploads::Entity::find()
        .filter(file_uploads::Column::Status.eq(STATUS_COMPLETING))
        .filter(file_uploads::Column::UpdatedAt.lt(completing_cutoff))
        .all(&ctx.db)
        .await
        .db_err()?;

    let soft_deleted = purge_soft_expired(&s3_client, &ctx.db, &soft_targets, now).await;
    purge_stale_completing(&s3_client, ctx, &stale_completing, now).await?;
    let hard_deleted = purge_hard_expired(&ctx.db, &hard_targets).await;

    Ok(UploadPurgeOutcome {
        soft_deleted,
        hard_deleted,
    })
}

async fn purge_soft_expired(
    s3_client: &SharedS3Client,
    db: &DatabaseConnection,
    targets: &[file_uploads::Model],
    now: chrono::DateTime<FixedOffset>,
) -> u64 {
    let mut soft_deleted = 0_u64;
    for upload in targets {
        abort_multipart_if_possible(
            s3_client,
            &upload.bucket,
            &upload.temp_key,
            upload.s3_upload_id.as_deref(),
        )
        .await;
        delete_temp_prefix(s3_client, &upload.bucket, upload.id).await;
        delete_object_if_exists(s3_client, &upload.bucket, &upload.temp_key).await;

        if let Err(e) = file_uploads::Entity::update_many()
            .col_expr(file_uploads::Column::Status, Expr::value(STATUS_EXPIRED))
            .col_expr(
                file_uploads::Column::StatusReason,
                Expr::value(PURGE_STATUS_REASON),
            )
            .col_expr(file_uploads::Column::ExpiredAt, Expr::value(now))
            .col_expr(
                file_uploads::Column::S3UploadId,
                Expr::value(sea_orm::Value::String(None)),
            )
            .col_expr(file_uploads::Column::UpdatedAt, Expr::value(now))
            .filter(file_uploads::Column::Id.eq(upload.id))
            .exec(db)
            .await
        {
            tracing::error!(error = ?e, upload_id = %upload.id, "soft-expire update failed");
            continue;
        }
        soft_deleted += 1;
    }
    soft_deleted
}

async fn purge_stale_completing(
    s3_client: &SharedS3Client,
    ctx: &AppContext,
    targets: &[file_uploads::Model],
    now: chrono::DateTime<FixedOffset>,
) -> loco_rs::Result<()> {
    for upload in targets {
        let before = FileUploadAuditSnapshot::from(upload);
        let final_key = upload
            .expected_hash
            .as_deref()
            .map(|hash| final_storage_key(upload.id, hash));
        cleanup_complete_failure(
            s3_client,
            &upload.bucket,
            upload.id,
            &upload.temp_key,
            final_key.as_deref(),
        )
        .await;

        file_uploads::Entity::update_many()
            .col_expr(file_uploads::Column::Status, Expr::value(STATUS_ABORTED))
            .col_expr(
                file_uploads::Column::StatusReason,
                Expr::value(COMPLETE_STALE_ABORTED_REASON),
            )
            .col_expr(
                file_uploads::Column::S3UploadId,
                Expr::value(sea_orm::Value::String(None)),
            )
            .col_expr(file_uploads::Column::UpdatedAt, Expr::value(now))
            .filter(file_uploads::Column::Id.eq(upload.id))
            .exec(&ctx.db)
            .await
            .db_err()?;

        let mut after_upload = upload.clone();
        after_upload.status = STATUS_ABORTED.to_string();
        after_upload.status_reason = Some(COMPLETE_STALE_ABORTED_REASON.to_string());
        after_upload.s3_upload_id = None;
        after_upload.updated_at = now;

        let after = FileUploadAuditSnapshot::from(&after_upload);
        let audit_ctx = AuditContext {
            trace_id: None,
            request_id: None,
            tenant_id: upload.tenant_id,
            user_id: None,
            ip_address: None,
            user_agent: None,
        };
        audit_service::log(
            &ctx.db,
            &audit_ctx,
            AuditAction::Purge,
            "file_upload",
            &upload.id.to_string(),
            Some(&before),
            Some(&after),
        )
        .await?;
    }
    Ok(())
}

async fn purge_hard_expired(
    db: &DatabaseConnection,
    targets: &[file_uploads::Model],
) -> u64 {
    let mut hard_deleted = 0_u64;
    for upload in targets {
        let res: loco_rs::Result<()> = async {
            file_upload_idempotency::Entity::delete_many()
                .filter(file_upload_idempotency::Column::UploadId.eq(upload.id))
                .exec(db)
                .await
                .db_err()?;
            file_upload_parts::Entity::delete_many()
                .filter(file_upload_parts::Column::UploadId.eq(upload.id))
                .exec(db)
                .await
                .db_err()?;
            file_uploads::Entity::delete_by_id(upload.id)
                .exec(db)
                .await
                .db_err()?;
            Ok(())
        }
        .await;
        if let Err(e) = res {
            tracing::error!(error = ?e, upload_id = %upload.id, "hard-delete failed");
            continue;
        }
        hard_deleted += 1;
    }
    hard_deleted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> InitiateUploadRequest {
        InitiateUploadRequest {
            file_name: "video.mp4".to_string(),
            expected_size: 16 * 1024 * 1024,
            expected_hash: Some(
                "b3:dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355"
                    .to_string(),
            ),
            expected_hash_algo: "b3".to_string(),
            part_size: 8 * 1024 * 1024,
            expected_hash_fast: None,
            mime_type_hint: Some("video/mp4".to_string()),
        }
    }

    fn sample_upload(
        part_size: i64,
        parts_total: i32,
        expected_size: i64,
    ) -> file_uploads::Model {
        file_uploads::Model {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            file_name: "video.mp4".to_string(),
            mime_type_hint: Some("video/mp4".to_string()),
            expected_size,
            expected_hash: Some(
                "b3:dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355"
                    .to_string(),
            ),
            expected_hash_algo: "b3".to_string(),
            part_size,
            parts_total,
            parts_received: 0,
            storage_backend: STORAGE_BACKEND_MINIO.to_string(),
            bucket: "bucket".to_string(),
            temp_key: "uploads/1/multipart.bin".to_string(),
            s3_upload_id: Some("s3-upload-id".to_string()),
            status: STATUS_INITIATED.to_string(),
            status_reason: None,
            expires_at: Utc::now().fixed_offset(),
            expired_at: None,
            completed_file_id: None,
            expected_hash_fast: None,
            created_at: Utc::now().fixed_offset(),
            updated_at: Utc::now().fixed_offset(),
            created_by: Uuid::new_v4(),
            updated_by: Uuid::new_v4(),
        }
    }

    #[test]
    fn validate_initiate_accepts_default_part_size() {
        let mut request = sample_request();
        request.part_size = 0;

        validate_initiate_request(&request).unwrap();
    }

    #[test]
    fn validate_initiate_rejects_blacklisted_mime_hint() {
        let mut request = sample_request();
        request.mime_type_hint = Some("application/x-sh".to_string());

        let err = validate_initiate_request(&request).unwrap_err();
        let message = format!("{err:?}");
        assert!(message.contains("unsupported_media_type"));
    }

    #[test]
    fn validate_registered_size_rejects_wrong_non_final_part() {
        let upload = sample_upload(8 * 1024 * 1024, 3, 20 * 1024 * 1024);
        let err = validate_registered_size(&upload, 1, 1024).unwrap_err();
        let message = format!("{err:?}");
        assert!(message.contains("invalid_part_size"));
    }

    #[test]
    fn ensure_parts_continuity_rejects_gap() {
        let upload = sample_upload(8 * 1024 * 1024, 3, 20 * 1024 * 1024);
        let parts = vec![
            file_upload_parts::Model {
                id: Uuid::new_v4(),
                upload_id: upload.id,
                part_number: 1,
                etag: "a".to_string(),
                size: 8 * 1024 * 1024,
                created_at: Utc::now().fixed_offset(),
            },
            file_upload_parts::Model {
                id: Uuid::new_v4(),
                upload_id: upload.id,
                part_number: 3,
                etag: "b".to_string(),
                size: 4 * 1024 * 1024,
                created_at: Utc::now().fixed_offset(),
            },
        ];

        let err = ensure_parts_continuity(&upload, &parts).unwrap_err();
        let message = format!("{err:?}");
        assert!(message.contains("missing_parts") || message.contains("part_number_gap"));
    }

    #[test]
    fn final_storage_key_matches_locked_layout() {
        let file_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let key = final_storage_key(
            file_id,
            "b3:dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355",
        );

        assert_eq!(
            key,
            "files/11111111-1111-1111-1111-111111111111/dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355.bin"
        );
    }

    #[test]
    fn register_endpoint_includes_part_number() {
        assert_eq!(endpoint_register_part(7), "register-part:7");
    }
}
