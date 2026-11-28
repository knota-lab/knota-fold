use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use crate::extractors::TenantContext;
use crate::services::audit_service;
use crate::views::audit_logs::AuditLogQuery;

#[utoipa::path(
    get,
    path = "/api/audit-logs",
    tag = "审计日志",
    description = "查询审计日志列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(query): Query<AuditLogQuery>,
) -> Result<Response> {
    let tenant_filter = if tc.is_super_admin {
        None
    } else {
        Some(tc.tenant_id)
    };

    let result = audit_service::query_audit_logs(&ctx.db, &query, tenant_filter).await?;

    format::json(result)
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/audit-logs")
        .add("/", openapi(get(list), routes!(list)))
}
