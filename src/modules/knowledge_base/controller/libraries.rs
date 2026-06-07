use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use crate::extractors::TenantContext;
use crate::modules::knowledge_base::service;
use crate::modules::knowledge_base::views::{
    CreateLibraryRequest, LibraryResponse, UpdateLibraryRequest,
};
use crate::utils::error::IntoModelResult;
use crate::views::errors::parse_uuid;

#[utoipa::path(
    post,
    path = "/api/kb-libraries",
    tag = "知识库",
    description = "创建知识库",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn create(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateLibraryRequest>,
) -> Result<Response> {
    let library = service::create_library(
        &ctx.db,
        &service::CreateLibraryParams {
            tenant_id: tc.tenant_id,
            name: params.name,
            description: params.description,
            sort_order: params.sort_order,
            created_by: tc.user_id,
        },
    )
    .await
    .model_err()?;
    format::json(LibraryResponse::from_model(&library))
}

#[utoipa::path(
    get,
    path = "/api/kb-libraries",
    tag = "知识库",
    description = "查询知识库列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let items = service::list_libraries(&ctx.db, tc.tenant_id)
        .await
        .model_err()?;
    format::json(
        items
            .iter()
            .map(LibraryResponse::from_model)
            .collect::<Vec<_>>(),
    )
}

#[utoipa::path(
    put,
    path = "/api/kb-libraries/{id}",
    tag = "知识库",
    description = "更新知识库",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn update(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<UpdateLibraryRequest>,
) -> Result<Response> {
    let library_id = parse_uuid(id)?;
    let library = service::update_library(
        &ctx.db,
        tc.tenant_id,
        library_id,
        &service::UpdateLibraryParams {
            name: params.name,
            description: params.description,
            sort_order: params.sort_order,
        },
    )
    .await
    .model_err()?;
    format::json(LibraryResponse::from_model(&library))
}

#[utoipa::path(
    delete,
    path = "/api/kb-libraries/{id}",
    tag = "知识库",
    description = "删除空知识库",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn delete(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let library_id = parse_uuid(id)?;
    service::delete_library(&ctx.db, tc.tenant_id, library_id)
        .await
        .model_err()?;
    format::json(serde_json::json!({"success": true}))
}
