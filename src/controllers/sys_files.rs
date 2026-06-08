//! Track 2 跨租户文件 controller（超管） — Wave 1 骨架。
//!
//! 与 `files.rs` 一一对应，handler 名加 `sys_` 前缀。
//! 路径多 `/{tenantCode}` 一段。
//! 业务反查也并入本文件，单独 `business_routes()` 暴露。

use axum::{
    extract::{DefaultBodyLimit, Multipart, Path, Query},
    http::StatusCode,
};
use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use uuid::Uuid;

use crate::controllers::files::MAX_SMALL_UPLOAD_BODY_BYTES;
use crate::extractors::{RequestMeta, TenantContext};
use crate::services::{
    file_reference_service, file_service, resource_types::ResourceType,
};
use crate::views::audit_logs::AuditContext;
use crate::views::errors::err_bad_request;
use crate::views::file_references::AttachReferenceRequest;
use crate::views::files::{
    DedupCheckRequest, DownloadUrlQuery, SmallUploadRequest, SoftDeleteRequest,
};
use crate::views::pagination::PaginationParams;

/// Sys-side mirror of `controllers::files::attach_to_service_request`.
/// Kept colocated to avoid a cross-module helper export and to make
/// `resource_type` validation visible in the cross-tenant attack
/// surface.
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
        mime_type: payload.mime_type,
    })
}

struct SmallUploadMultipartParts {
    file_name: String,
    file_bytes: bytes::Bytes,
    attach_payload: Option<AttachReferenceRequest>,
    mime_type_hint: Option<String>,
}

async fn parse_small_upload_multipart(
    mut multipart: Multipart,
) -> Result<SmallUploadMultipartParts> {
    let mut upload: Option<(String, bytes::Bytes)> = None;
    let mut attach_payload: Option<AttachReferenceRequest> = None;
    let mut mime_type_hint: Option<String> = None;
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
            "mimeTypeHint" => {
                mime_type_hint = Some(field.text().await.map_err(|err| {
                    tracing::error!(error = ?err, "could not read mimeTypeHint text");
                    err_bad_request(
                        "upload.mime_type_hint_read_failed",
                        "无法读取 mimeTypeHint 字段",
                    )
                })?);
            }
            _ => {}
        }
    }

    let (file_name, file_bytes) = upload.ok_or_else(|| {
        err_bad_request(
            "upload.file_field_required",
            "multipart 字段 `file` 是必需的",
        )
    })?;
    Ok(SmallUploadMultipartParts {
        file_name,
        file_bytes,
        attach_payload,
        mime_type_hint,
    })
}

#[utoipa::path(get, path = "/api/sys/tenants/{tenantCode}/files", tag = "超管-文件管理", description = "跨租户分页查询",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn sys_list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(tenant_code): Path<String>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }
    let pagination = pagination.into();
    let tenant = file_service::resolve_target_tenant(&ctx.db, &tenant_code).await?;
    let response =
        file_service::sys_list_paginated(&ctx.db, tenant.id, &pagination).await?;
    format::json(response)
}

#[utoipa::path(get, path = "/api/sys/tenants/{tenantCode}/files/{id}", tag = "超管-文件管理", description = "跨租户查询单个",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn sys_get_one(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path((tenant_code, id)): Path<(String, Uuid)>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }
    let tenant = file_service::resolve_target_tenant(&ctx.db, &tenant_code).await?;
    let response = file_service::sys_get_by_id(&ctx.db, tenant.id, id).await?;
    format::json(crate::views::files::FileResponse::from(response))
}

#[utoipa::path(post, path = "/api/sys/tenants/{tenantCode}/files", tag = "超管-文件管理", description = "跨租户直接上传（multipart/form-data，设计 §8.1 L579-598）",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn sys_small_upload(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(tenant_code): Path<String>,
    multipart: Multipart,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    // Resolve the target tenant via the shared Wave 2d helper so that
    // missing-tenant lookups consistently return the structured
    // `404 { error: "tenant_not_found" }` response, and so that real
    // backend failures still surface as 5xx (rather than being flattened
    // by the legacy direct `find_tenant_by_code` call site).
    let tenant = file_service::resolve_target_tenant(&ctx.db, &tenant_code).await?;
    let audit_ctx = AuditContext {
        trace_id: Some(meta.trace_id.clone()),
        request_id: meta.request_id.clone(),
        tenant_id: tenant.id,
        user_id: Some(tc.user_id),
        ip_address: meta.ip_address.clone(),
        user_agent: meta.user_agent.clone(),
    };

    let parts = parse_small_upload_multipart(multipart).await?;
    let attach = match parts.attach_payload {
        Some(payload) => Some(attach_to_service_request(payload)?),
        None => Some(file_reference_service::default_self_attach()),
    };
    let params = SmallUploadRequest {
        name: parts.file_name,
        mime_type_hint: parts.mime_type_hint,
        attach_to: None,
    };
    let response = file_service::sys_small_upload(
        &ctx.db,
        &ctx,
        &file_service::SysSmallUploadParams {
            tc: &tc,
            tenant_code: &tenant_code,
            params: &params,
            bytes: parts.file_bytes,
            audit_ctx: &audit_ctx,
            attach,
        },
    )
    .await?;

    Ok((StatusCode::CREATED, axum::Json(response)).into_response())
}

