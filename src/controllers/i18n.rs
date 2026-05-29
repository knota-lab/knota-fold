//! User-facing i18n endpoints — authenticated reads only.
//!
//! Both endpoints are JWT-gated but **whitelisted** in
//! [`crate::middleware::casbin_authz`] because every authenticated user
//! needs them on every page render. Per-tenant scoping is enforced
//! inside the services (which read tenant_id from `TenantContext`).
//!
//! Endpoints:
//!
//! - `GET /api/public/i18n/locales` — list enabled locales (5-min server cache)
//! - `GET /api/i18n/bundles/{namespace}/{locale}` — resolve a bundle, with
//!   `If-None-Match` → `304 Not Modified` short-circuit. Response carries an
//!   `ETag` header of the form `"{global_rev}-{tenant_rev}"`.

use axum::http::header::{ETAG, IF_NONE_MATCH};
use axum::http::HeaderValue;
use axum::http::StatusCode;
use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use crate::extractors::TenantContext;
use crate::models::users;
use crate::services::{i18n_locale_service, i18n_service};
use crate::utils::error::{IntoModelResult, OptionErrInto};
use crate::views::errors::err_bad_request;
use crate::views::i18n::{self as i18n_views, BASE_LOCALE};

#[utoipa::path(
    get,
    path = "/api/public/i18n/locales",
    tag = "国际化",
    description = "查询当前启用的语种列表（5 分钟服务端缓存）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_enabled_locales(
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let locales = i18n_locale_service::list_enabled_cached(&ctx).await?;
    format::json(locales)
}

