use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use crate::extractors::TenantContext;
use crate::modules::notification::service;
use crate::modules::notification::views::InboxQueryParams;
use crate::utils::error::IntoModelResult;
use crate::views::errors::parse_uuid;

#[utoipa::path(
    get,
    path = "/api/notifications/inbox",
    tag = "通知管理",
    description = "查询收件箱列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn inbox(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(params): Query<InboxQueryParams>,
) -> Result<Response> {
    let page = params.page.unwrap_or(1);
    let page_size = params.page_size.unwrap_or(20).min(100);

    let result =
        service::query::get_inbox(&ctx.db, tc.user_id, params.read, page, page_size)
            .await
            .model_err()?;

    format::json(result)
}

#[utoipa::path(
    get,
    path = "/api/notifications/unread-count",
    tag = "通知管理",
    description = "查询未读通知数量",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn unread_count(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let result = service::query::get_unread_count(&ctx.db, tc.user_id)
        .await
        .model_err()?;
    format::json(result)
}

#[utoipa::path(
    put,
    path = "/api/notifications/{id}/read",
    tag = "通知管理",
    description = "标记单条通知已读",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn mark_read(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let recipient_id = parse_uuid(id)?;

    service::query::mark_read(&ctx.db, recipient_id, tc.user_id)
        .await
        .model_err()?;

    format::json(serde_json::json!({"success": true}))
}

#[utoipa::path(
    put,
    path = "/api/notifications/read-all",
    tag = "通知管理",
    description = "标记全部通知已读",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn mark_all_read(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let count = service::query::mark_all_read(&ctx.db, tc.user_id)
        .await
        .model_err()?;

    format::json(serde_json::json!({"success": true, "count": count}))
}

#[utoipa::path(
    get,
    path = "/api/notifications/forced",
    tag = "通知管理",
    description = "查询强制通知（未确认的强通知）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn forced(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let items = service::query::get_forced_notifications(&ctx.db, tc.user_id)
        .await
        .model_err()?;
    format::json(items)
}
