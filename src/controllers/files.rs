//! Track 1 文件 CRUD / 下载 / 引用绑定 controller — Wave 2a.

use axum::{
    extract::{DefaultBodyLimit, Multipart, Path, Query},
    http::StatusCode,
    response::IntoResponse,
};
use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use uuid::Uuid;

use crate::extractors::{RequestMeta, TenantContext};
use crate::services::{
    file_reference_service, file_service, resource_types::ResourceType,
};
use crate::views::audit_logs::AuditContext;
use crate::views::errors::err_bad_request;
use crate::views::file_references::AttachReferenceRequest;
use crate::views::files::{
    DedupCheckRequest, DownloadUrlQuery, FileResponse, SmallUploadRequest,
    SoftDeleteRequest,
};

pub(crate) const MAX_SMALL_UPLOAD_BYTES: usize = 5 * 1024 * 1024;
// Allow a small envelope headroom (multipart boundary, headers, filename)
// on top of the 5 MiB file-body contract so a file exactly at 5 MiB is
// accepted at the transport layer; the handler still enforces the strict
// 5 MiB file-body cap and returns 413 for over-limit content.
pub(crate) const MAX_SMALL_UPLOAD_BODY_BYTES: usize = MAX_SMALL_UPLOAD_BYTES + 64 * 1024;

/// Translate the wire-level [`AttachReferenceRequest`] (with stringly-
/// typed `resource_type`) into the strongly-typed
/// [`file_reference_service::AttachRequest`] consumed by the service
/// layer. The `file_id` is filled in by the service after the file row
/// is materialized — callers pass [`Uuid::nil`] here.
///
/// Centralizing this conversion in the controller layer keeps the
/// service layer free of wire-format concerns and ensures
/// `resource_type` is validated *before* we open any database
/// transaction.
fn attach_to_service_request(
    payload: AttachReferenceRequest,
) -> Result<file_reference_service::AttachRequest> {
    let resource_type = ResourceType::parse(&payload.resource_type).map_err(|err| {
        crate::views::errors::err_custom(
            StatusCode::BAD_REQUEST,
            "unknown_resource_type",
            &err.to_string(),
        )
    })?;
    Ok(file_reference_service::AttachRequest {
        file_id: Uuid::nil(),
        resource_type,
        resource_id: payload.resource_id,
        field_name: payload.field_name.unwrap_or_default(),
        display_name: payload.display_name,
    })
}

#[utoipa::path(get, path = "/api/files", tag = "文件管理", description = "分页查询文件列表",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(pagination): Query<loco_rs::prelude::model::query::PaginationQuery>,
) -> Result<Response> {
    let response: crate::views::pagination::PaginatedResponse<FileResponse> =
        file_service::list_paginated(&ctx.db, tc.tenant_id, &pagination).await?;
    format::json(response)
}

#[utoipa::path(get, path = "/api/files/{id}", tag = "文件管理", description = "查询单个文件详情",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn get_one(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let response: crate::models::_entities::files::Model =
        file_service::get_by_id(&ctx.db, tc.tenant_id, id).await?;
    format::json(FileResponse::from(response))
}

#[utoipa::path(post, path = "/api/files", tag = "文件管理", description = "直接上传（小文件，multipart/form-data，设计 §8.1 L579-598）",
    responses((status = 201, description = "Created")))]
