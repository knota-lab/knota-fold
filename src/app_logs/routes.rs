use axum::extract::{Path, Query};
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::app_logs::writer;
use crate::utils::error::IntoLocoResult;

// ── Response types (must match DB columns for FromRow) ──────────

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogItem {
    pub id: i64,
    pub trace_id: String,
    pub request_id: String,
    pub timestamp: i64,
    pub method: String,
    pub path: String,
    pub route: Option<String>,
    pub status_code: Option<i64>,
    pub duration_ms: Option<i64>,
    pub user_id: Option<String>,
    pub tenant_code: Option<String>,
    pub ip_address: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TraceSpanItem {
    pub id: i64,
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub span_name: String,
    pub span_type: Option<String>,
    pub start_time: i64,
    pub duration_ms: Option<i64>,
    pub fields_json: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LogEntryItem {
    pub id: i64,
    pub trace_id: String,
    pub span_id: Option<String>,
    pub timestamp: i64,
    pub level: String,
    pub target: Option<String>,
    pub message: Option<String>,
    pub fields_json: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceDetail {
    pub request: RequestLogItem,
    pub spans: Vec<TraceSpanItem>,
    pub entries: Vec<LogEntryItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResponse {
    pub items: Vec<RequestLogItem>,
    pub total_items: i64,
    pub total_pages: i64,
    pub page: i64,
    pub page_size: i64,
}

// ── Query params ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListParams {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub status_code: Option<i64>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub q: Option<String>,
    pub trace_id: Option<String>,
    pub ip_address: Option<String>,
    pub has_error: Option<bool>,
    pub min_duration: Option<i64>,
    pub max_duration: Option<i64>,
    pub user_id: Option<String>,
    pub request_id: Option<String>,
}

// ── Handlers ────────────────────────────────────────────────────

pub async fn stats() -> Result<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "droppedCount": crate::app_logs::layer::dropped_count(),
    })))
}