#[utoipa::path(post, path = "/api/sys/tenants/{tenantCode}/files/dedup-check", tag = "超管-文件管理", description = "跨租户秒传探测",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn sys_dedup_check(
    tc: TenantContext,
    _meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(tenant_code): Path<String>,
    Json(params): Json<DedupCheckRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }
    let response =
        file_service::sys_dedup_check(&ctx.db, &tc, &tenant_code, &params).await?;
    format::json(response)
}

#[utoipa::path(get, path = "/api/sys/tenants/{tenantCode}/files/{id}/download-url", tag = "超管-文件管理", description = "跨租户下载 URL (?disposition=inline|attachment, 默认 attachment)",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn sys_download_url(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path((tenant_code, id)): Path<(String, Uuid)>,
    Query(query): Query<DownloadUrlQuery>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }
    let disposition = file_service::parse_disposition(query.disposition.as_deref())?;
    let tenant = file_service::resolve_target_tenant(&ctx.db, &tenant_code).await?;
    let response =
        file_service::sys_get_download_url(&ctx, tenant.id, id, disposition).await?;
    format::json(response)
}

#[utoipa::path(get, path = "/api/sys/tenants/{tenantCode}/files/{id}/content", tag = "超管-文件管理", description = "跨租户代理下载",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn sys_proxy_content(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path((tenant_code, id)): Path<(String, Uuid)>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }
    file_service::sys_stream_download(&ctx, &tc, &tenant_code, id).await
}

#[utoipa::path(delete, path = "/api/sys/tenants/{tenantCode}/files/{id}", tag = "超管-文件管理", description = "跨租户软删除",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn sys_soft_delete(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path((tenant_code, id)): Path<(String, Uuid)>,
    Json(params): Json<SoftDeleteRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }
    let tenant = file_service::resolve_target_tenant(&ctx.db, &tenant_code).await?;
    let audit_ctx = AuditContext {
        trace_id: Some(meta.trace_id.clone()),
        request_id: meta.request_id.clone(),
        tenant_id: tenant.id,
        user_id: Some(tc.user_id),
        ip_address: meta.ip_address.clone(),
        user_agent: meta.user_agent.clone(),
    };
    let response = file_service::sys_soft_delete(
        &ctx.db, tenant.id, tc.user_id, id, &params, &audit_ctx,
    )
    .await?;
    format::json(response)
}

#[utoipa::path(post, path = "/api/sys/tenants/{tenantCode}/files/{id}/restore", tag = "超管-文件管理", description = "跨租户恢复",
    responses((status = 200, description = "Success")))]
#[debug_handler]
pub(crate) async fn sys_restore(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path((tenant_code, id)): Path<(String, Uuid)>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }
    let tenant = file_service::resolve_target_tenant(&ctx.db, &tenant_code).await?;
    let audit_ctx = AuditContext {
        trace_id: Some(meta.trace_id.clone()),
        request_id: meta.request_id.clone(),
        tenant_id: tenant.id,
        user_id: Some(tc.user_id),
        ip_address: meta.ip_address.clone(),
        user_agent: meta.user_agent.clone(),
    };
    let response =
        file_service::sys_restore(&ctx.db, tenant.id, tc.user_id, id, &audit_ctx).await?;
    format::json(response)
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/sys/tenants/{tenantCode}/files")
        .add("/", openapi(get(sys_list), routes!(sys_list)))
        .add(
            "/",
            openapi(post(sys_small_upload), routes!(sys_small_upload))
                .layer(DefaultBodyLimit::max(MAX_SMALL_UPLOAD_BODY_BYTES)),
        )
        .add(
            "/dedup-check",
            openapi(post(sys_dedup_check), routes!(sys_dedup_check)),
        )
        .add("/{id}", openapi(get(sys_get_one), routes!(sys_get_one)))
        .add(
            "/{id}",
            openapi(delete(sys_soft_delete), routes!(sys_soft_delete)),
        )
        .add(
            "/{id}/restore",
            openapi(post(sys_restore), routes!(sys_restore)),
        )
        .add(
            "/{id}/download-url",
            openapi(get(sys_download_url), routes!(sys_download_url)),
        )
        .add(
            "/{id}/content",
            openapi(get(sys_proxy_content), routes!(sys_proxy_content)),
        )
}
