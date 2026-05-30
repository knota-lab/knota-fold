use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use crate::extractors::RequestMeta;
use crate::extractors::TenantContext;
use crate::modules::notification::errors::NotificationError;
use crate::modules::notification::service;
use crate::modules::notification::views::{
    CreateNotificationRequest, NotificationResponse,
};
use crate::utils::error::IntoModelResult;
use crate::views::pagination::PaginatedResponse;

#[utoipa::path(
    post,
    path = "/api/notifications",
    tag = "通知管理",
    description = "创建通知公告",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn create(
    tc: TenantContext,
    _meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateNotificationRequest>,
) -> Result<Response> {
    // Validate: platform type only super_admin
    if params.notification_type == "platform" && !tc.is_super_admin {
        return NotificationError::PlatformRequiresSuperAdmin.to_response();
    }
    // Validate: tenant_role requires roles
    if params.notification_type == "tenant_role"
        && params
            .target_role_codes
            .as_ref()
            .is_none_or(std::vec::Vec::is_empty)
    {
        return NotificationError::NoRolesSelected.to_response();
    }

    // Determine tenant_id: platform = None, others = Some(tc.tenant_id)
    // Do NOT use tc.tenant_filter() — super_admin's tenant_filter() returns None,
    // which would make tenant_all/tenant_role notifications have NULL tenant_id.
    let tenant_id = match params.notification_type.as_str() {
        "platform" => None,
        _ => Some(tc.tenant_id),
    };

    let notification = service::create::create_notification(
        &ctx.db,
        &service::create::CreateNotificationParams {
            tenant_id,
            created_by: tc.user_id,
            title: &params.title,
            content: &params.content,
            notification_type: &params.notification_type,
            priority: params.priority.as_deref().unwrap_or("normal"),
            target_role_codes: params.target_role_codes.as_deref(),
        },
    )
    .await
    .model_err()?;

    format::json(NotificationResponse::from_model(&notification))
}

#[utoipa::path(
    get,
    path = "/api/notifications",
    tag = "通知管理",
    description = "查询通知公告列表（管理端）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(query): Query<NotificationListQuery>,
) -> Result<Response> {
    let tenant_filter = if tc.is_super_admin {
        None // super_admin sees all
    } else {
        Some(tc.tenant_id)
    };

    let page = query.page.unwrap_or(1);
    let page_size = query.page_size.unwrap_or(20).min(100);

    let (items, total_items) =
        crate::modules::notification::models::notifications::Model::list_managed(
            &ctx.db,
            tenant_filter,
            query.notification_type,
            page,
            page_size,
        )
        .await
        .model_err()?;

    let total_pages = if total_items == 0 {
        0
    } else {
        total_items.div_ceil(page_size)
    };

    let responses: Vec<NotificationResponse> =
        items.iter().map(NotificationResponse::from_model).collect();

    format::json(PaginatedResponse {
        items: responses,
        total_pages,
        total_items,
        page,
        page_size,
    })
}

#[utoipa::path(
    put,
    path = "/api/notifications/{id}/revoke",
    tag = "通知管理",
    description = "撤回通知公告",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn revoke(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let notification_id = parse_uuid(id)?;

    let tenant_filter = if tc.is_super_admin {
        None
    } else {
        Some(tc.tenant_id)
    };

    service::revoke::revoke_notification(
        &ctx.db,
        notification_id,
        tc.user_id,
        tc.is_super_admin,
        tenant_filter,
    )
    .await
    .model_err()?;

    format::json(serde_json::json!({"success": true}))
}

use crate::views::errors::parse_uuid;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationListQuery {
    pub page: Option<u64>,
    pub page_size: Option<u64>,
    pub notification_type: Option<String>,
}
