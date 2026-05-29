use axum::{
    extract::{DefaultBodyLimit, Path},
    http::{header, HeaderMap, StatusCode},
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

/// Sys-side mirror of
/// `controllers::file_uploads::attach_to_service_request`. Validation
/// runs in this layer so cross-tenant `attachTo` payloads with
/// unknown `resource_type` short-circuit before any DB I/O.
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

fn ensure_super_admin(tc: &TenantContext) -> Result<()> {
    if !tc.is_super_admin {
        return Err(crate::views::errors::authz::err_super_admin_required());
    }

    Ok(())
}

async fn ensure_active_tenant_by_id(
    ctx: &AppContext,
    tenant_id: Uuid,
) -> Result<tenants::Model> {
    let tenant = tenants::Model::find_by_id(&ctx.db, tenant_id)
        .await
        .model_err()?;
    if tenant.status != "active" {
        return Err(crate::views::errors::err_forbidden(
            "common.tenant_inactive",
            "租户已停用",
        ));
    }

    Ok(tenant)
}

#[utoipa::path(
    post,
    path = "/api/sys/tenants/{tenantId}/file-uploads",
    tag = "超管-文件上传",
    description = "跨租户初始化"
)]
#[debug_handler]
pub(crate) async fn sys_initiate(
    tc: TenantContext,
    _meta: RequestMeta,
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path(tenant_id): Path<Uuid>,
    Json(params): Json<InitiateUploadRequest>,
) -> Result<Response> {
    ensure_super_admin(&tc)?;
    let _tenant = ensure_active_tenant_by_id(&ctx, tenant_id).await?;
    let response = file_upload_service::initiate_upload(
        &ctx,
        tenant_id,
        tc.user_id,
        &params,
        idempotency_key(&headers),
    )
    .await?;

    Ok(json_endpoint_response(response))
}

#[utoipa::path(
    post,
    path = "/api/sys/tenants/{tenantId}/file-uploads/probe",
    tag = "超管-文件上传",
    description = "跨租户快速哈希探测",
    request_body = ProbeRequest,
    responses((status = 200, description = "Success", body = ProbeResponse))
)]
#[debug_handler]
pub(crate) async fn sys_probe(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(tenant_id): Path<Uuid>,
    Json(params): Json<ProbeRequest>,
) -> Result<Response> {
    ensure_super_admin(&tc)?;
    let tenant = ensure_active_tenant_by_id(&ctx, tenant_id).await?;
    let response = file_service::probe(&ctx, &tenant, &params).await?;
    format::json(response)
}

#[utoipa::path(
    post,
    path = "/api/sys/tenants/{tenantId}/file-uploads/instant-upload",
    tag = "超管-文件上传",
    description = "跨租户客户端驱动秒传",
    request_body = InstantUploadRequest,
    responses((status = 200, description = "Success", body = InstantUploadResponse))
)]
#[debug_handler]
pub(crate) async fn sys_instant_upload(
    tc: TenantContext,
    meta: RequestMeta,
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path(tenant_id): Path<Uuid>,
    Json(params): Json<InstantUploadRequest>,
) -> Result<Response> {
    ensure_super_admin(&tc)?;
    let tenant = ensure_active_tenant_by_id(&ctx, tenant_id).await?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
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
    path = "/api/sys/tenants/{tenantId}/file-uploads/{id}/parts/{partNumber}/sign",
    tag = "超管-文件上传",
    description = "跨租户分片预签名"
)]
#[debug_handler]
pub(crate) async fn sys_sign_part(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path((tenant_id, id, part_number)): Path<(Uuid, Uuid, u32)>,
    Json(_params): Json<serde_json::Value>,
) -> Result<Response> {
    ensure_super_admin(&tc)?;
    let response: SignPartResponse =
        file_upload_service::sign_part(&ctx, tenant_id, id, part_number).await?;
    format::json(response)
}

#[utoipa::path(
    post,
    path = "/api/sys/tenants/{tenantId}/file-uploads/{id}/parts/{partNumber}/register",
    tag = "超管-文件上传",
    description = "跨租户登记分片上传结果"
)]
#[debug_handler]
pub(crate) async fn sys_register_part(
    tc: TenantContext,
    _meta: RequestMeta,
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path((tenant_id, id, part_number)): Path<(Uuid, Uuid, u32)>,
    Json(params): Json<RegisterPartRequest>,
) -> Result<Response> {
    ensure_super_admin(&tc)?;
    let _tenant = ensure_active_tenant_by_id(&ctx, tenant_id).await?;
    let response = file_upload_service::register_part(
        &ctx,
        tenant_id,
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
    path = "/api/sys/tenants/{tenantId}/file-uploads/{id}/complete",
    tag = "超管-文件上传",
    description = "跨租户完成分片"
)]
#[debug_handler]
pub(crate) async fn sys_complete(
    tc: TenantContext,
    meta: RequestMeta,
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path((tenant_id, id)): Path<(Uuid, Uuid)>,
    Json(params): Json<CompleteUploadRequest>,
) -> Result<Response> {
    ensure_super_admin(&tc)?;
    let _tenant = ensure_active_tenant_by_id(&ctx, tenant_id).await?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let attach = match params.attach_to {
        Some(payload) => Some(attach_to_service_request(payload)?),
        None => Some(file_reference_service::default_self_attach()),
    };
    let response = file_upload_service::complete_upload(
        &ctx,
        tenant_id,
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
    path = "/api/sys/tenants/{tenantId}/file-uploads/{id}",
    tag = "超管-文件上传",
    description = "跨租户中止会话"
)]
#[debug_handler]
pub(crate) async fn sys_abort(
    tc: TenantContext,
    meta: RequestMeta,
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path((tenant_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Response> {
    ensure_super_admin(&tc)?;
    let _tenant = ensure_active_tenant_by_id(&ctx, tenant_id).await?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let response = file_upload_service::abort_upload(
        &ctx,
        tenant_id,
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
    path = "/api/sys/tenants/{tenantId}/file-uploads/{id}",
    tag = "超管-文件上传",
    description = "跨租户恢复分片上传状态"
)]
#[debug_handler]
pub(crate) async fn sys_resume(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path((tenant_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Response> {
    ensure_super_admin(&tc)?;
    match file_upload_service::resume_upload(&ctx.db, tenant_id, id).await? {
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
        .prefix("/api/sys/tenants/{tenantId}/file-uploads")
        .add(
            "/",
            openapi(post(sys_initiate), routes!(sys_initiate))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/probe",
            openapi(post(sys_probe), routes!(sys_probe))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/instant-upload",
            openapi(post(sys_instant_upload), routes!(sys_instant_upload))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/{id}/parts/{partNumber}/sign",
            openapi(post(sys_sign_part), routes!(sys_sign_part))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/{id}/parts/{partNumber}/register",
            openapi(post(sys_register_part), routes!(sys_register_part))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/{id}/complete",
            openapi(post(sys_complete), routes!(sys_complete))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add(
            "/{id}",
            openapi(delete(sys_abort), routes!(sys_abort))
                .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES)),
        )
        .add("/{id}", openapi(get(sys_resume), routes!(sys_resume)))
}
