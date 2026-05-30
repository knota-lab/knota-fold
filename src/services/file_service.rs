//! File service — Wave 2a small-upload / dedup implementation.

use std::time::Duration;

use aws_sdk_s3::{
    operation::get_object::GetObjectError, presigning::PresigningConfig,
    primitives::ByteStream,
};
use axum::{
    body::Body,
    http::{
        header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE},
        StatusCode,
    },
    response::Response,
};
use chrono::{Duration as ChronoDuration, Utc};
use loco_rs::{
    app::AppContext,
    controller::ErrorDetail,
    prelude::{model::query, *},
};
use sea_orm::{
    sea_query::{Expr, OnConflict, Query},
    ActiveModelTrait, ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection,
    DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    TransactionTrait, TryInsertResult,
};
use uuid::Uuid;

use crate::initializers::s3::{SharedS3Client, SharedS3Config};
use crate::models::{
    _entities::{file_instant_idempotency, file_references, files},
    file_repo, tenants,
};
use crate::services::{
    audit_service, file_reference_service, partition_policy, tenant_service,
};
use crate::utils::error::{db_err_into, ErrInto, IntoAppError, OptionErrInto};
use crate::utils::{
    file_hash::{build_storage_key, format_b3_hash, validate_b3_hash},
    id::generate_id,
    mime::{detect_mime, is_blacklisted},
};
use crate::views::audit_logs::{AuditAction, AuditContext, FileAuditSnapshot};
use crate::views::errors::err_bad_request;
use crate::views::file_uploads::{
    InstantUploadConfirmed, InstantUploadMiss, InstantUploadRequest,
    InstantUploadResponse, ProbeMissResponse, ProbeRequest, ProbeResponse,
    ProbeSuspectResponse, UploadFileSummary,
};
use crate::views::files::{
    DedupCheckRequest, DedupCheckResponse, DownloadUrlResponse, FileResponse,
    SmallUploadRequest, SoftDeleteRequest,
};
use crate::views::pagination::PaginatedResponse;

const ACTIVE_STATUS: &str = "ACTIVE";
const DELETED_STATUS: &str = "DELETED";
const STORAGE_BACKEND_MINIO: &str = "minio";
const CONTENT_HASH_ALGO_B3: &str = "b3";
const MAX_SMALL_UPLOAD_BYTES: usize = 5 * 1024 * 1024;
const MAX_FILE_NAME_LEN: usize = 512;
const FAST_HASH_THRESHOLD_BYTES: i64 = 32 * 1024 * 1024;
const MAX_PROXY_DOWNLOAD_BYTES: i64 = 100 * 1024 * 1024; // 100 MiB
const PROBE_BELOW_THRESHOLD_MESSAGE: &str = "Probe requires fileSize >= 32 MiB. Use /api/files (small <=5MiB) or /api/file-uploads (multipart >5MiB) directly.";
const DOWNLOAD_URL_TTL_SECONDS: u64 = 3600;
pub const GRACE_PERIOD_HOURS: i64 = 24;
/// Second grace window applied to `file_references` detachments.
///
/// A file is only purgeable when ALL its references have been detached for
/// at least this many hours, so a quick detach-then-reattach by the business
/// layer still finds the underlying object intact.
pub const REFERENCE_DETACH_GRACE_HOURS: i64 = 24;
const DEDUP_REVIVE_REASON: &str = "dedup_revive";

pub struct PurgeOutcome {
    pub purged: u64,
    pub errors: u64,
}

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

/// Disposition kind controls whether browsers render the response inline
/// (preview, e.g. images / PDFs) or force a download dialog.
///
/// Used both for the proxy-stream `Content-Disposition` header and the
/// presigned `response-content-disposition` query parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Attachment,
    Inline,
}

impl Disposition {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Attachment => "attachment",
            Self::Inline => "inline",
        }
    }
}

/// Parse the `?disposition=` query string. Defaults to `Attachment` when
/// absent. Any other value is rejected with 400 so callers can't silently
/// downgrade behaviour by typo.
pub fn parse_disposition(raw: Option<&str>) -> loco_rs::Result<Disposition> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None | Some("attachment") => Ok(Disposition::Attachment),
        Some("inline") => Ok(Disposition::Inline),
        Some(other) => Err(crate::views::errors::err_custom(
            StatusCode::BAD_REQUEST,
            "invalid_disposition",
            &format!("disposition must be 'inline' or 'attachment', got '{other}'"),
        )),
    }
}

fn build_content_disposition(name: &str, kind: Disposition) -> String {
    // Strict ASCII fallback: only printable ASCII (0x20..=0x7E) is allowed,
    // and `"` `\` `/` are additionally replaced. Control characters
    // (incl. `\r` `\n` `\t` and DEL) are stripped to prevent
    // header-injection / unsafe filename payloads being echoed via the
    // legacy `filename="..."` parameter. The canonical name still goes
    // through the RFC 8187 `filename*=UTF-8''` branch below.
    let ascii_fallback: String = name
        .chars()
        .map(|c| {
            let is_printable_ascii = (' '..='~').contains(&c);
            if is_printable_ascii && c != '"' && c != '\\' && c != '/' {
                c
            } else {
                '_'
            }
        })
        .collect();

    format!(
        "{}; filename=\"{}\"; filename*=UTF-8''{}",
        kind.as_str(),
        ascii_fallback,
        rfc8187_encode(name)
    )
}

