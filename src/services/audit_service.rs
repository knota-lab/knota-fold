use chrono::Utc;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde::Serialize;
use uuid::Uuid;

use crate::models::_entities::audit_logs;
use crate::utils::error::{IntoAppError, IntoLocoResult};
use crate::views::audit_logs::{
    AuditAction, AuditContext, AuditEntry, AuditLogQuery, AuditLogResponse,
};
use crate::views::pagination::PaginatedResponse;

/// Record a single audit log entry.
///
/// `db` accepts both `DatabaseConnection` and `DatabaseTransaction`,
/// enabling same-transaction audit writes.
#[tracing::instrument(skip_all)]
pub async fn log<C: ConnectionTrait>(
    db: &C,
    ctx: &AuditContext,
    action: AuditAction,
    resource_type: &str,
    resource_id: &str,
    before: Option<&(impl Serialize + Sync)>,
    after: Option<&(impl Serialize + Sync)>,
) -> loco_rs::Result<()> {
    let before_json = before.map(serde_json::to_value).transpose().loco_err()?;
    let after_json = after.map(serde_json::to_value).transpose().loco_err()?;

    let am = audit_logs::ActiveModel {
        id: ActiveValue::Set(crate::utils::id::generate_id()),
        request_id: ActiveValue::Set(ctx.request_id.clone()),
        trace_id: ActiveValue::Set(ctx.trace_id.clone()),
        tenant_id: ActiveValue::Set(ctx.tenant_id),
        user_id: ActiveValue::Set(ctx.user_id),
        action: ActiveValue::Set(action.as_str().to_string()),
        resource_type: ActiveValue::Set(resource_type.to_string()),
        resource_id: ActiveValue::Set(resource_id.to_string()),
        before_state: ActiveValue::Set(before_json),
        after_state: ActiveValue::Set(after_json),
        ip_address: ActiveValue::Set(ctx.ip_address.clone()),
        user_agent: ActiveValue::Set(ctx.user_agent.clone()),
        status: ActiveValue::Set("success".to_string()),
        error_message: ActiveValue::Set(None),
        created_at: ActiveValue::Set(Utc::now().into()),
    };

    am.insert(db).await.db_err()?;
    Ok(())
}

/// Record multiple audit log entries in one batch (for sync operations).
#[tracing::instrument(skip_all)]
pub async fn log_batch<C: ConnectionTrait>(
    db: &C,
    ctx: &AuditContext,
    entries: Vec<AuditEntry>,
) -> loco_rs::Result<()> {
    for entry in entries {
        let am = audit_logs::ActiveModel {
            id: ActiveValue::Set(crate::utils::id::generate_id()),
            request_id: ActiveValue::Set(ctx.request_id.clone()),
            trace_id: ActiveValue::Set(ctx.trace_id.clone()),
            tenant_id: ActiveValue::Set(ctx.tenant_id),
            user_id: ActiveValue::Set(ctx.user_id),
            action: ActiveValue::Set(entry.action.as_str().to_string()),
            resource_type: ActiveValue::Set(entry.resource_type),
            resource_id: ActiveValue::Set(entry.resource_id),
            before_state: ActiveValue::Set(entry.before),
            after_state: ActiveValue::Set(entry.after),
            ip_address: ActiveValue::Set(ctx.ip_address.clone()),
            user_agent: ActiveValue::Set(ctx.user_agent.clone()),
            status: ActiveValue::Set("success".to_string()),
            error_message: ActiveValue::Set(None),
            created_at: ActiveValue::Set(Utc::now().into()),
        };

        am.insert(db).await.db_err()?;
    }
    Ok(())
}

/// Query audit logs with pagination and filters.
/// Non-super-admin callers must have `tenant_id_filter` set for tenant isolation.
#[tracing::instrument(skip_all)]
pub async fn query_audit_logs(
    db: &impl ConnectionTrait,
    query: &AuditLogQuery,
    tenant_id_filter: Option<Uuid>,
) -> loco_rs::Result<PaginatedResponse<AuditLogResponse>> {
    let page = query.page.unwrap_or(1);
    let page_size = query.page_size.unwrap_or(20).min(100);

    let mut select = audit_logs::Entity::find();

    // Tenant isolation
    if let Some(tid) = tenant_id_filter {
        select = select.filter(audit_logs::Column::TenantId.eq(tid));
    } else if let Some(tid) = query.tenant_id {
        // Super admin can filter by specific tenant
        select = select.filter(audit_logs::Column::TenantId.eq(tid));
    }

    if let Some(ref rt) = query.resource_type {
        select = select.filter(audit_logs::Column::ResourceType.eq(rt.as_str()));
    }
    if let Some(ref ri) = query.resource_id {
        select = select.filter(audit_logs::Column::ResourceId.eq(ri.as_str()));
    }
    if let Some(ref action) = query.action {
        select = select.filter(audit_logs::Column::Action.eq(action.as_str()));
    }
    if let Some(uid) = query.user_id {
        select = select.filter(audit_logs::Column::UserId.eq(uid));
    }
    if let Some(ref from) = query.from {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(from) {
            select = select.filter(audit_logs::Column::CreatedAt.gte(dt));
        }
    }
    if let Some(ref to) = query.to {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(to) {
            select = select.filter(audit_logs::Column::CreatedAt.lte(dt));
        }
    }

    select = select.order_by_desc(audit_logs::Column::CreatedAt);

    let total_items = select.clone().count(db).await.db_err()?;

    let total_pages = if total_items == 0 {
        0
    } else {
        total_items.div_ceil(page_size)
    };

    let items: Vec<audit_logs::Model> = select
        .offset((page.saturating_sub(1)) * page_size)
        .limit(page_size)
        .all(db)
        .await
        .db_err()?;

    let response_items: Vec<AuditLogResponse> =
        items.iter().map(AuditLogResponse::from_model).collect();

    Ok(PaginatedResponse {
        items: response_items,
        total_pages,
        total_items,
        page,
        page_size,
    })
}
