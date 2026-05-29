use axum::{
    extract::{DefaultBodyLimit, Path},
    http::header,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use uuid::Uuid;

use crate::utils::error::IntoModelResult;
use crate::{
    extractors::{RequestMeta, TenantContext},
    models::tenants,
    services::file_upload_service::JsonEndpointResponse,
    services::{
        file_reference_service, file_service, file_upload_service,
        resource_types::ResourceType,
    },
    views::{
        audit_logs::AuditContext,
        file_references::AttachReferenceRequest,
        file_uploads::{
            CompleteUploadRequest, ExpiredUploadResponse, InitiateUploadRequest,
            InstantUploadRequest, InstantUploadResponse, ProbeRequest, ProbeResponse,
            RegisterPartRequest, ResumeUploadResponse, SignPartResponse,
        },
    },
};

const JSON_BODY_LIMIT_BYTES: usize = 64 * 1024;

fn idempotency_key(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
}

/// Wave 5 D4c/d: translate a wire `AttachReferenceRequest` (used as
/// `attachTo` sidecar in JSON or multipart) into the strongly-typed
/// service request. `file_id` is filled in by the service after the
/// file row is materialized.
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

fn json_endpoint_response(response: JsonEndpointResponse) -> Response {
    (
        response.status_code,
        [(header::CONTENT_TYPE, "application/json")],
        response.body_bytes,
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/api/file-uploads",
    tag = "文件上传",
    description = "初始化分片上传会话"
)]
#[debug_handler]
pub(crate) async fn initiate(
    tc: TenantContext,
    _meta: RequestMeta,
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Json(params): Json<InitiateUploadRequest>,
) -> Result<Response> {
    let response = file_upload_service::initiate_upload(
        &ctx,
        tc.tenant_id,
        tc.user_id,
        &params,
        idempotency_key(&headers),
    )
    .await?;

    Ok(json_endpoint_response(response))
}

#[utoipa::path(
    post,
    path = "/api/file-uploads/probe",
    tag = "文件上传",
    description = "快速哈希探测上传提示",
    request_body = ProbeRequest,
    responses((status = 200, description = "Success", body = ProbeResponse))
)]
#[debug_handler]
pub(crate) async fn probe(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<ProbeRequest>,
) -> Result<Response> {
    let tenant = tenants::Model::find_by_id(&ctx.db, tc.tenant_id)
        .await
        .model_err()?;
    let response = file_service::probe(&ctx, &tenant, &params).await?;
    format::json(response)
}

#[utoipa::path(
    post,
    path = "/api/file-uploads/instant-upload",
    tag = "文件上传",
    description = "客户端驱动的秒传 (full-hash 去重)",
    request_body = InstantUploadRequest,
    responses((status = 200, description = "Success", body = InstantUploadResponse))
)]
#[debug_handler]
pub(crate) async fn instant_upload(
    tc: TenantContext,
    meta: RequestMeta,
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Json(params): Json<InstantUploadRequest>,
) -> Result<Response> {
    let tenant = tenants::Model::find_by_id(&ctx.db, tc.tenant_id)
        .await
        .model_err()?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    // Validate `attachTo.resource_type` BEFORE entering the service so
    // we never run the dedup probe + cache lookup just to fail late on
    // a typo'd resource type.
    let attach = match params.attach_to.clone() {
        Some(payload) => Some(attach_to_service_request(payload)?),
        None => Some(file_reference_service::default_self_attach()),
    };
    let response = file_service::instant_upload(
        &ctx,
        &tenant,
        tc.user_id,
        &params,
        idempotency_key(&headers).unwrap_or(""),
        &audit_ctx,
        attach,
    )
    .await?;
    format::json(response)
}

#[utoipa::path(
    post,
    path = "/api/file-uploads/{id}/parts/{partNumber}/sign",
    tag = "文件上传",
    description = "申请分片预签名 URL"
)]
#[debug_handler]
pub(crate) async fn sign_part(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path((id, part_number)): Path<(Uuid, u32)>,
    Json(_params): Json<serde_json::Value>,
) -> Result<Response> {
    let response: SignPartResponse =
        file_upload_service::sign_part(&ctx, tc.tenant_id, id, part_number).await?;
    format::json(response)
}

#[utoipa::path(
    post,
    path = "/api/file-uploads/{id}/parts/{partNumber}/register",
    tag = "文件上传",
    description = "登记分片上传结果"
)]
#[debug_handler]
pub(crate) async fn register_part(
    tc: TenantContext,
    _meta: RequestMeta,
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path((id, part_number)): Path<(Uuid, u32)>,
    Json(params): Json<RegisterPartRequest>,
) -> Result<Response> {
    let response = file_upload_service::register_part(
        &ctx,
        tc.tenant_id,
        tc.user_id,
        id,
        part_number,
        &params,
        idempotency_key(&headers),
    )
    .await?;
    Ok(json_endpoint_response(response))
}

#[utoipa::path(
    post,
    path = "/api/file-uploads/{id}/complete",
    tag = "文件上传",
    description = "完成分片上传"
)]
#[debug_handler]
pub(crate) async fn complete(
    tc: TenantContext,
    meta: RequestMeta,
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
    Json(params): Json<CompleteUploadRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    // Validate attachTo BEFORE any DB I/O so unknown resource_type
    // short-circuits with 400 instead of running through the full
    // multipart finalize. Mirrors the small/instant upload entry points.
    let attach = match params.attach_to {
        Some(payload) => Some(attach_to_service_request(payload)?),
        None => Some(file_reference_service::default_self_attach()),
    };
    let response = file_upload_service::complete_upload(
        &ctx,
        tc.tenant_id,
        tc.user_id,
        id,
        idempotency_key(&headers),
        &audit_ctx,
        attach,
    )
    .await?;
    Ok(json_endpoint_response(response))
}

#[utoipa::path(
    delete,
    path = "/api/file-uploads/{id}",
    tag = "文件上传",
    description = "中止上传会话"
)]
#[debug_handler]
pub(crate) async fn abort(
    tc: TenantContext,
    meta: RequestMeta,
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let response = file_upload_service::abort_upload(
        &ctx,
        tc.tenant_id,
        tc.user_id,
        id,
        idempotency_key(&headers),
        &audit_ctx,
    )
    .await?;
    Ok(json_endpoint_response(response))
}

#[utoipa::path(
    get,
    path = "/api/file-uploads/{id}",
    tag = "文件上传",
    description = "恢复分片上传状态"
)]
#[debug_handler]
pub(crate) async fn resume(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    match file_upload_service::resume_upload(&ctx.db, tc.tenant_id, id).await? {
        Ok(response) => {
            let response: ResumeUploadResponse = response;
            format::json(response)
        }
        Err(response) => {
            let response: ExpiredUploadResponse = response;
            Ok((StatusCode::GONE, axum::Json(response)).into_response())
        }
    }
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/file-uploads")
        .add(
            "/",
            openapi(post(initiate), routes!(initiate))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/probe",
            openapi(post(probe), routes!(probe))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/instant-upload",
            openapi(post(instant_upload), routes!(instant_upload))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/{id}/parts/{partNumber}/sign",
            openapi(post(sign_part), routes!(sign_part))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/{id}/parts/{partNumber}/register",
            openapi(post(register_part), routes!(register_part))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/{id}/complete",
            openapi(post(complete), routes!(complete))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/{id}",
            openapi(delete(abort), routes!(abort))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add("/{id}", openapi(get(resume), routes!(resume)))
}
