//! Cross-tenant file-reference controller (super-admin) — Wave 5 D3.
//!
//! Mirrors [`crate::controllers::file_references`] under
//! `/api/sys/tenants/{tenantCode}/...`. Every handler:
//!
//! 1. Asserts `tc.is_super_admin`.
//! 2. Resolves `tenantCode` → target tenant id via
//!    [`file_service::resolve_target_tenant`].
//! 3. Builds an [`AuditContext`] whose `tenant_id` is the **target**
//!    tenant (not the caller's tenant) so the service-layer tenant
//!    filter naturally scopes to the right rows.

use axum::http::StatusCode;
use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use serde::Deserialize;
use uuid::Uuid;

use crate::extractors::{RequestMeta, TenantContext};
use crate::services::{
    file_reference_service, file_service, resource_types::ResourceType, tenant_service,
};
use crate::views::audit_logs::AuditContext;
use crate::views::file_references::{AttachReferenceRequest, FileReferenceResponse};

/// Optional query filter for sys cross-tenant list — same shape as the
/// tenant version, but the route also takes `tenantCode` from the
/// path (not the query string) per the sys convention.
#[derive(Debug, Deserialize)]
pub struct SysListReferencesFilter {
    #[serde(default)]
    pub resource_type: Option<String>,
}

fn parse_resource_type(s: &str) -> Result<ResourceType> {
    ResourceType::parse(s).map_err(|err| {
        crate::views::errors::err_custom(
            StatusCode::BAD_REQUEST,
            "unknown_resource_type",
            &err.to_string(),
        )
    })
}

fn require_super_admin(tc: &TenantContext) -> Result<()> {
    if tc.is_super_admin {
        Ok(())
    } else {
        Err(crate::views::errors::authz::err_super_admin_required())
    }
}

fn build_sys_audit_ctx(
    tc: &TenantContext,
    meta: &RequestMeta,
    tenant_id: Uuid,
) -> AuditContext {
    AuditContext {
        trace_id: Some(meta.trace_id.clone()),
        request_id: meta.request_id.clone(),
        tenant_id,
        user_id: Some(tc.user_id),
        ip_address: meta.ip_address.clone(),
        user_agent: meta.user_agent.clone(),
    }
}

#[utoipa::path(
    post,
    path = "/api/sys/tenants/{tenantCode}/files/{id}/references",
    tag = "超管-文件引用",
    description = "跨租户绑定文件到业务资源",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn sys_attach(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path((tenant_code, file_id)): Path<(String, Uuid)>,
    Json(payload): Json<AttachReferenceRequest>,
) -> Result<Response> {
    require_super_admin(&tc)?;
    let tenant = tenant_service::find_tenant_by_code(&ctx.db, &tenant_code).await?;
    if tenant.status != "active" {
        return crate::views::errors::forbidden("common.tenant_inactive", "租户已停用");
    }
    let audit_ctx = build_sys_audit_ctx(&tc, &meta, tenant.id);
    let resource_type = parse_resource_type(&payload.resource_type)?;
    let req = file_reference_service::AttachRequest {
        file_id,
        resource_type,
        resource_id: payload.resource_id,
        field_name: payload.field_name.unwrap_or_default(),
        display_name: payload.display_name,
    };
    let row = file_reference_service::attach(&ctx.db, &audit_ctx, req).await?;
    format::json(FileReferenceResponse::from(row))
}

#[utoipa::path(
    get,
    path = "/api/sys/tenants/{tenantCode}/files/{id}/references",
    tag = "超管-文件引用",
    description = "跨租户列出文件活跃引用",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn sys_list_by_file(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path((tenant_code, file_id)): Path<(String, Uuid)>,
) -> Result<Response> {
    require_super_admin(&tc)?;
    let tenant = file_service::resolve_target_tenant(&ctx.db, &tenant_code).await?;
    let rows = file_reference_service::list_by_file(&ctx.db, tenant.id, file_id).await?;
    let response: Vec<FileReferenceResponse> =
        rows.into_iter().map(FileReferenceResponse::from).collect();
    format::json(response)
}

#[utoipa::path(
    delete,
    path = "/api/sys/tenants/{tenantCode}/file-references/{id}",
    tag = "超管-文件引用",
    description = "跨租户软解除单条文件引用",
    responses((status = 204, description = "No Content"))
)]
#[debug_handler]
pub(crate) async fn sys_detach(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path((tenant_code, reference_id)): Path<(String, Uuid)>,
) -> Result<Response> {
    require_super_admin(&tc)?;
    let tenant = tenant_service::find_tenant_by_code(&ctx.db, &tenant_code).await?;
    if tenant.status != "active" {
        return crate::views::errors::forbidden("common.tenant_inactive", "租户已停用");
    }
    let audit_ctx = build_sys_audit_ctx(&tc, &meta, tenant.id);
    file_reference_service::detach(&ctx.db, &audit_ctx, reference_id).await?;
    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

pub fn routes_files_subpath() -> Routes {
    Routes::new()
        .prefix("/api/sys/tenants/{tenantCode}/files/{id}/references")
        .add("/", openapi(post(sys_attach), routes!(sys_attach)))
        .add(
            "/",
            openapi(get(sys_list_by_file), routes!(sys_list_by_file)),
        )
}

pub fn routes_root() -> Routes {
    Routes::new()
        .prefix("/api/sys/tenants/{tenantCode}/file-references")
        .add(
            "/",
            openapi(get(sys_list_for_tenant), routes!(sys_list_for_tenant)),
        )
        .add("/{id}", openapi(delete(sys_detach), routes!(sys_detach)))
}

#[utoipa::path(
    get,
    path = "/api/sys/tenants/{tenantCode}/file-references",
    tag = "超管-文件引用",
    description = "跨租户分页列出指定租户的全部活跃文件引用（join files），可选 ?resource_type=xxx 过滤",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn sys_list_for_tenant(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(tenant_code): Path<String>,
    Query(pagination): Query<loco_rs::prelude::model::query::PaginationQuery>,
    Query(filter): Query<SysListReferencesFilter>,
) -> Result<Response> {
    require_super_admin(&tc)?;
    let tenant = file_service::resolve_target_tenant(&ctx.db, &tenant_code).await?;
    let resource_type_filter = match filter.resource_type.as_deref() {
        Some(s) if !s.is_empty() => Some(parse_resource_type(s)?),
        _ => None,
    };
    let response = file_reference_service::list_for_tenant_paginated(
        &ctx.db,
        tenant.id,
        resource_type_filter,
        &pagination,
    )
    .await?;
    format::json(response)
}
