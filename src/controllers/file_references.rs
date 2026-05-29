//! File-reference attach / detach / list controller — Wave 5 D3.
//!
//! Three endpoints, all under tenant scope (`/api/...`):
//!
//! - `POST   /api/files/{id}/references`  →  attach this file to a
//!   business resource. Idempotent. Body = [`AttachReferenceRequest`].
//! - `GET    /api/files/{id}/references`  →  list active references
//!   targeting this file ("X 处使用" detail drawer).
//! - `DELETE /api/file-references/{id}`   →  soft-detach a single
//!   reference by its own id.
//!
//! See [`crate::controllers::sys_file_references`] for the cross-tenant
//! mirror used by super-admin tooling.

use axum::http::StatusCode;
use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use serde::Deserialize;
use uuid::Uuid;

use crate::extractors::{RequestMeta, TenantContext};
use crate::services::{file_reference_service, resource_types::ResourceType};
use crate::views::audit_logs::AuditContext;
use crate::views::file_references::{AttachReferenceRequest, FileReferenceResponse};

/// Optional query filter for the tenant-wide list endpoint.
///
/// `resource_type` is the wire form (`"system:attachment"`,
/// `"crm:contract"`, ...). We parse it through [`ResourceType::parse`]
/// so unknown values fail closed with `400 unknown_resource_type`
/// rather than silently returning `[]`.
#[derive(Debug, Deserialize)]
pub struct ListReferencesFilter {
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

#[utoipa::path(
    post,
    path = "/api/files/{id}/references",
    tag = "文件引用",
    description = "将文件绑定到业务资源（幂等，已绑定返回原行；曾绑定后被解除则复活）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn attach(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(file_id): Path<Uuid>,
    Json(payload): Json<AttachReferenceRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
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
    path = "/api/files/{id}/references",
    tag = "文件引用",
    description = "列出文件当前所有活跃业务引用",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_by_file(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(file_id): Path<Uuid>,
) -> Result<Response> {
    let rows =
        file_reference_service::list_by_file(&ctx.db, tc.tenant_id, file_id).await?;
    let response: Vec<FileReferenceResponse> =
        rows.into_iter().map(FileReferenceResponse::from).collect();
    format::json(response)
}

#[utoipa::path(
    delete,
    path = "/api/file-references/{id}",
    tag = "文件引用",
    description = "软解除单条文件引用（幂等）",
    responses((status = 204, description = "No Content"))
)]
#[debug_handler]
pub(crate) async fn detach(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(reference_id): Path<Uuid>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    file_reference_service::detach(&ctx.db, &audit_ctx, reference_id).await?;
    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

#[utoipa::path(
    get,
    path = "/api/file-references",
    tag = "文件引用",
    description = "分页列出当前租户全部活跃文件引用（join files），可选 ?resource_type=xxx 过滤",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_for_tenant(
    tc: TenantContext,
    _meta: RequestMeta,
    State(ctx): State<AppContext>,
    Query(pagination): Query<loco_rs::prelude::model::query::PaginationQuery>,
    Query(filter): Query<ListReferencesFilter>,
) -> Result<Response> {
    let resource_type_filter = match filter.resource_type.as_deref() {
        Some(s) if !s.is_empty() => Some(parse_resource_type(s)?),
        _ => None,
    };
    let response = file_reference_service::list_for_tenant_paginated(
        &ctx.db,
        tc.tenant_id,
        resource_type_filter,
        &pagination,
    )
    .await?;
    format::json(response)
}

/// Routes mounted under `/api/files/{id}/references` and
/// `/api/file-references/{id}`. We split into two `Routes` so each
/// gets its proper prefix; the app registers both.
pub fn routes_files_subpath() -> Routes {
    Routes::new()
        .prefix("/api/files/{id}/references")
        .add("/", openapi(post(attach), routes!(attach)))
        .add("/", openapi(get(list_by_file), routes!(list_by_file)))
}

pub fn routes_root() -> Routes {
    Routes::new()
        .prefix("/api/file-references")
        .add("/", openapi(get(list_for_tenant), routes!(list_for_tenant)))
        .add("/{id}", openapi(delete(detach), routes!(detach)))
}