pub async fn list(Query(params): Query<ListParams>) -> Result<Json<ListResponse>> {
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * page_size;

    let db = match writer::log_db() {
        Some(pool) => pool,
        None => {
            return Ok(Json(ListResponse {
                items: vec![],
                total_items: 0,
                total_pages: 0,
                page,
                page_size,
            }));
        }
    };

    // Build WHERE clauses.
    let mut where_parts: Vec<&'static str> = Vec::new();
    if params.method.is_some() {
        where_parts.push("method = ?");
    }
    if params.path.is_some() {
        where_parts.push("path LIKE '%' || ? || '%'");
    }
    if params.status_code.is_some() {
        where_parts.push("status_code = ?");
    }
    if params.from.is_some() {
        where_parts.push("timestamp >= ?");
    }
    if params.to.is_some() {
        where_parts.push("timestamp <= ?");
    }
    if params.q.is_some() {
        where_parts.push("error LIKE '%' || ? || '%'");
    }
    if params.trace_id.is_some() {
        where_parts.push("trace_id LIKE '%' || ? || '%'");
    }
    if params.ip_address.is_some() {
        where_parts.push("ip_address = ?");
    }
    if params.has_error == Some(true) {
        where_parts.push("error IS NOT NULL AND error != ''");
    }
    if params.min_duration.is_some() {
        where_parts.push("duration_ms >= ?");
    }
    if params.max_duration.is_some() {
        where_parts.push("duration_ms <= ?");
    }
    if params.user_id.is_some() {
        where_parts.push("user_id = ?");
    }
    if params.request_id.is_some() {
        where_parts.push("request_id = ?");
    }

    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_parts.join(" AND "))
    };

    // Count.
    let count_sql = format!("SELECT COUNT(*) FROM request_logs {where_sql}");
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(ref v) = params.method {
        count_q = count_q.bind(v.as_str());
    }
    if let Some(ref v) = params.path {
        count_q = count_q.bind(v.as_str());
    }
    if let Some(v) = params.status_code {
        count_q = count_q.bind(v);
    }
    if let Some(v) = params.from {
        count_q = count_q.bind(v);
    }
    if let Some(v) = params.to {
        count_q = count_q.bind(v);
    }
    if let Some(ref v) = params.q {
        count_q = count_q.bind(v.as_str());
    }
    if let Some(ref v) = params.trace_id {
        count_q = count_q.bind(v.as_str());
    }
    if let Some(ref v) = params.ip_address {
        count_q = count_q.bind(v.as_str());
    }
    if let Some(v) = params.min_duration {
        count_q = count_q.bind(v);
    }
    if let Some(v) = params.max_duration {
        count_q = count_q.bind(v);
    }
    if let Some(ref v) = params.user_id {
        count_q = count_q.bind(v.as_str());
    }
    if let Some(ref v) = params.request_id {
        count_q = count_q.bind(v.as_str());
    }

    let total_items: i64 = count_q.fetch_one(db).await.loco_err()?;

    // Data.
    let data_sql = format!(
        "SELECT id, trace_id, request_id, timestamp, method, path, route, \
          status_code, duration_ms, user_id, tenant_code, ip_address, error \
          FROM request_logs {where_sql} \
         ORDER BY timestamp DESC LIMIT ? OFFSET ?"
    );
    let mut data_q = sqlx::query_as::<_, RequestLogItem>(&data_sql);
    if let Some(ref v) = params.method {
        data_q = data_q.bind(v.as_str());
    }
    if let Some(ref v) = params.path {
        data_q = data_q.bind(v.as_str());
    }
    if let Some(v) = params.status_code {
        data_q = data_q.bind(v);
    }
    if let Some(v) = params.from {
        data_q = data_q.bind(v);
    }
    if let Some(v) = params.to {
        data_q = data_q.bind(v);
    }
    if let Some(ref v) = params.q {
        data_q = data_q.bind(v.as_str());
    }
    if let Some(ref v) = params.trace_id {
        data_q = data_q.bind(v.as_str());
    }
    if let Some(ref v) = params.ip_address {
        data_q = data_q.bind(v.as_str());
    }
    if let Some(v) = params.min_duration {
        data_q = data_q.bind(v);
    }
    if let Some(v) = params.max_duration {
        data_q = data_q.bind(v);
    }
    if let Some(ref v) = params.user_id {
        data_q = data_q.bind(v.as_str());
    }
    if let Some(ref v) = params.request_id {
        data_q = data_q.bind(v.as_str());
    }
    data_q = data_q.bind(page_size).bind(offset);

    let rows = data_q.fetch_all(db).await.loco_err()?;

    let total_pages = if total_items > 0 {
        (total_items + page_size - 1) / page_size
    } else {
        0
    };

    Ok(Json(ListResponse {
        items: rows,
        total_items,
        total_pages,
        page,
        page_size,
    }))
}

pub async fn get_trace(Path(trace_id): Path<String>) -> Result<Json<TraceDetail>> {
    let db = writer::log_db().ok_or_else(|| {
        crate::views::errors::err_internal(
            "app_logs.module_not_enabled",
            "app-logs 模块未启用",
        )
    })?;

    let request = sqlx::query_as::<_, RequestLogItem>(
        "SELECT id, trace_id, request_id, timestamp, method, path, route, \
         status_code, duration_ms, user_id, tenant_code, ip_address, error \
         FROM request_logs WHERE trace_id = ? LIMIT 1",
    )
    .bind(&trace_id)
    .fetch_optional(db)
    .await
    .loco_err()?;

    let request = request.ok_or_else(|| {
        crate::views::errors::err_not_found("app_logs.trace_not_found", "trace 不存在")
    })?;

    let spans = sqlx::query_as::<_, TraceSpanItem>(
        "SELECT id, trace_id, span_id, parent_span_id, span_name, span_type, \
         start_time, duration_ms, fields_json \
         FROM trace_spans WHERE trace_id = ? ORDER BY start_time",
    )
    .bind(&trace_id)
    .fetch_all(db)
    .await
    .loco_err()?;

    let entries = sqlx::query_as::<_, LogEntryItem>(
        "SELECT id, trace_id, span_id, timestamp, level, target, message, fields_json \
         FROM log_entries WHERE trace_id = ? ORDER BY timestamp",
    )
    .bind(&trace_id)
    .fetch_all(db)
    .await
    .loco_err()?;

    Ok(Json(TraceDetail {
        request,
        spans,
        entries,
    }))
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/admin/app-logs")
        .add("/stats", get(stats))
        .add("/", get(list))
        .add("/{trace_id}", get(get_trace))
}
