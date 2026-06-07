use loco_openapi::prelude::*;
use loco_rs::prelude::*;

use crate::extractors::TenantContext;
use crate::modules::knowledge_base::service;
use crate::modules::knowledge_base::views::{
    CreateFolderRequest, FolderListQuery, FolderResponse, UpdateFolderRequest,
};
use crate::utils::error::IntoModelResult;
use crate::views::errors::parse_uuid;

#[utoipa::path(
    post,
    path = "/api/kb-folders",
    tag = "知识库",
    description = "创建知识库目录",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn create(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateFolderRequest>,
) -> Result<Response> {
    let folder = service::create_folder(
        &ctx.db,
        &service::CreateFolderParams {
            tenant_id: tc.tenant_id,
            library_id: params.library_id,
            parent_id: params.parent_id,
            name: params.name,
            sort_order: params.sort_order,
            created_by: tc.user_id,
        },
    )
    .await
    .model_err()?;
    format::json(FolderResponse::from_model(&folder))
}

#[utoipa::path(
    get,
    path = "/api/kb-folders",
    tag = "知识库",
    description = "查询知识库目录列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(params): Query<FolderListQuery>,
) -> Result<Response> {
    let items =
        service::list_folders(&ctx.db, tc.tenant_id, params.library_id, params.parent_id)
            .await
            .model_err()?;
    format::json(
        items
            .iter()
            .map(FolderResponse::from_model)
            .collect::<Vec<_>>(),
    )
}

#[utoipa::path(
    put,
    path = "/api/kb-folders/{id}",
    tag = "知识库",
    description = "更新知识库目录",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn update(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<UpdateFolderRequest>,
) -> Result<Response> {
    let folder_id = parse_uuid(id)?;
    let folder = service::update_folder(
        &ctx.db,
        tc.tenant_id,
        folder_id,
        &service::UpdateFolderParams {
            name: params.name,
            sort_order: params.sort_order,
        },
    )
    .await
    .model_err()?;
    format::json(FolderResponse::from_model(&folder))
}

#[utoipa::path(
    delete,
    path = "/api/kb-folders/{id}",
    tag = "知识库",
    description = "删除空知识库目录",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn delete(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let folder_id = parse_uuid(id)?;
    service::delete_folder(&ctx.db, tc.tenant_id, folder_id)
        .await
        .model_err()?;
    format::json(serde_json::json!({"success": true}))
}