fn rfc8187_encode(value: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(value.len() * 3);
    for byte in value.as_bytes() {
        match *byte {
            b'0'..=b'9'
            | b'A'..=b'Z'
            | b'a'..=b'z'
            | b'!'
            | b'#'
            | b'$'
            | b'&'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~' => encoded.push(*byte as char),
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

fn map_s3_error(err: &aws_sdk_s3::error::SdkError<GetObjectError>) -> Error {
    if err
        .as_service_error()
        .is_some_and(aws_sdk_s3::operation::get_object::GetObjectError::is_no_such_key)
    {
        tracing::error!(error = ?err, "S3 GetObject reported NoSuchKey for existing file row");
        crate::views::errors::err_custom(
            StatusCode::BAD_GATEWAY,
            "s3_object_missing",
            "S3 object not found for existing file row",
        )
    } else {
        // Log the full SDK error server-side; do NOT leak backend error
        // text to the client (could expose bucket / endpoint / network
        // implementation details).
        tracing::error!(error = ?err, "S3 GetObject failed");
        crate::views::errors::err_custom(
            StatusCode::BAD_GATEWAY,
            "s3_error",
            "failed to fetch object from storage",
        )
    }
}

/// Resolve `tenant_code` → tenant model for cross-tenant (sys_*) endpoints.
///
/// Only true "tenant does not exist" (`Error::NotFound` from
/// `tenant_service::find_tenant_by_code`) is rewritten to the structured
/// `404 { error: "tenant_not_found" }` Wave 2d contract response. All
/// other failures (DB outage, transport error) propagate as 5xx so we
/// don't silently mask backend incidents as 404s.
pub(crate) async fn resolve_target_tenant(
    db: &DatabaseConnection,
    tenant_code: &str,
) -> loco_rs::Result<crate::models::_entities::tenants::Model> {
    match tenant_service::find_tenant_by_code(db, tenant_code).await {
        Ok(tenant) => Ok(tenant),
        Err(Error::NotFound) => Err(crate::views::errors::err_custom(
            StatusCode::NOT_FOUND,
            "tenant_not_found",
            "target tenant not found",
        )),
        Err(e) => Err(e),
    }
}

pub fn init_runtime(ctx: &AppContext) -> loco_rs::Result<()> {
    let _ = require_shared_s3_client(ctx)?;
    let _ = require_shared_s3_config(ctx)?;
    Ok(())
}

fn validate_file_name(name: &str) -> loco_rs::Result<()> {
    if name.trim().is_empty() {
        return Err(err_bad_request("file.name_empty", "文件名不能为空"));
    }

    if name.len() > MAX_FILE_NAME_LEN {
        return Err(err_bad_request(
            "file.name_too_long",
            format!("文件名过长 (max {MAX_FILE_NAME_LEN} chars)"),
        ));
    }

    Ok(())
}

fn probe_below_threshold_error() -> Error {
    crate::views::errors::err_custom(
        StatusCode::BAD_REQUEST,
        "PROBE_BELOW_THRESHOLD",
        PROBE_BELOW_THRESHOLD_MESSAGE,
    )
}

pub async fn probe(
    ctx: &AppContext,
    tenant: &tenants::Model,
    req: &ProbeRequest,
) -> loco_rs::Result<ProbeResponse> {
    validate_file_name(&req.file_name)?;
    if req.file_size <= 0 {
        return Err(err_bad_request(
            "file.size_must_be_positive",
            "文件大小必须大于 0",
        ));
    }

    crate::views::file_uploads::validate_b3_fast_hash(
        &req.content_hash_fast,
        "contentHashFast",
    )?;

    if let Some(mime_type_hint) = req.mime_type_hint.as_deref() {
        tracing::info!(mime_type_hint, "probe mimeTypeHint is advisory only");
    }

    if req.file_size < FAST_HASH_THRESHOLD_BYTES {
        return Err(probe_below_threshold_error());
    }

    let matches = file_repo::find_active_by_fast_hash_and_size(
        &ctx.db,
        tenant.id,
        &req.content_hash_fast,
        req.file_size,
    )
    .await?;

    if matches.is_empty() {
        let policy = partition_policy::load_policy_config(&ctx.db, tenant.id).await?;
        let upload_hint = partition_policy::compute(req.file_size as u64, &policy)?;
        return Ok(ProbeResponse::Miss(ProbeMissResponse { upload_hint }));
    }

    Ok(ProbeResponse::Suspect(ProbeSuspectResponse {
        expires_at: (Utc::now() + ChronoDuration::minutes(5)).fixed_offset(),
        requires_full_hash_confirm: true,
    }))
}

fn validate_small_upload_params(
    params: &SmallUploadRequest,
    bytes: &bytes::Bytes,
) -> loco_rs::Result<()> {
    if params.name.trim().is_empty() {
        return Err(err_bad_request("file.name_empty", "文件名不能为空"));
    }

    if params.name.len() > MAX_FILE_NAME_LEN {
        return Err(err_bad_request(
            "file.name_too_long",
            format!("文件名过长 (max {MAX_FILE_NAME_LEN} chars)"),
        ));
    }

    if bytes.is_empty() {
        return Err(err_bad_request("file.content_empty", "文件内容不能为空"));
    }

    if bytes.len() > MAX_SMALL_UPLOAD_BYTES {
        return Err(crate::views::errors::err_custom(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "small upload payload exceeds 5 MiB limit",
        ));
    }

    Ok(())
}

#[tracing::instrument(skip_all)]
pub async fn get_by_id<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    file_id: Uuid,
) -> loco_rs::Result<files::Model> {
    files::Entity::find()
        .filter(files::Column::Id.eq(file_id))
        .filter(files::Column::TenantId.eq(tenant_id))
        .filter(files::Column::DeletedAt.is_null())
        .one(db)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)
}

#[tracing::instrument(skip_all)]
pub async fn list_paginated(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    pagination: &query::PaginationQuery,
) -> loco_rs::Result<PaginatedResponse<FileResponse>> {
    let base_query = files::Entity::find()
        .filter(files::Column::TenantId.eq(tenant_id))
        .filter(files::Column::DeletedAt.is_null())
        .order_by_desc(files::Column::CreatedAt);

    let page_response = query::paginate(db, base_query, None, pagination).await?;

    Ok(PaginatedResponse::from_page_response(
        &page_response,
        pagination,
        |model| FileResponse::from(model.clone()),
    ))
}

#[tracing::instrument(skip_all)]
pub async fn sys_list_paginated(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    pagination: &query::PaginationQuery,
) -> loco_rs::Result<PaginatedResponse<FileResponse>> {
    let base_query = files::Entity::find()
        .filter(files::Column::TenantId.eq(tenant_id))
        .order_by_desc(files::Column::CreatedAt);

    let page_response = query::paginate(db, base_query, None, pagination).await?;

    Ok(PaginatedResponse::from_page_response(
        &page_response,
        pagination,
        |model| FileResponse::from(model.clone()),
    ))
}

#[tracing::instrument(skip_all)]
pub async fn sys_get_by_id<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    file_id: Uuid,
) -> loco_rs::Result<files::Model> {
    files::Entity::find()
        .filter(files::Column::Id.eq(file_id))
        .filter(files::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)
}

/// POST /api/files — direct (small) upload.
#[tracing::instrument(skip_all)]
pub async fn small_upload(
    ctx: &AppContext,
    tenant_id: Uuid,
    user_id: Uuid,
    params: &SmallUploadRequest,
    bytes: bytes::Bytes,
    audit_ctx: &AuditContext,
    attach: Option<file_reference_service::AttachRequest>,
) -> loco_rs::Result<FileResponse> {
    small_upload_inner(ctx, tenant_id, user_id, params, bytes, audit_ctx, attach).await
}

