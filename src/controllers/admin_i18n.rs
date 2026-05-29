//! Admin-only i18n endpoints — global locales + global translations CRUD.
//!
//! All routes mounted under Casbin authz; permissions auto-sync from URL paths.
//! Only super-admins ("SUPER_ADMIN" role) should be granted these in seed data.
//!
//! Endpoints:
//!
//! - `GET    /api/admin/i18n/locales`              — list all locales (admin view)
//! - `POST   /api/admin/i18n/locales`              — create locale
//! - `PATCH  /api/admin/i18n/locales/{locale}`     — update locale (label/enabled/sort)
//! - `DELETE /api/admin/i18n/locales/{locale}`     — delete locale (base locale forbidden)
//! - `GET    /api/admin/i18n/keys`                 — list global keys (paged, locales bundled)
//! - `GET    /api/admin/i18n/namespaces`           — list namespaces with key/locale counts
//! - `POST   /api/admin/i18n/translations`         — upsert global translation
//! - `PATCH  /api/admin/i18n/translations/{id}`    — update value by id
//! - `DELETE /api/admin/i18n/translations/{id}`    — delete by id

use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use uuid::Uuid;

use crate::extractors::{RequestMeta, TenantContext};
use crate::services::{i18n_locale_service, i18n_service};
use crate::views::audit_logs::AuditContext;
use crate::views::errors::err_bad_request;
use crate::views::i18n::{
    BatchUpdateRequest, CreateLocaleRequest, ExportQuery, ImportRequest, KeyListParams,
    UpdateLocaleRequest, UpdateTranslationRequest, UpsertGlobalTranslationRequest,
    IMPORT_MAX_BYTES,
};

/// Reject the request early if `Content-Length` exceeds the i18n import cap.
/// Bodies without `Content-Length` are allowed through; the JSON parser will
/// still fail on absurdly large payloads, just less gracefully.
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

// ── Locale CRUD ─────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/api/admin/i18n/locales",
    tag = "国际化",
    description = "[超管] 列出全部 locale（含未启用）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_locales(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let items = i18n_locale_service::list_all(&ctx).await?;
    format::json(items)
}

#[utoipa::path(
    post,
    path = "/api/admin/i18n/locales",
    tag = "国际化",
    description = "[超管] 创建 locale",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn create_locale(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateLocaleRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let model =
        i18n_locale_service::create(&ctx, tc.user_id, &params, &audit_ctx).await?;
    format::json(model)
}

#[utoipa::path(
    patch,
    path = "/api/admin/i18n/locales/{locale}",
    tag = "国际化",
    description = "[超管] 更新 locale（label/启用/排序）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn update_locale(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(locale): Path<String>,
    Json(params): Json<UpdateLocaleRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let model =
        i18n_locale_service::update(&ctx, &locale, tc.user_id, &params, &audit_ctx)
            .await?;
    format::json(model)
}

#[utoipa::path(
    delete,
    path = "/api/admin/i18n/locales/{locale}",
    tag = "国际化",
    description = "[超管] 删除 locale（base locale 不可删）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn delete_locale(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(locale): Path<String>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    i18n_locale_service::delete(&ctx, &locale, &audit_ctx).await?;
    format::json(())
}

// ── Global translations ─────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/api/admin/i18n/keys",
    tag = "国际化",
    description = "[超管] 列出全局翻译 key（按 (namespace,key) 分组分页，每行附带所有 locale）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_global_keys(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(params): Query<KeyListParams>,
) -> Result<Response> {
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).clamp(1, 200);

    let resp = i18n_service::list_global_keys(
        &ctx.db,
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
    path = "/api/admin/i18n/namespaces",
    tag = "国际化",
    description = "[超管] 列出所有全局 namespace 及其 key/locale 计数（用于矩阵管理 UI 的父行）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_global_namespaces(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let resp = i18n_service::list_namespaces(&ctx.db, None).await?;
    format::json(resp)
}

#[utoipa::path(
    post,
    path = "/api/admin/i18n/translations",
    tag = "国际化",
    description = "[超管] Upsert 全局翻译（按 namespace/key/locale 唯一）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn upsert_global_translation(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<UpsertGlobalTranslationRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let resp =
        i18n_service::upsert_global_translation(&ctx, tc.user_id, &params, &audit_ctx)
            .await?;
    format::json(resp)
}

