//! CI-side i18n endpoints — gated by [`crate::middleware::ci_token_layer`],
//! NOT by Casbin.
//!
//! The only operation today is "apply manifest", invoked by GitHub Actions
//! after the extractor CLI has scanned the frontend source tree. The body is
//! the manifest JSON (see `system-design/国际化.md` §13).
//!
//! Audit context is built manually because no JWT / tenant exists for CI:
//! - `tenant_id` = nil UUID (sentinel for "system / cross-tenant operation")
//! - `user_id`   = `None` (no human acted)
//! - `trace_id` / `request_id` / `ip` / `user_agent` come from `RequestMeta`

use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use uuid::Uuid;

use crate::extractors::RequestMeta;
use crate::services::i18n_manifest_service;
use crate::views::audit_logs::AuditContext;
use crate::views::i18n::ManifestUploadRequest;

#[utoipa::path(
    post,
    path = "/api/ci/i18n/manifest",
    tag = "CI / 国际化",
    description = "应用 i18n 提取器输出的清单（创建/更新 entries 与 locations，未出现的标 stale）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(
    trace_id = %meta.trace_id,
    request_id = %meta.request_id.as_deref().unwrap_or(""),
    entries = payload.entries.len(),
))]
pub(crate) async fn apply_manifest(
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(payload): Json<ManifestUploadRequest>,
) -> Result<Response> {
    // System-level audit context: nil tenant, no user, real network metadata.
    let audit_ctx = AuditContext {
        trace_id: Some(meta.trace_id.clone()),
        request_id: meta.request_id.clone(),
        tenant_id: Uuid::nil(),
        user_id: None,
        ip_address: meta.ip_address.clone(),
        user_agent: meta.user_agent.clone(),
    };

    let result =
        i18n_manifest_service::apply_manifest(&ctx, &payload, &audit_ctx).await?;

    format::json(result)
}

pub fn routes() -> Routes {
    Routes::new().prefix("/api/ci/i18n").add(
        "/manifest",
        openapi(post(apply_manifest), routes!(apply_manifest)),
    )
}