#[tracing::instrument(skip_all)]
async fn small_upload_inner(
    ctx: &AppContext,
    tenant_id: Uuid,
    user_id: Uuid,
    params: &SmallUploadRequest,
    bytes: bytes::Bytes,
    audit_ctx: &AuditContext,
    attach: Option<file_reference_service::AttachRequest>,
) -> loco_rs::Result<FileResponse> {
    validate_small_upload_params(params, &bytes)?;

    let detected_mime = detect_mime(&bytes);
    if is_blacklisted(detected_mime) {
        return Err(crate::views::errors::err_custom(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "detected MIME type is blocked for upload",
        ));
    }

    let content_hash = format_b3_hash(&blake3::hash(&bytes));
    let size = i64::try_from(bytes.len())
        .map_err(|_| err_bad_request("file.size_overflow", "文件大小溢出"))?;
    let file_id = generate_id();
    let storage_key = build_storage_key(
        &content_hash,
        tenant_id,
        &ctx.environment.to_string(),
        file_id,
    )?;
    let s3_client = require_shared_s3_client(ctx)?;
    let s3_config = require_shared_s3_config(ctx)?;

    let active_model = build_file_active_model(
        file_id,
        tenant_id,
        user_id,
        params,
        detected_mime,
        size,
        &content_hash,
        &s3_config.bucket,
        &storage_key,
    );

    // PUT-first: establish object durability before DB row.
    if let Err(err) = s3_client
        .put_object()
        .bucket(s3_config.bucket.clone())
        .key(storage_key.clone())
        .body(ByteStream::from(bytes.clone()))
        .content_type(detected_mime)
        .send()
        .await
    {
        tracing::error!(error = ?err, file_id = %file_id, "failed to upload file object");
        return Err(crate::views::errors::err_custom(
            StatusCode::SERVICE_UNAVAILABLE,
            "storage_unavailable",
            "failed to store uploaded object",
        ));
    }

    let txn = match ctx.db.begin().await {
        Ok(t) => t,
        Err(e) => {
            cleanup_uploaded_object(&s3_client, &s3_config.bucket, &storage_key, file_id)
                .await;
            return Err(db_err_into(&e));
        }
    };

    let insert_result = files::Entity::insert(active_model)
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
        .exec(&txn)
        .await;

    let insert_result = match insert_result {
        Ok(r) => r,
        Err(e) => {
            let _ = txn.rollback().await;
            cleanup_uploaded_object(&s3_client, &s3_config.bucket, &storage_key, file_id)
                .await;
            return Err(db_err_into(&e));
        }
    };

    let model = match insert_result {
        TryInsertResult::Inserted(_) => {
            handle_inserted_file(
                txn,
                &InsertedFileParams {
                    s3_client: &s3_client,
                    bucket: &s3_config.bucket,
                    storage_key: &storage_key,
                    tenant_id,
                    file_id,
                    user_id,
                    audit_ctx,
                    attach: attach.as_ref(),
                    params,
                },
            )
            .await?
        }
        TryInsertResult::Conflicted => {
            // Same tenant already has a row with this (hash, size). Drop the
            // (empty) txn first, then clean up the orphan object we just PUT.
            let _ = txn.rollback().await;
            cleanup_uploaded_object(&s3_client, &s3_config.bucket, &storage_key, file_id)
                .await;
            handle_conflict_dedup(
                ctx,
                &content_hash,
                size,
                &UploadActor {
                    tenant_id,
                    user_id,
                    audit_ctx,
                },
                attach.as_ref(),
                params,
            )
            .await?
        }
        TryInsertResult::Empty => {
            let _ = txn.rollback().await;
            cleanup_uploaded_object(&s3_client, &s3_config.bucket, &storage_key, file_id)
                .await;
            return Err(Error::CustomError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetail::new(
                    "file.empty_insert",
                    "file insert unexpectedly produced an empty insert statement",
                ),
            ));
        }
    };

    Ok(FileResponse::from(model))
}

