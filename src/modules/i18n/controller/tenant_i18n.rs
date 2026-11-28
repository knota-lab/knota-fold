//! Tenant-scoped i18n endpoints — tenant override CRUD.
//!
//! Current-tenant routes (tenant-admin or super-admin): operate on caller's
//! tenant via JWT-derived `tenant_id`.
//!
//! All routes mounted under Casbin authz; permissions auto-sync from URL paths.
//! Tenant cross-checks are also enforced inside the service (see
//! `delete_tenant_override_by_id`, which returns `NotFound` across tenants).

use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use uuid::Uuid;

use crate::extractors::{RequestMeta, TenantContext};
use crate::services::i18n_service;
use crate::views::audit_logs::AuditContext;
use crate::views::errors::err_bad_request;
use crate::views::i18n::{
    BatchUpdateRequest, ExportQuery, ImportRequest, KeyListParams, IMPORT_MAX_BYTES,
};

/// Query params for `DELETE …/cell` — addresses a single tenant override row
/// by its `(namespace, key, locale)` natural key. We use query string instead
/// of a path because `namespace` legitimately contains `.` (e.g.
/// `Tenant.Settings`) and `key` may contain arbitrary segment characters,
/// which complicates URL templating and percent-encoding round-trips.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteCellQuery {
    pub namespace: String,
    pub key: String,
    pub locale: String,
}

fn enforce_import_body_limit(headers: &axum::http::HeaderMap) -> Result<()> {
    if let Some(len) = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
    {
        if len > IMPORT_MAX_BYTES {
            return Err(err_bad_request(
                "i18n.import_body_too_large",
                format!("请求体 {len} bytes 超过上限 {IMPORT_MAX_BYTES}"),
            ));
        }
    }
    Ok(())
}

// ── Current-tenant routes ───────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/api/tenant/i18n/keys",
    tag = "国际化",
    description = "列出当前租户覆盖翻译 key（按 (namespace,key) 分组分页，每行附带所有 locale）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_current_tenant_keys(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(params): Query<KeyListParams>,
) -> Result<Response> {
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).clamp(1, 200);

    let resp = i18n_service::list_tenant_keys(
        &ctx.db,
        tc.tenant_id,
        params.namespace.as_deref(),
        params.q.as_deref(),
        params.empty_locale.as_deref(),
        page,
        page_size,
    )
    .await?;
    format::json(resp)
}

#[utoipa::path(
    get,
    path = "/api/tenant/i18n/namespaces",
    tag = "国际化",
    description = "列出当前租户覆盖翻译的 namespace 及其 key/locale 计数",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_current_tenant_namespaces(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let resp = i18n_service::list_tenant_namespaces(&ctx.db, tc.tenant_id).await?;
    format::json(resp)
}

#[utoipa::path(
    delete,
    path = "/api/tenant/i18n/translations/{id}",
    tag = "国际化",
    description = "删除当前租户覆盖翻译（跨租户访问返回 404）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn delete_current_tenant_override(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    i18n_service::delete_tenant_override_by_id(&ctx, id, tc.tenant_id, &audit_ctx)
        .await?;
    format::json(())
}

#[utoipa::path(
    delete,
    path = "/api/tenant/i18n/cell",
    tag = "国际化",
    description = "按 (namespace,key,locale) 删除当前租户单个覆盖翻译，cell 回退至全局值。幂等：无对应行也返回 200。",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn delete_current_tenant_cell(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Query(q): Query<DeleteCellQuery>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let removed = i18n_service::delete_tenant_override_by_triple(
        &ctx,
        tc.tenant_id,
        &q.namespace,
        &q.key,
        &q.locale,
        &audit_ctx,
    )
    .await?;
    format::json(serde_json::json!({ "removed": removed }))
}

// ── Route registration ──────────────────────────────────────────────────────

/// Current-tenant override routes (JWT-derived tenant).
pub fn tenant_routes() -> Routes {
    Routes::new()
        .prefix("/api/tenant/i18n")
        .add(
            "/keys",
            openapi(
                get(list_current_tenant_keys),
                routes!(list_current_tenant_keys),
            ),
        )
        .add(
            "/namespaces",
            openapi(
                get(list_current_tenant_namespaces),
                routes!(list_current_tenant_namespaces),
            ),
        )
        .add(
            "/translations/{id}",
            openapi(
                delete(delete_current_tenant_override),
                routes!(delete_current_tenant_override),
            ),
        )
        .add(
            "/cell",
            openapi(
                delete(delete_current_tenant_cell),
                routes!(delete_current_tenant_cell),
            ),
        )
        .add(
            "/translations/import",
            openapi(post(import_current_tenant), routes!(import_current_tenant)),
        )
        .add(
            "/translations/batch-update",
            openapi(
                post(batch_update_current_tenant),
                routes!(batch_update_current_tenant),
            ),
        )
        .add(
            "/translations/export",
            openapi(get(export_current_tenant), routes!(export_current_tenant)),
        )
}

// ── Import / export handlers ────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/tenant/i18n/translations/import",
    tag = "国际化",
    description = "批量导入当前租户覆盖翻译（≤500 entries / ≤2MB；仅 Tenant* 命名空间）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn import_current_tenant(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ImportRequest>,
) -> Result<Response> {
    enforce_import_body_limit(&headers)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let resp =
        i18n_service::import_tenant(&ctx, tc.tenant_id, tc.user_id, &req, &audit_ctx)
            .await?;
    format::json(resp)
}

#[utoipa::path(
    post,
    path = "/api/tenant/i18n/translations/batch-update",
    tag = "国际化",
    description = "批量更新已存在的租户覆盖翻译（不会创建新记录）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn batch_update_current_tenant(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    headers: axum::http::HeaderMap,
    Json(req): Json<BatchUpdateRequest>,
) -> Result<Response> {
    enforce_import_body_limit(&headers)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let resp = i18n_service::batch_update_tenant(
        &ctx,
        tc.tenant_id,
        tc.user_id,
        &req.entries,
        &audit_ctx,
    )
    .await?;
    format::json(resp)
}

#[utoipa::path(
    get,
    path = "/api/tenant/i18n/translations/export",
    tag = "国际化",
    description = "导出当前租户覆盖翻译（可按 namespace/locale 过滤）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn export_current_tenant(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(q): Query<ExportQuery>,
) -> Result<Response> {
    let resp = i18n_service::export_tenant(
        &ctx.db,
        tc.tenant_id,
        q.namespace.as_deref(),
        q.locale.as_deref(),
    )
    .await?;
    format::json(resp)
}
