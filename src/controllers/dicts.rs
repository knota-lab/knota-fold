use axum::extract::Query;
use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use crate::extractors::{RequestMeta, TenantContext};
use crate::services::dict_service;
use crate::views::audit_logs::AuditContext;
use crate::views::dicts::{
    CreateDictItemRequest, CreateDictTypeRequest, DictItemResponse, DictItemTreeResponse,
    DictItemsQuery, DictTypeResponse, ToggleStatusRequest, UpdateDictItemRequest,
    UpdateDictTypeRequest,
};
use crate::views::errors::parse_uuid;
use crate::views::pagination::{PaginatedResponse, PaginationParams};

// ══════════════════════════════════════════════
//  Dict Type handlers
// ══════════════════════════════════════════════

#[utoipa::path(
    get,
    path = "/api/dict-types",
    tag = "字典管理",
    description = "查询字典类型列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_types(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Response> {
    let pagination = pagination.into();
    let response: PaginatedResponse<DictTypeResponse> =
        dict_service::list_dict_types(&ctx.db, tc.tenant_filter(), &pagination).await?;

    format::json(response)
}

#[utoipa::path(
    post,
    path = "/api/dict-types",
    tag = "字典管理",
    description = "创建字典类型",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn create_type(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateDictTypeRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let dict_type = dict_service::create_dict_type(
        &ctx.db,
        tc.tenant_filter(),
        tc.user_id,
        &params,
        &audit_ctx,
    )
    .await?;

    format::json(DictTypeResponse::from_model(&dict_type))
}

#[utoipa::path(
    put,
    path = "/api/dict-types/{id}",
    tag = "字典管理",
    description = "更新字典类型",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn update_type(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<UpdateDictTypeRequest>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let dict_type = dict_service::update_dict_type(
        &ctx.db,
        id_uuid,
        tc.tenant_filter(),
        tc.user_id,
        &params,
        &audit_ctx,
    )
    .await?;

    format::json(DictTypeResponse::from_model(&dict_type))
}

#[utoipa::path(
    put,
    path = "/api/dict-types/{id}/status",
    tag = "字典管理",
    description = "切换字典类型状态",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn toggle_type_status(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<ToggleStatusRequest>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let dict_type = dict_service::toggle_dict_type_status(
        &ctx.db,
        id_uuid,
        tc.tenant_filter(),
        tc.user_id,
        params.version,
        &audit_ctx,
    )
    .await?;

    format::json(DictTypeResponse::from_model(&dict_type))
}

#[utoipa::path(
    post,
    path = "/api/dict-types/{id}/reset",
    tag = "字典管理",
    description = "重置租户字典类型覆盖（恢复为系统默认）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn reset_type_override(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    dict_service::reset_dict_type_override(&ctx.db, id_uuid, tc.tenant_id, &audit_ctx)
        .await?;

    format::json(())
}

// ══════════════════════════════════════════════
//  Dict Item handlers
// ══════════════════════════════════════════════

#[utoipa::path(
    get,
    path = "/api/dicts",
    tag = "字典管理",
    description = "查询字典项列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list_items(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(query): Query<DictItemsQuery>,
) -> Result<Response> {
    let response: Vec<DictItemResponse> =
        dict_service::list_dict_items(&ctx.db, tc.tenant_filter(), &query.type_code)
            .await?;

    format::json(response)
}

#[utoipa::path(
    get,
    path = "/api/dicts/tree",
    tag = "字典管理",
    description = "查询字典项树",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn tree_items(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(query): Query<DictItemsQuery>,
) -> Result<Response> {
    let response: Vec<DictItemTreeResponse> =
        dict_service::get_dict_item_tree(&ctx.db, tc.tenant_filter(), &query.type_code)
            .await?;

    format::json(response)
}

#[utoipa::path(
    post,
    path = "/api/dicts",
    tag = "字典管理",
    description = "创建字典项",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn create_item(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateDictItemRequest>,
) -> Result<Response> {
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let item = dict_service::create_dict_item(
        &ctx.db,
        tc.tenant_filter(),
        tc.user_id,
        &params,
        &audit_ctx,
    )
    .await?;

    format::json(DictItemResponse::from_model(&item))
}

#[utoipa::path(
    put,
    path = "/api/dicts/{id}",
    tag = "字典管理",
    description = "更新字典项",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn update_item(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<UpdateDictItemRequest>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let item = dict_service::update_dict_item(
        &ctx.db,
        id_uuid,
        tc.tenant_filter(),
        tc.user_id,
        &params,
        &audit_ctx,
    )
    .await?;

    format::json(DictItemResponse::from_model(&item))
}

#[utoipa::path(
    put,
    path = "/api/dicts/{id}/status",
    tag = "字典管理",
    description = "切换字典项状态",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn toggle_item_status(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<ToggleStatusRequest>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    let item = dict_service::toggle_dict_item_status(
        &ctx.db,
        id_uuid,
        tc.tenant_filter(),
        tc.user_id,
        params.version,
        &audit_ctx,
    )
    .await?;

    format::json(DictItemResponse::from_model(&item))
}

#[utoipa::path(
    post,
    path = "/api/dicts/{id}/reset",
    tag = "字典管理",
    description = "重置租户字典项覆盖（恢复为系统默认）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all, fields(trace_id = %meta.trace_id, request_id = %meta.request_id.as_deref().unwrap_or("")))]
pub(crate) async fn reset_item_override(
    tc: TenantContext,
    meta: RequestMeta,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let id_uuid = parse_uuid(id)?;
    let audit_ctx = AuditContext::from_request(&tc, &meta);
    dict_service::reset_dict_item_override(&ctx.db, id_uuid, tc.tenant_id, &audit_ctx)
        .await?;

    format::json(())
}

// ══════════════════════════════════════════════
//  Route registration
// ══════════════════════════════════════════════

pub fn dict_type_routes() -> Routes {
    Routes::new()
        .prefix("/api/dict-types")
        .add("/", openapi(get(list_types), routes!(list_types)))
        .add("/", openapi(post(create_type), routes!(create_type)))
        .add("/{id}", openapi(put(update_type), routes!(update_type)))
        .add(
            "/{id}/status",
            openapi(put(toggle_type_status), routes!(toggle_type_status)),
        )
        .add(
            "/{id}/reset",
            openapi(post(reset_type_override), routes!(reset_type_override)),
        )
}

pub fn dict_item_routes() -> Routes {
    Routes::new()
        .prefix("/api/dicts")
        .add("/", openapi(get(list_items), routes!(list_items)))
        .add("/tree", openapi(get(tree_items), routes!(tree_items)))
        .add("/", openapi(post(create_item), routes!(create_item)))
        .add("/{id}", openapi(put(update_item), routes!(update_item)))
        .add(
            "/{id}/status",
            openapi(put(toggle_item_status), routes!(toggle_item_status)),
        )
        .add(
            "/{id}/reset",
            openapi(post(reset_item_override), routes!(reset_item_override)),
        )
}