async fn cleanup_uploaded_object(
    client: &SharedS3Client,
    bucket: &str,
    key: &str,
    file_id: Uuid,
) {
    if let Err(cleanup_err) = client
        .delete_object()
        .bucket(bucket.to_string())
        .key(key.to_string())
        .send()
        .await
    {
        tracing::error!(
            error = ?cleanup_err,
            file_id = %file_id,
            key = %key,
            "failed to delete just-uploaded object after DB conflict/error"
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn build_file_active_model(
    file_id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
    params: &SmallUploadRequest,
    detected_mime: &str,
    size: i64,
    content_hash: &str,
    bucket: &str,
    storage_key: &str,
) -> files::ActiveModel {
    files::ActiveModel {
        id: ActiveValue::Set(file_id),
        tenant_id: ActiveValue::Set(tenant_id),
        name: ActiveValue::Set(params.name.clone()),
        mime_type: ActiveValue::Set(detected_mime.to_string()),
        size: ActiveValue::Set(size),
        content_hash: ActiveValue::Set(content_hash.to_string()),
        content_hash_algo: ActiveValue::Set(CONTENT_HASH_ALGO_B3.to_string()),
        content_hash_fast: ActiveValue::Set(None),
        storage_backend: ActiveValue::Set(STORAGE_BACKEND_MINIO.to_string()),
        bucket: ActiveValue::Set(bucket.to_string()),
        storage_key: ActiveValue::Set(storage_key.to_string()),
        multipart_upload_id: ActiveValue::Set(None),
        status: ActiveValue::Set(ACTIVE_STATUS.to_string()),
        status_reason: ActiveValue::Set(None),
        deleted_at: ActiveValue::Set(None),
        purge_at: ActiveValue::Set(None),
        deleted_by: ActiveValue::Set(None),
        uploaded_by: ActiveValue::Set(user_id),
        created_by: ActiveValue::Set(user_id),
        updated_by: ActiveValue::Set(user_id),
        ..Default::default()
    }
}

/// Parameters for [`handle_inserted_file`].
struct InsertedFileParams<'a> {
    s3_client: &'a SharedS3Client,
    bucket: &'a str,
    storage_key: &'a str,
    tenant_id: Uuid,
    file_id: Uuid,
    user_id: Uuid,
    audit_ctx: &'a AuditContext,
    attach: Option<&'a file_reference_service::AttachRequest>,
    params: &'a SmallUploadRequest,
}

/// Handle the `TryInsertResult::Inserted` branch: read back the row, audit,
/// optionally attach in the same txn, and commit. On failure, rollback and
/// clean up the S3 object.
async fn handle_inserted_file(
    txn: DatabaseTransaction,
    p: &InsertedFileParams<'_>,
) -> loco_rs::Result<files::Model> {
    let s3_client = p.s3_client;
    let bucket = p.bucket;
    let storage_key = p.storage_key;
    let tenant_id = p.tenant_id;
    let file_id = p.file_id;
    let _ = p.user_id;
    let audit_ctx = p.audit_ctx;
    let attach = p.attach;
    let params = p.params;
    let inserted = match get_by_id(&txn, tenant_id, file_id).await {
        Ok(m) => m,
        Err(e) => {
            let _ = txn.rollback().await;
            cleanup_uploaded_object(s3_client, bucket, storage_key, file_id).await;
            return Err(e);
        }
    };
    let snapshot = FileAuditSnapshot::from(&inserted);
    if let Err(audit_err) = audit_service::log(
        &txn,
        audit_ctx,
        AuditAction::Create,
        "file",
        &inserted.id.to_string(),
        None::<&FileAuditSnapshot>,
        Some(&snapshot),
    )
    .await
    {
        let _ = txn.rollback().await;
        cleanup_uploaded_object(s3_client, bucket, storage_key, file_id).await;
        return Err(audit_err);
    }
    // Wave 5 D4 — Path 1 (Insert): same-txn attach.
    if let Some(attach_req) = attach {
        let mut req = attach_req.clone();
        req.file_id = inserted.id;
        if let Err(attach_err) =
            file_reference_service::attach_in_txn(&txn, audit_ctx, req).await
        {
            let _ = txn.rollback().await;
            cleanup_uploaded_object(s3_client, bucket, storage_key, file_id).await;
            return Err(attach_err);
        }
    }
    if let Err(commit_err) = txn.commit().await {
        cleanup_uploaded_object(s3_client, bucket, storage_key, file_id).await;
        return Err(db_err_into(&commit_err));
    }
    let _ = params;
    Ok(inserted)
}

/// Identity and audit context shared by upload helper functions.
struct UploadActor<'a> {
    tenant_id: Uuid,
    user_id: Uuid,
    audit_ctx: &'a AuditContext,
}

/// Handle the `TryInsertResult::Conflicted` branch: find the existing winner,
/// revive if soft-deleted, and optionally attach.
async fn handle_conflict_dedup(
    ctx: &AppContext,
    content_hash: &str,
    size: i64,
    actor: &UploadActor<'_>,
    attach: Option<&file_reference_service::AttachRequest>,
    params: &SmallUploadRequest,
) -> loco_rs::Result<files::Model> {
    let winner = find_any_by_hash(&ctx.db, actor.tenant_id, content_hash)
        .await?
        .filter(|model| model.size == size)
        .ok_or_else(|| {
            Error::CustomError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetail::new(
                    "file.dedup_conflict_winner_missing",
                    "duplicate file row not found after conflict",
                ),
            )
        })?;

    if winner.deleted_at.is_none() {
        return Ok(winner);
    }

    let revive_txn = ctx.db.begin().await.db_err()?;
    let current = sys_get_by_id(&revive_txn, actor.tenant_id, winner.id).await?;
    if current.deleted_at.is_none() {
        revive_txn.rollback().await.db_err()?;
        return Ok(current);
    }

    let purge_at = current.purge_at.ok_or_else(|| {
        crate::views::errors::err_custom(
            StatusCode::GONE,
            "grace_expired",
            "file restore grace period has expired",
        )
    })?;
    let now = Utc::now().fixed_offset();
    if now >= purge_at {
        revive_txn.rollback().await.db_err()?;
        return Err(crate::views::errors::err_custom(
            StatusCode::GONE,
            "grace_expired",
            "file restore grace period has expired",
        ));
    }

    let before = FileAuditSnapshot::from(&current);
    let mut active_model: files::ActiveModel = current.into();
    active_model.status = ActiveValue::Set(ACTIVE_STATUS.to_string());
    active_model.deleted_at = ActiveValue::Set(None);
    active_model.purge_at = ActiveValue::Set(None);
    active_model.deleted_by = ActiveValue::Set(None);
    active_model.status_reason = ActiveValue::Set(Some(DEDUP_REVIVE_REASON.to_string()));
    active_model.name = ActiveValue::Set(params.name.clone());
    active_model.updated_at = ActiveValue::Set(now);
    active_model.updated_by = ActiveValue::Set(actor.user_id);
    let revived = active_model.update(&revive_txn).await.db_err()?;
    let after = FileAuditSnapshot::from(&revived);
    audit_service::log(
        &revive_txn,
        actor.audit_ctx,
        AuditAction::Restore,
        "file",
        &revived.id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await?;
    // Wave 5 D4 — Path 3 (Revive): same-txn attach.
    if let Some(attach_req) = attach {
        let mut req = attach_req.clone();
        req.file_id = revived.id;
        if let Err(attach_err) =
            file_reference_service::attach_in_txn(&revive_txn, actor.audit_ctx, req).await
        {
            let _ = revive_txn.rollback().await;
            return Err(attach_err);
        }
    }
    revive_txn.commit().await.db_err()?;
    Ok(revived)
}

#[tracing::instrument(skip_all)]
pub async fn dedup_check(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    params: &DedupCheckRequest,
) -> loco_rs::Result<DedupCheckResponse> {
    dedup_check_inner(db, tenant_id, params).await
}

#[tracing::instrument(skip_all)]
async fn dedup_check_inner(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    params: &DedupCheckRequest,
) -> loco_rs::Result<DedupCheckResponse> {
    validate_b3_hash(&params.content_hash)?;

    let file = find_active_by_hash(db, tenant_id, &params.content_hash)
        .await?
        .filter(|model| model.size == params.size)
        .map(FileResponse::from);

    Ok(DedupCheckResponse {
        hit: file.is_some(),
        file,
    })
}

/// Parameters for [`sys_small_upload`].
pub struct SysSmallUploadParams<'a> {
    pub tc: &'a crate::extractors::TenantContext,
    pub tenant_code: &'a str,
    pub params: &'a SmallUploadRequest,
    pub bytes: bytes::Bytes,
    pub audit_ctx: &'a AuditContext,
    pub attach: Option<file_reference_service::AttachRequest>,
}

#[tracing::instrument(skip_all)]
pub async fn sys_small_upload(
    db: &DatabaseConnection,
    ctx: &AppContext,
    p: &SysSmallUploadParams<'_>,
) -> loco_rs::Result<FileResponse> {
    let tenant = resolve_target_tenant(db, p.tenant_code).await?;
    small_upload_inner(
        ctx,
        tenant.id,
        p.tc.user_id,
        p.params,
        p.bytes.clone(),
        p.audit_ctx,
        p.attach.clone(),
    )
    .await
}