#[utoipa::path(
    patch,
    path = "/api/admin/i18n/translations/{id}",
    tag = "国际化",
    description = "[超管] 按 id 更新全局翻译 value",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn update_global_translation(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
    Json(params): Json<UpdateTranslationRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let resp = i18n_service::update_global_translation_by_id(
        &ctx, id, tc.user_id, &params, &audit_ctx,
    )
    .await?;
    format::json(resp)
}

#[utoipa::path(
    delete,
    path = "/api/admin/i18n/translations/{id}",
    tag = "国际化",
    description = "[超管] 按 id 删除全局翻译",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn delete_global_translation(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    i18n_service::delete_global_translation_by_id(&ctx, id, &audit_ctx).await?;
    format::json(())
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/admin/i18n")
        .add(
            "/locales",
            openapi(get(list_locales), routes!(list_locales)),
        )
        .add(
            "/locales",
            openapi(post(create_locale), routes!(create_locale)),
        )
        .add(
            "/locales/{locale}",
            openapi(patch(update_locale), routes!(update_locale)),
        )
        .add(
            "/locales/{locale}",
            openapi(delete(delete_locale), routes!(delete_locale)),
        )
        .add(
            "/keys",
            openapi(get(list_global_keys), routes!(list_global_keys)),
        )
        .add(
            "/namespaces",
            openapi(get(list_global_namespaces), routes!(list_global_namespaces)),
        )
        .add(
            "/translations",
            openapi(
                post(upsert_global_translation),
                routes!(upsert_global_translation),
            ),
        )
        .add(
            "/translations/{id}",
            openapi(
                patch(update_global_translation),
                routes!(update_global_translation),
            ),
        )
        .add(
            "/translations/{id}",
            openapi(
                delete(delete_global_translation),
                routes!(delete_global_translation),
            ),
        )
        .add(
            "/translations/import",
            openapi(
                post(import_global_translations),
                routes!(import_global_translations),
            ),
        )
        .add(
            "/translations/export",
            openapi(
                get(export_global_translations),
                routes!(export_global_translations),
            ),
        )
        .add(
            "/translations/batch-update",
            openapi(
                post(batch_update_global_translations),
                routes!(batch_update_global_translations),
            ),
        )
        // ── Entries management ─────────────────────────────────────────────
        .add(
            "/entries/{id}/locations",
            openapi(get(list_entry_locations), routes!(list_entry_locations)),
        )
        .add(
            "/entries/{id}",
            openapi(delete(delete_entry), routes!(delete_entry)),
        )
}

// ── Import / export ─────────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/admin/i18n/translations/import",
    tag = "国际化",
    description = "[超管] 批量导入全局翻译（≤500 entries / ≤2MB；strategy = replace|skip）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn import_global_translations(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ImportRequest>,
) -> Result<Response> {
    enforce_import_body_limit(&headers)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let resp = i18n_service::import_global(&ctx, tc.user_id, &req, &audit_ctx).await?;
    format::json(resp)
}

#[utoipa::path(
    post,
    path = "/api/admin/i18n/translations/batch-update",
    tag = "国际化",
    description = "[超管] 批量更新已存在的全局翻译（不会创建新记录）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn batch_update_global_translations(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    headers: axum::http::HeaderMap,
    Json(req): Json<BatchUpdateRequest>,
) -> Result<Response> {
    enforce_import_body_limit(&headers)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let resp =
        i18n_service::batch_update_global(&ctx, tc.user_id, &req.entries, &audit_ctx)
            .await?;
    format::json(resp)
}

#[utoipa::path(
    get,
    path = "/api/admin/i18n/translations/export",
    tag = "国际化",
    description = "[超管] 导出全局翻译（可按 namespace/locale 过滤）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn export_global_translations(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(q): Query<ExportQuery>,
) -> Result<Response> {
    let resp =
        i18n_service::export_global(&ctx.db, q.namespace.as_deref(), q.locale.as_deref())
            .await?;
    format::json(resp)
}

// ── Entries management (super-admin) ────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/api/admin/i18n/entries/{id}/locations",
    tag = "国际化",
    description = "[超管] 查询 stable_id 在源码中的出现位置",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_entry_locations(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let locations = i18n_service::list_entry_locations(&ctx.db, id).await?;
    format::json(locations)
}

#[utoipa::path(
    delete,
    path = "/api/admin/i18n/entries/{id}",
    tag = "国际化",
    description = "[超管] 强制删除 stable_id（级联删除翻译和位置）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn delete_entry(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    i18n_service::delete_entry_cascade(&ctx, id, &audit_ctx).await?;
    format::json(())
}