#[debug_handler]
pub(crate) async fn small_upload(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    mut multipart: Multipart,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let mut upload: Option<(String, bytes::Bytes)> = None;
    // Wave 5 D4b: optional sidecar field carrying the same payload shape
    // as `POST /api/files/{id}/references`. When present, the file row
    // and its initial business binding are created in one logical
    // operation (atomic for Insert + Revive paths; sequenced in a
    // follow-up txn for the dedup-active path — see service docs).
    let mut attach_payload: Option<AttachReferenceRequest> = None;

    while let Some(field) = multipart.next_field().await.map_err(|err| {
        tracing::error!(error = ?err, "could not read multipart field");
        err_bad_request(
            "upload.multipart_field_read_failed",
            "无法读取 multipart 字段",
        )
    })? {
        match field.name().unwrap_or("") {
            "file" => {
                if upload.is_some() {
                    return Err(err_bad_request(
                        "upload.duplicate_file_field",
                        "仅允许一个 file 字段",
                    ));
                }

                let file_name =
                    field.file_name().map(str::to_owned).ok_or_else(|| {
                        err_bad_request("upload.file_name_not_found", "文件名未找到")
                    })?;
                let file_bytes = field.bytes().await.map_err(|err| {
                    tracing::error!(error = ?err, "could not read multipart bytes");
                    err_bad_request("upload.file_bytes_read_failed", "无法读取文件数据")
                })?;

                if file_bytes.len() > MAX_SMALL_UPLOAD_BYTES {
                    return Err(crate::views::errors::err_custom(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "payload_too_large",
                        "small upload payload exceeds 5 MiB limit",
                    ));
                }

                upload = Some((file_name, file_bytes));
            }
            "attachTo" => {
                if attach_payload.is_some() {
                    return Err(err_bad_request(
                        "upload.duplicate_attach_to",
                        "仅允许一个 attachTo 字段",
                    ));
                }
                let raw = field.text().await.map_err(|err| {
                    tracing::error!(error = ?err, "could not read attachTo text");
                    err_bad_request(
                        "upload.attach_to_field_read_failed",
                        "无法读取 attachTo 字段",
                    )
                })?;
                let parsed: AttachReferenceRequest =
                    serde_json::from_str(&raw).map_err(|err| {
                        err_bad_request(
                            "upload.attach_to_invalid_json",
                            format!("attachTo 不是有效的 JSON: {err}"),
                        )
                    })?;
                attach_payload = Some(parsed);
            }
            _ => continue,
        }
    }

    let (file_name, file_bytes) = upload.ok_or_else(|| {
        err_bad_request(
            "upload.file_field_required",
            "multipart 字段 `file` 是必需的",
        )
    })?;
    // Validate `resource_type` BEFORE opening any db txn / writing S3.
    let attach = match attach_payload {
        Some(payload) => Some(attach_to_service_request(payload)?),
        None => Some(file_reference_service::default_self_attach()),
    };
    let params = SmallUploadRequest {
        name: file_name,
        mime_type_hint: None,
        attach_to: None,
    };

    let response: FileResponse = file_service::small_upload(
        &ctx,
        tc.tenant_id,
        tc.user_id,
        &params,
        file_bytes,
        &audit_ctx,
        attach,
    )
    .await?;

    Ok((StatusCode::CREATED, axum::Json(response)).into_response())
}

#[utoipa::path(post, path = "/api/files/dedup-check", tag = "文件管理", description = "秒传精确探测",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn dedup_check(
    tc: TenantContext,
    _meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<DedupCheckRequest>,
) -> Result<Response> {
    let response: crate::views::files::DedupCheckResponse =
        file_service::dedup_check(&ctx.db, tc.tenant_id, &params).await?;
    format::json(response)
}

#[utoipa::path(get, path = "/api/files/{id}/download-url", tag = "文件管理", description = "获取短期下载 URL (?disposition=inline|attachment, 默认 attachment)",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn download_url(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
    Query(query): Query<DownloadUrlQuery>,
) -> Result<Response> {
    let disposition = file_service::parse_disposition(query.disposition.as_deref())?;
    let response: crate::views::files::DownloadUrlResponse =
        file_service::get_download_url(&ctx, tc.tenant_id, id, disposition).await?;
    format::json(response)
}

#[utoipa::path(get, path = "/api/files/{id}/content", tag = "文件管理", description = "服务端代理下载",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn proxy_content(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    file_service::stream_download(&ctx, &tc, id).await
}

#[utoipa::path(delete, path = "/api/files/{id}", tag = "文件管理", description = "软删除文件",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn soft_delete(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
    Json(params): Json<SoftDeleteRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let response = file_service::soft_delete(
        &ctx.db,
        tc.tenant_id,
        tc.user_id,
        id,
        &params,
        &audit_ctx,
    )
    .await?;
    format::json(response)
}

#[utoipa::path(post, path = "/api/files/{id}/restore", tag = "文件管理", description = "恢复软删除",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn restore(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let response =
        file_service::restore(&ctx.db, tc.tenant_id, tc.user_id, id, &audit_ctx).await?;
    format::json(response)
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/files")
        .add("/", openapi(get(list), routes!(list)))
        .add(
            "/",
            // Wave 2a B3 (Oracle re-review fix): override axum's 2 MiB default
            // multipart limit so /api/files actually supports the 5 MiB
            // contract. The handler still returns 413 explicitly for parity
            // with the documented error code.
            openapi(post(small_upload), routes!(small_upload))
                .layer(DefaultBodyLimit::max(MAX_SMALL_UPLOAD_BODY_BYTES)),
        )
        .add(
            "/dedup-check",
            openapi(post(dedup_check), routes!(dedup_check)),
        )
        .add("/{id}", openapi(get(get_one), routes!(get_one)))
        .add("/{id}", openapi(delete(soft_delete), routes!(soft_delete)))
        .add("/{id}/restore", openapi(post(restore), routes!(restore)))
        .add(
            "/{id}/download-url",
            openapi(get(download_url), routes!(download_url)),
        )
        .add(
            "/{id}/content",
            openapi(get(proxy_content), routes!(proxy_content)),
        )
}