#[tracing::instrument(skip_all)]
pub async fn sys_dedup_check(
    db: &DatabaseConnection,
    _tc: &crate::extractors::TenantContext,
    tenant_code: &str,
    params: &DedupCheckRequest,
) -> loco_rs::Result<DedupCheckResponse> {
    let tenant = resolve_target_tenant(db, tenant_code).await?;
    dedup_check_inner(db, tenant.id, params).await
}

#[tracing::instrument(skip_all)]
pub async fn soft_delete(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    file_id: Uuid,
    params: &SoftDeleteRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<FileResponse> {
    let txn = db.begin().await.db_err()?;
    let model = files::Entity::find()
        .filter(files::Column::Id.eq(file_id))
        .filter(files::Column::TenantId.eq(tenant_id))
        .one(&txn)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    if model.deleted_at.is_some() || model.status == DELETED_STATUS {
        return Err(crate::views::errors::err_custom(
            StatusCode::CONFLICT,
            "already_deleted",
            "file is already deleted",
        ));
    }

    let before = FileAuditSnapshot::from(&model);
    let now = Utc::now().fixed_offset();
    let mut active_model: files::ActiveModel = model.into();
    active_model.status = ActiveValue::Set(DELETED_STATUS.to_string());
    active_model.deleted_at = ActiveValue::Set(Some(now));
    active_model.purge_at =
        ActiveValue::Set(Some(now + ChronoDuration::hours(GRACE_PERIOD_HOURS)));
    active_model.deleted_by = ActiveValue::Set(Some(user_id));
    active_model.status_reason = ActiveValue::Set(params.reason.clone());
    active_model.updated_at = ActiveValue::Set(now);
    active_model.updated_by = ActiveValue::Set(user_id);
    let updated = active_model.update(&txn).await.db_err()?;
    let after = FileAuditSnapshot::from(&updated);

    audit_service::log(
        &txn,
        audit_ctx,
        AuditAction::Delete,
        "file",
        &updated.id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await?;

    txn.commit().await.db_err()?;
    Ok(FileResponse::from(updated))
}

#[tracing::instrument(skip_all)]
pub async fn restore(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    file_id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<FileResponse> {
    let txn = db.begin().await.db_err()?;
    let model = files::Entity::find()
        .filter(files::Column::Id.eq(file_id))
        .filter(files::Column::TenantId.eq(tenant_id))
        .one(&txn)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    if model.deleted_at.is_none() || model.status != DELETED_STATUS {
        return Err(crate::views::errors::err_custom(
            StatusCode::CONFLICT,
            "not_deleted",
            "file is not deleted",
        ));
    }

    let purge_at = model.purge_at.ok_or_else(|| {
        crate::views::errors::err_custom(
            StatusCode::GONE,
            "grace_expired",
            "file restore grace period has expired",
        )
    })?;
    let now = Utc::now().fixed_offset();
    if now >= purge_at {
        return Err(crate::views::errors::err_custom(
            StatusCode::GONE,
            "grace_expired",
            "file restore grace period has expired",
        ));
    }

    let before = FileAuditSnapshot::from(&model);
    let mut active_model: files::ActiveModel = model.into();
    active_model.status = ActiveValue::Set(ACTIVE_STATUS.to_string());
    active_model.deleted_at = ActiveValue::Set(None);
    active_model.purge_at = ActiveValue::Set(None);
    active_model.deleted_by = ActiveValue::Set(None);
    active_model.status_reason = ActiveValue::Set(None);
    active_model.updated_at = ActiveValue::Set(now);
    active_model.updated_by = ActiveValue::Set(user_id);
    let updated = active_model.update(&txn).await.db_err()?;
    let after = FileAuditSnapshot::from(&updated);

    audit_service::log(
        &txn,
        audit_ctx,
        AuditAction::Restore,
        "file",
        &updated.id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await?;

    txn.commit().await.db_err()?;
    Ok(FileResponse::from(updated))
}

#[tracing::instrument(skip_all)]
pub async fn get_download_url(
    ctx: &AppContext,
    tenant_id: Uuid,
    file_id: Uuid,
    disposition: Disposition,
) -> loco_rs::Result<DownloadUrlResponse> {
    let file = files::Entity::find()
        .filter(files::Column::Id.eq(file_id))
        .filter(files::Column::TenantId.eq(tenant_id))
        .filter(files::Column::DeletedAt.is_null())
        .filter(files::Column::Status.eq(ACTIVE_STATUS))
        .one(&ctx.db)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;
    let s3_client = require_shared_s3_client(ctx)?;

    let presign_config =
        PresigningConfig::expires_in(Duration::from_secs(DOWNLOAD_URL_TTL_SECONDS))
            .err_info(crate::error_info::common::DB_ERROR)?;

    // Override the response Content-Type / Content-Disposition headers via
    // S3 `response-content-*` query parameters so the browser surfaces the
    // original file name + MIME — without these, the storage layer echoes
    // the raw `.bin` CAS object key and `application/octet-stream`.
    let presigned_request = s3_client
        .get_object()
        .bucket(file.bucket.clone())
        .key(file.storage_key.clone())
        .response_content_type(file.mime_type.clone())
        .response_content_disposition(build_content_disposition(&file.name, disposition))
        .presigned(presign_config)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, file_id = %file_id, "failed to presign download url");
            crate::views::errors::err_custom(
                StatusCode::SERVICE_UNAVAILABLE,
                "storage_unavailable",
                "failed to generate download url",
            )
        })?;

    Ok(DownloadUrlResponse {
        url: presigned_request.uri().to_string(),
        // Wave 2a: 1-hour TTL per plan §3 contract.
        expires_at: (Utc::now()
            + ChronoDuration::seconds(DOWNLOAD_URL_TTL_SECONDS as i64))
        .fixed_offset(),
    })
}

#[tracing::instrument(skip_all)]
pub async fn sys_get_download_url(
    ctx: &AppContext,
    tenant_id: Uuid,
    file_id: Uuid,
    disposition: Disposition,
) -> loco_rs::Result<DownloadUrlResponse> {
    let file = files::Entity::find()
        .filter(files::Column::Id.eq(file_id))
        .filter(files::Column::TenantId.eq(tenant_id))
        .one(&ctx.db)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;
    let s3_client = require_shared_s3_client(ctx)?;

    let presign_config =
        PresigningConfig::expires_in(Duration::from_secs(DOWNLOAD_URL_TTL_SECONDS))
            .err_info(crate::error_info::common::DB_ERROR)?;

    let presigned_request = s3_client
        .get_object()
        .bucket(file.bucket.clone())
        .key(file.storage_key.clone())
        .response_content_type(file.mime_type.clone())
        .response_content_disposition(build_content_disposition(&file.name, disposition))
        .presigned(presign_config)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, file_id = %file_id, "failed to presign sys download url");
            crate::views::errors::err_custom(
                StatusCode::SERVICE_UNAVAILABLE,
                "storage_unavailable",
                "failed to generate download url",
            )
        })?;

    Ok(DownloadUrlResponse {
        url: presigned_request.uri().to_string(),
        expires_at: (Utc::now()
            + ChronoDuration::seconds(DOWNLOAD_URL_TTL_SECONDS as i64))
        .fixed_offset(),
    })
}