#[utoipa::path(
    get,
    path = "/api/i18n/bundles/{namespace}/{locale}",
    tag = "国际化",
    description = "拉取一个 (namespace, locale) bundle；支持 If-None-Match → 304",
    responses(
        (status = 200, description = "Bundle 内容"),
        (status = 304, description = "ETag 命中，未变更"),
    )
)]
#[debug_handler]
pub(crate) async fn get_bundle(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path((namespace, locale)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<Response> {
    let tenant_filter = tc.tenant_filter();

    // Cheap pre-flight: compute ETag without materializing the bundle.
    let etag =
        i18n_service::compute_etag(&ctx, &locale, &namespace, tenant_filter).await?;

    if let Some(if_none_match) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok())
    {
        if etag_matches(if_none_match, &etag) {
            let mut resp = axum::response::Response::new(axum::body::Body::empty());
            *resp.status_mut() = StatusCode::NOT_MODIFIED;
            if let Ok(value) = HeaderValue::from_str(&etag) {
                resp.headers_mut().insert(ETAG, value);
            }
            return Ok(resp);
        }
    }

    // Cache miss / changed → materialize and return full bundle.
    let (bundle, fresh_etag) =
        i18n_service::resolve_bundle(&ctx, &locale, &namespace, tenant_filter).await?;

    let mut resp = format::json(bundle)?;
    if let Ok(value) = HeaderValue::from_str(&fresh_etag) {
        resp.headers_mut().insert(ETAG, value);
    }
    Ok(resp)
}

/// Match an `If-None-Match` header value against our generated ETag.
///
/// Spec-compliant clients send `If-None-Match: "abc"` (quoted), possibly
/// comma-separated for multiple tags, and may include the `W/` weak prefix.
/// Our ETags are strong and quoted, so we tolerate both forms.
fn etag_matches(if_none_match: &str, our_etag: &str) -> bool {
    if if_none_match.trim() == "*" {
        return true;
    }
    if_none_match
        .split(',')
        .map(str::trim)
        .map(|tag| tag.strip_prefix("W/").unwrap_or(tag))
        .any(|tag| tag == our_etag)
}

// ── Public (unauthenticated) bundle endpoint ──────────────────────────────

/// Namespaces safe to expose without authentication (login page, etc.).
const PUBLIC_NAMESPACES: &[&str] = &["Login", "Common"];

#[utoipa::path(
    get,
    path = "/api/public/i18n/bundles/{namespace}/{locale}",
    tag = "国际化",
    description = "公开 bundle 端点，无需登录。仅限白名单 namespace（Login 等）。",
    responses(
        (status = 200, description = "Bundle 内容"),
        (status = 304, description = "ETag 命中，未变更"),
        (status = 403, description = "Namespace 不在公开白名单中"),
    )
)]
#[debug_handler]
pub(crate) async fn get_public_bundle(
    State(ctx): State<AppContext>,
    Path((namespace, locale)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<Response> {
    if !PUBLIC_NAMESPACES.contains(&namespace.as_str()) {
        return crate::views::errors::i18n::namespace_not_public(&namespace);
    }

    // No tenant filter — return global-only content.
    let tenant_filter = None;

    let etag =
        i18n_service::compute_etag(&ctx, &locale, &namespace, tenant_filter).await?;

    if let Some(if_none_match) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok())
    {
        if etag_matches(if_none_match, &etag) {
            let mut resp = axum::response::Response::new(axum::body::Body::empty());
            *resp.status_mut() = StatusCode::NOT_MODIFIED;
            if let Ok(value) = HeaderValue::from_str(&etag) {
                resp.headers_mut().insert(ETAG, value);
            }
            return Ok(resp);
        }
    }

    let (bundle, fresh_etag) =
        i18n_service::resolve_bundle(&ctx, &locale, &namespace, tenant_filter).await?;

    let mut resp = format::json(bundle)?;
    if let Ok(value) = HeaderValue::from_str(&fresh_etag) {
        resp.headers_mut().insert(ETAG, value);
    }
    Ok(resp)
}

/// Routes that require **no authentication** — registered without
/// `CasbinAuthzLayer` in `app.rs`.
pub fn public_routes() -> Routes {
    Routes::new()
        .prefix("/api/public/i18n")
        .add(
            "/bundles/{namespace}/{locale}",
            openapi(get(get_public_bundle), routes!(get_public_bundle)),
        )
        .add(
            "/locales",
            openapi(get(list_enabled_locales), routes!(list_enabled_locales)),
        )
}

/// Routes that require **JWT authentication** (TenantContext) but no Casbin.
/// Every logged-in user needs bundles + locale preferences to render any page.
pub fn user_routes() -> Routes {
    Routes::new()
        .prefix("/api/i18n")
        .add(
            "/bundles/{namespace}/{locale}",
            openapi(get(get_bundle), routes!(get_bundle)),
        )
        .add("/me", openapi(get(get_i18n_me), routes!(get_i18n_me)))
        .add("/me", openapi(put(update_i18n_me), routes!(update_i18n_me)))
}

// ── User preference endpoints ──────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/api/i18n/me",
    tag = "国际化",
    description = "返回当前用户的语言偏好及默认 fallback",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_i18n_me(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let user = users::Entity::find_by_id(tc.user_id)
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    format::json(i18n_views::I18nMeResponse {
        preferred_locale: user.preferred_locale,
        default_locale: BASE_LOCALE.to_string(),
    })
}

#[utoipa::path(
    put,
    path = "/api/i18n/me",
    tag = "国际化",
    description = "更新当前用户的语言偏好",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn update_i18n_me(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(req): Json<i18n_views::UpdateI18nMeRequest>,
) -> Result<Response> {
    let user = users::Entity::find_by_id(tc.user_id)
        .one(&ctx.db)
        .await
        .model_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    // Validate: if set, locale must be a known enabled locale.
    if let Some(ref locale) = req.preferred_locale {
        let enabled = i18n_locale_service::list_enabled_cached(&ctx).await?;
        if !enabled.iter().any(|l| l.locale == *locale) {
            return Err(err_bad_request(
                "i18n.unsupported_locale",
                format!("不支持的语言: {locale}"),
            ));
        }
    }

    let updated = users::ActiveModel {
        preferred_locale: sea_orm::ActiveValue::set(req.preferred_locale),
        ..user.into()
    }
    .update(&ctx.db)
    .await
    .model_err()?;

    format::json(i18n_views::I18nMeResponse {
        preferred_locale: updated.preferred_locale,
        default_locale: BASE_LOCALE.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::etag_matches;

    #[test]
    fn matches_exact_quoted() {
        assert!(etag_matches("\"5-3\"", "\"5-3\""));
    }

    #[test]
    fn matches_wildcard() {
        assert!(etag_matches("*", "\"5-3\""));
    }

    #[test]
    fn matches_weak_prefix() {
        assert!(etag_matches("W/\"5-3\"", "\"5-3\""));
    }

    #[test]
    fn matches_in_list() {
        assert!(etag_matches("\"1-1\", \"5-3\", \"7-2\"", "\"5-3\""));
    }

    #[test]
    fn rejects_mismatch() {
        assert!(!etag_matches("\"5-4\"", "\"5-3\""));
    }
}