#[tracing::instrument(skip_all)]
pub async fn sys_soft_delete(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    file_id: Uuid,
    params: &SoftDeleteRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<FileResponse> {
    soft_delete(db, tenant_id, user_id, file_id, params, audit_ctx).await
}

#[tracing::instrument(skip_all)]
pub async fn sys_restore(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    file_id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<FileResponse> {
    restore(db, tenant_id, user_id, file_id, audit_ctx).await
}

#[tracing::instrument(skip_all)]
pub async fn stream_download(
    ctx: &AppContext,
    tc: &crate::extractors::TenantContext,
    file_id: Uuid,
) -> loco_rs::Result<Response> {
    let row = get_by_id(&ctx.db, tc.tenant_id, file_id).await?;
    stream_download_inner(ctx, row).await
}

#[tracing::instrument(skip_all)]
pub async fn sys_stream_download(
    ctx: &AppContext,
    _tc: &crate::extractors::TenantContext,
    tenant_code: &str,
    file_id: Uuid,
) -> loco_rs::Result<Response> {
    let tenant = resolve_target_tenant(&ctx.db, tenant_code).await?;
    let row = sys_get_by_id(&ctx.db, tenant.id, file_id).await?;
    stream_download_inner(ctx, row).await
}

#[tracing::instrument(skip_all)]
async fn stream_download_inner(
    ctx: &AppContext,
    row: files::Model,
) -> loco_rs::Result<Response> {
    if row.size > MAX_PROXY_DOWNLOAD_BYTES {
        return Err(crate::views::errors::err_custom(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file_too_large_for_proxy",
            "File too large for proxy download, use download-url endpoint instead",
        ));
    }

    let s3 = require_shared_s3_client(ctx)?;
    let resp = s3
        .get_object()
        .bucket(&row.bucket)
        .key(&row.storage_key)
        .send()
        .await
        .map_err(|e| map_s3_error(&e))?;

    let async_read = resp.body.into_async_read();
    let stream = tokio_util::io::ReaderStream::with_capacity(async_read, 64 * 1024);
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, row.mime_type)
        .header(CONTENT_LENGTH, row.size.to_string())
        .header(
            CONTENT_DISPOSITION,
            build_content_disposition(&row.name, Disposition::Attachment),
        )
        .body(body)
        .err_info(crate::error_info::common::DB_ERROR)
}

#[tracing::instrument(skip_all)]
pub async fn purge_files(ctx: &AppContext) -> loco_rs::Result<PurgeOutcome> {
    let now = Utc::now().fixed_offset();
    let s3_client = require_shared_s3_client(ctx)?;

    // Reference-safety filter: skip any file that still has a file_reference
    // row that is either active (deleted_at IS NULL) or detached more
    // recently than REFERENCE_DETACH_GRACE_HOURS ago. Combined with the
    // existing files.purge_at filter (set at soft_delete time to
    // deleted_at + GRACE_PERIOD_HOURS) this implements the documented
    // dual-grace contract: a file is purgeable only when (a) it has been
    // soft-deleted past its own grace AND (b) every reference to it has
    // been detached for the reference grace window.
    let reference_grace_cutoff =
        now - ChronoDuration::hours(REFERENCE_DETACH_GRACE_HOURS);
    let blocking_refs_subquery =
        Query::select()
            .column(file_references::Column::FileId)
            .from(file_references::Entity)
            .and_where(Expr::col(file_references::Column::DeletedAt).is_null().or(
                Expr::col(file_references::Column::DeletedAt).gt(reference_grace_cutoff),
            ))
            .to_owned();

    let targets = files::Entity::find()
        .filter(files::Column::Status.eq(DELETED_STATUS))
        .filter(files::Column::PurgeAt.lte(now))
        .filter(Expr::col(files::Column::Id).not_in_subquery(blocking_refs_subquery))
        .order_by_asc(files::Column::PurgeAt)
        .limit(200)
        .all(&ctx.db)
        .await
        .db_err()?;

    let mut purged = 0_u64;
    let mut errors = 0_u64;
    for file in targets {
        if purge_single_file(ctx, &s3_client, &file).await {
            purged += 1;
        } else {
            errors += 1;
        }
    }

    Ok(PurgeOutcome { purged, errors })
}

/// Returns `true` if purged successfully, `false` on error (error already logged).
async fn purge_single_file(
    ctx: &AppContext,
    s3_client: &SharedS3Client,
    file: &files::Model,
) -> bool {
    if let Err(err) = s3_client
        .delete_object()
        .bucket(file.bucket.clone())
        .key(file.storage_key.clone())
        .send()
        .await
    {
        tracing::warn!(error = ?err, file_id = %file.id, "purge_files failed to delete object; skipping DB row delete");
        return false;
    }

    let txn = match ctx.db.begin().await {
        Ok(txn) => txn,
        Err(err) => {
            tracing::error!(error = ?err, file_id = %file.id, "purge_files failed to open transaction");
            return false;
        }
    };

    let current = match files::Entity::find_by_id(file.id).one(&txn).await {
        Ok(Some(current)) => current,
        Ok(None) => {
            let _ = txn.rollback().await;
            tracing::warn!(file_id = %file.id, "purge_files target row disappeared before DB delete");
            return false;
        }
        Err(err) => {
            let _ = txn.rollback().await;
            tracing::error!(error = ?err, file_id = %file.id, "purge_files failed to reload target row");
            return false;
        }
    };
    let before = FileAuditSnapshot::from(&current);

    let delete_result = match files::Entity::delete_many()
        .filter(files::Column::Id.eq(file.id))
        .exec(&txn)
        .await
    {
        Ok(result) if result.rows_affected == 1 => result,
        Ok(_) => {
            let _ = txn.rollback().await;
            tracing::error!(file_id = %file.id, "purge_files did not delete target row");
            return false;
        }
        Err(err) => {
            let _ = txn.rollback().await;
            tracing::error!(error = ?err, file_id = %file.id, "purge_files failed to delete row");
            return false;
        }
    };

    let audit_ctx = AuditContext {
        trace_id: None,
        request_id: None,
        tenant_id: file.tenant_id,
        user_id: None,
        ip_address: None,
        user_agent: None,
    };
    if let Err(err) = audit_service::log(
        &txn,
        &audit_ctx,
        AuditAction::Purge,
        "file",
        &file.id.to_string(),
        Some(&before),
        None::<&FileAuditSnapshot>,
    )
    .await
    {
        let _ = txn.rollback().await;
        tracing::error!(error = ?err, file_id = %file.id, rows_affected = delete_result.rows_affected, "purge_files failed to audit row deletion");
        return false;
    }

    if let Err(err) = txn.commit().await {
        tracing::error!(error = ?err, file_id = %file.id, "purge_files failed to commit transaction");
        return false;
    }

    true
}

#[tracing::instrument(skip_all)]
pub async fn find_active_by_hash(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    content_hash: &str,
) -> loco_rs::Result<Option<files::Model>> {
    files::Entity::find()
        .filter(files::Column::TenantId.eq(tenant_id))
        .filter(files::Column::ContentHash.eq(content_hash))
        .filter(files::Column::DeletedAt.is_null())
        .order_by_desc(files::Column::CreatedAt)
        .one(db)
        .await
        .db_err()
}

#[tracing::instrument(skip_all)]
pub async fn find_any_by_hash(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    content_hash: &str,
) -> loco_rs::Result<Option<files::Model>> {
    files::Entity::find()
        .filter(files::Column::TenantId.eq(tenant_id))
        .filter(files::Column::ContentHash.eq(content_hash))
        .order_by_desc(files::Column::CreatedAt)
        .one(db)
        .await
        .db_err()
}

/// Given a candidate `winner` row found by hash+size lookup (which may be
/// either active or soft-deleted), return a usable active file row.
///
/// - If the winner is already active, returned as-is with `revived = false`.
/// - If soft-deleted and still within the grace window, the row is restored
///   in its own transaction (status, `deleted_at`, `purge_at`, `deleted_by`,
///   `status_reason` cleared; `updated_at/by` stamped) and an `AuditAction::Restore`
///   entry is written. Returned with `revived = true`.
/// - If soft-deleted and grace has expired, returns a `gone` error so the
///   caller can fall back to a fresh upload.
///
/// This helper deliberately does **not** mutate the winner's `name`: instant-
/// upload (秒传) is a deduplication primitive, business-display names live on
/// `file_references.display_name` and are owned by the attach API.
///
/// The caller MUST re-fetch the row inside the helper's own transaction by
/// passing the `db` pool (not an outer transaction) - this protects against
/// races where another request restored or re-deleted the row between the
/// initial lookup and our update.
#[tracing::instrument(skip_all, fields(file_id = %winner.id))]
pub async fn revive_or_use_winner(
    db: &DatabaseConnection,
    winner: files::Model,
    user_id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<(files::Model, bool)> {
    if winner.deleted_at.is_none() {
        return Ok((winner, false));
    }

    let tenant_id = winner.tenant_id;
    let file_id = winner.id;

    let revive_txn = db.begin().await.db_err()?;
    let current = sys_get_by_id(&revive_txn, tenant_id, file_id).await?;

    if current.deleted_at.is_none() {
        // Raced with another revive - good enough, drop our txn and use it.
        revive_txn.rollback().await.db_err()?;
        return Ok((current, false));
    }

    let purge_at = current.purge_at.ok_or_else(|| {
        crate::views::errors::err_custom(
            StatusCode::GONE,
            "grace_expired",
            "file restore grace period has expired",
        )
    })?;
    let now = Utc::now().fixed_offset();
    if now >= purge_at {
        revive_txn.rollback().await.db_err()?;
        return Err(crate::views::errors::err_custom(
            StatusCode::GONE,
            "grace_expired",
            "file restore grace period has expired",
        ));
    }

    let before = FileAuditSnapshot::from(&current);
    let mut active_model: files::ActiveModel = current.into();
    active_model.status = ActiveValue::Set(ACTIVE_STATUS.to_string());
    active_model.deleted_at = ActiveValue::Set(None);
    active_model.purge_at = ActiveValue::Set(None);
    active_model.deleted_by = ActiveValue::Set(None);
    active_model.status_reason = ActiveValue::Set(Some(DEDUP_REVIVE_REASON.to_string()));
    active_model.updated_at = ActiveValue::Set(now);
    active_model.updated_by = ActiveValue::Set(user_id);
    let revived = active_model.update(&revive_txn).await.db_err()?;
    let after = FileAuditSnapshot::from(&revived);
    audit_service::log(
        &revive_txn,
        audit_ctx,
        AuditAction::Restore,
        "file",
        &revived.id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await?;
    revive_txn.commit().await.db_err()?;

    Ok((revived, true))
}

// =============================================================================
// instant_upload (秒传) - Wave 4
// =============================================================================

const INSTANT_UPLOAD_FAST_HASH_MISMATCH_REASON: &str = "fast_hash_mismatch";

/// Build a canonical `UploadFileSummary` from a `files::Model` using the same
/// `From` impl that `complete_upload` uses; ensures the JSON shape (and `status`
/// case) is identical across multipart-complete and instant-upload responses.
fn upload_summary_from(model: &files::Model) -> UploadFileSummary {
    UploadFileSummary::from(model)
}

/// Look up a cached instant-upload response keyed by
/// `(tenant_id, user_id, expected_hash, idempotency_key)` and revalidate the
/// referenced `file_id` is still active for this tenant before returning.
///
/// If the cached file row has since been hard-purged or moved to a different
/// tenant, the cache entry is silently treated as a miss so the caller can
/// re-resolve from the live `files` table.
async fn lookup_instant_upload_cache(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    expected_hash: &str,
    idempotency_key: &str,
) -> loco_rs::Result<Option<InstantUploadResponse>> {
    let Some(cached) = file_instant_idempotency::Entity::find_by_id((
        tenant_id,
        user_id,
        expected_hash.to_string(),
        idempotency_key.to_string(),
    ))
    .one(db)
    .await
    .db_err()?
    else {
        return Ok(None);
    };

    let cached_id: Uuid = serde_json::from_slice(&cached.response_body)
        .err_info(crate::error_info::common::DB_ERROR)?;

    let Some(model) = files::Entity::find_by_id(cached_id)
        .filter(files::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .db_err()?
    else {
        return Ok(None);
    };

    if model.deleted_at.is_some() {
        // Cached row was soft-deleted again after our previous Confirmed
        // response. Treat as cache miss so the caller re-runs the revive
        // pipeline and produces a fresh response.
        return Ok(None);
    }

    Ok(Some(InstantUploadResponse::Confirmed(
        InstantUploadConfirmed {
            file: upload_summary_from(&model),
            revived: false,
        },
    )))
}

/// Best-effort cache write of a Confirmed instant-upload response.
/// Conflicts (concurrent identical request) are swallowed; downstream
/// reads will hit whichever row landed first.
async fn store_instant_upload_cache(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    expected_hash: &str,
    idempotency_key: &str,
    file_id: Uuid,
) -> loco_rs::Result<()> {
    let body =
        serde_json::to_vec(&file_id).err_info(crate::error_info::common::DB_ERROR)?;
    let _ =
        file_instant_idempotency::Entity::insert(file_instant_idempotency::ActiveModel {
            tenant_id: ActiveValue::Set(tenant_id),
            user_id: ActiveValue::Set(user_id),
            expected_hash: ActiveValue::Set(expected_hash.to_string()),
            idempotency_key: ActiveValue::Set(idempotency_key.to_string()),
            response_body: ActiveValue::Set(body),
            status_code: ActiveValue::Set(200),
            created_at: ActiveValue::Set(Utc::now().fixed_offset()),
        })
        .on_conflict(
            OnConflict::columns([
                file_instant_idempotency::Column::TenantId,
                file_instant_idempotency::Column::UserId,
                file_instant_idempotency::Column::ExpectedHash,
                file_instant_idempotency::Column::IdempotencyKey,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec(db)
        .await;
    Ok(())
}

/// Client-driven instant-upload (秒传) endpoint.
///
/// Preconditions: the client has already received a `Suspect` verdict from
/// `/probe` and computed the full BLAKE3 hash. We attempt to dedup against an
/// existing same-tenant file row (active or soft-deleted within grace) keyed
/// by `(content_hash, size)`. On hit, no bytes are uploaded and the existing
/// row is returned (revived if necessary). On miss, a `Miss` response with a
/// fresh `upload_hint` is returned so the client can fall back to the standard
/// multipart flow.
///
/// Idempotency: `(tenant_id, user_id, expected_hash, idempotency_key)` cache
/// guarantees safe retries. The cache stores only the resolved `file_id` and
/// re-validates liveness on each replay.
#[tracing::instrument(skip_all, fields(tenant = %tenant.id, user = %user_id))]
pub async fn instant_upload(
    ctx: &AppContext,
    tenant: &tenants::Model,
    user_id: Uuid,
    req: &InstantUploadRequest,
    idempotency_key: &str,
    audit_ctx: &AuditContext,
    // Wave 5 D4c: optional attach payload — applied even on a dedup hit
    // (the explicit user-confirmed semantics: "复用命中也 attach"), so
    // that two business resources sharing the same content both end up
    // bound to the single underlying file row.
    //
    // Attach happens AFTER `revive_or_use_winner` returns (the file row
    // is already committed by then), so this is sequenced rather than
    // atomic. On attach failure we surface the error; the file row
    // stays as-is (it pre-existed or was successfully revived).
    attach: Option<file_reference_service::AttachRequest>,
) -> loco_rs::Result<InstantUploadResponse> {
    // ---- validation ----
    validate_file_name(&req.file_name)?;
    if req.expected_size <= 0 {
        return Err(err_bad_request(
            "file.expected_size_must_be_positive",
            "expectedSize 必须大于 0",
        ));
    }
    if req.expected_hash_algo != CONTENT_HASH_ALGO_B3 {
        return Err(crate::views::errors::err_custom(
            StatusCode::BAD_REQUEST,
            "unsupported_hash_algo",
            "expectedHashAlgo must be 'b3'",
        ));
    }
    validate_b3_hash(&req.expected_hash)?;
    crate::views::file_uploads::validate_b3_fast_hash(
        &req.expected_hash_fast,
        "expectedHashFast",
    )?;
    if idempotency_key.trim().is_empty() {
        return Err(crate::views::errors::err_custom(
            StatusCode::BAD_REQUEST,
            "missing_idempotency_key",
            "Idempotency-Key header is required",
        ));
    }

    let tenant_id = tenant.id;

    // ---- idempotency replay ----
    if let Some(cached) = lookup_instant_upload_cache(
        &ctx.db,
        tenant_id,
        user_id,
        &req.expected_hash,
        idempotency_key,
    )
    .await?
    {
        return Ok(cached);
    }

    // ---- dedup lookup (active OR soft-deleted within grace) ----
    let Some(winner) = file_repo::find_any_by_hash_and_size(
        &ctx.db,
        tenant_id,
        &req.expected_hash,
        req.expected_size,
    )
    .await?
    else {
        // Miss: client must fall back to multipart upload.
        let policy = partition_policy::load_policy_config(&ctx.db, tenant_id).await?;
        let upload_hint = partition_policy::compute(req.expected_size as u64, &policy)?;
        return Ok(InstantUploadResponse::Miss(InstantUploadMiss {
            upload_hint,
        }));
    };

    // ---- defensive fast-hash cross-check ----
    // The probe→instant-upload contract assumes the client's expected_hash_fast
    // matches what we stored on the original ingest. If a stored row's
    // `content_hash_fast` is non-NULL and disagrees, the client is replaying
    // probe results against a different file - reject explicitly rather than
    // silently dedup the wrong content.
    if let Some(stored_fast) = winner.content_hash_fast.as_deref() {
        if stored_fast != req.expected_hash_fast {
            tracing::warn!(
                file_id = %winner.id,
                stored_fast,
                client_fast = %req.expected_hash_fast,
                "instant_upload fast-hash mismatch against winner row"
            );
            return Err(crate::views::errors::err_custom(
                StatusCode::UNPROCESSABLE_ENTITY,
                INSTANT_UPLOAD_FAST_HASH_MISMATCH_REASON,
                "expectedHashFast does not match stored fast hash for this content",
            ));
        }
    }

    // ---- revive if needed, else return active winner ----
    let (active, revived) =
        revive_or_use_winner(&ctx.db, winner, user_id, audit_ctx).await?;

    // ---- Wave 5 D4c: attach (post-confirm) ----
    // Sequenced after the file row is guaranteed active. Idempotent in
    // the service layer, so even if the caller retries with the same
    // Idempotency-Key (which short-circuits earlier and skips this), an
    // accidental duplicate attach is a no-op rather than an error.
    if let Some(attach_req) = attach {
        let mut req = attach_req;
        req.file_id = active.id;
        file_reference_service::attach(&ctx.db, audit_ctx, req).await?;
    }

    // ---- cache (best effort) ----
    store_instant_upload_cache(
        &ctx.db,
        tenant_id,
        user_id,
        &req.expected_hash,
        idempotency_key,
        active.id,
    )
    .await?;

    Ok(InstantUploadResponse::Confirmed(InstantUploadConfirmed {
        file: upload_summary_from(&active),
        revived,
    }))
}
