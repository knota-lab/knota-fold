use std::collections::HashSet;
use std::time::Duration;

use aws_sdk_s3::presigning::PresigningConfig;
use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};

use crate::config::{AppSettings, ConfigExt};
use crate::extractors::TenantContext;
use crate::initializers::knowledge_base::SharedSearchProvider;
use crate::initializers::s3::{SharedS3Client, SharedS3Config};
use crate::models::_entities::{document_lines, kb_chunks, kb_documents};
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::service;
use crate::modules::knowledge_base::views::{
    CreateDocumentRequest, DocumentAssetResponse, DocumentListQuery,
    DocumentPreviewResponse, DocumentResponse, PresignDocumentAssetsRequest,
    PresignDocumentAssetsResponse, PresignedDocumentAssetResponse,
};
use crate::utils::error::IntoModelResult;
use crate::views::errors::{err_bad_request, err_forbidden, err_internal, parse_uuid};
use crate::views::pagination::PaginatedResponse;
use crate::workers::indexing_worker::{IndexingWorker, IndexingWorkerArgs};

const ASSET_PRESIGN_TTL_SECONDS: u64 = 3600;

#[utoipa::path(
    post,
    path = "/api/kb-documents",
    tag = "知识库",
    description = "上传文档",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn create(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateDocumentRequest>,
) -> Result<Response> {
    if params.content.is_none() && params.file_id.is_none() {
        return Err(err_bad_request(
            "knowledge_base.document_source_required",
            "content or fileId is required",
        ));
    }

    let source_type = params.source_type.unwrap_or_else(|| {
        if params.content.is_some() {
            "text/plain".to_string()
        } else {
            "kb_upload".to_string()
        }
    });

    let scope = params.scope.unwrap_or_else(|| "tenant".to_string());
    let location = service::resolve_document_location(
        &ctx.db,
        tc.tenant_id,
        params.library_id,
        params.folder_id,
    )
    .await
    .model_err()?;

    let doc = service::document_service::create_document(
        &ctx.db,
        &service::document_service::CreateDocumentParams {
            tenant_id: tc.tenant_id,
            title: params.title,
            description: params.description,
            library_id: location.library_id,
            folder_id: location.folder_id,
            source_type,
            scope,
            file_id: params.file_id,
            created_by: tc.user_id,
        },
    )
    .await
    .model_err()?;

    if let Some(ref content) = params.content {
        service::set_full_text(&ctx.db, doc.id, content)
            .await
            .model_err()?;
    }

    IndexingWorker::perform_later(
        &ctx,
        IndexingWorkerArgs {
            document_id: doc.id,
            tenant_id: tc.tenant_id,
            trace_id: None,
            parent_span_id: None,
        },
    )
    .await
    .model_err()?;

    format::json(DocumentResponse::from_model(&doc))
}

#[utoipa::path(
    get,
    path = "/api/kb-documents",
    tag = "知识库",
    description = "查询文档列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(params): Query<DocumentListQuery>,
) -> Result<Response> {
    let page = params.page.unwrap_or(1);
    let page_size = params.page_size.unwrap_or(20).min(100);

    let mut query = kb_documents::Entity::find()
        .filter(kb_documents::Column::TenantId.eq(tc.tenant_id));

    if let Some(ref status) = params.status {
        query = query.filter(kb_documents::Column::Status.eq(status));
    }

    if let Some(ref scope) = params.scope {
        query = query.filter(kb_documents::Column::Scope.eq(scope));
    }
    if let Some(library_id) = params.library_id {
        query = query.filter(kb_documents::Column::LibraryId.eq(library_id));
    }
    if let Some(folder_id) = params.folder_id {
        query = query.filter(kb_documents::Column::FolderId.eq(folder_id));
    }

    let paginator = query
        .order_by_desc(kb_documents::Column::CreatedAt)
        .paginate(&ctx.db, page_size);

    let total_items = paginator.num_items().await.model_err()?;
    let total_pages = paginator.num_pages().await.model_err()?;
    let items = paginator.fetch_page(page - 1).await.model_err()?;

    format::json(PaginatedResponse {
        items: items.iter().map(DocumentResponse::from_model).collect(),
        total_pages,
        total_items,
        page,
        page_size,
    })
}

#[utoipa::path(
    get,
    path = "/api/kb-documents/{id}",
    tag = "知识库",
    description = "查询文档详情",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let doc_id = parse_uuid(id)?;
    let doc = service::get_document(&ctx.db, doc_id, tc.tenant_id)
        .await
        .model_err()?;

    format::json(DocumentResponse::from_model(&doc))
}

#[utoipa::path(
    get,
    path = "/api/kb/documents/{id}/preview",
    tag = "知识库",
    description = "获取文档预览 Markdown 与解析资源列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn preview(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let doc_id = parse_uuid(id)?;
    let doc = service::get_document(&ctx.db, doc_id, tc.tenant_id)
        .await
        .model_err()?;
    ensure_document_readable(&doc, tc.user_id)?;

    let markdown = doc.full_text.clone().ok_or_else(|| {
        err_bad_request(
            "knowledge_base.document_not_parsed",
            "document preview is not available before parsing completes",
        )
    })?;

    format::json(DocumentPreviewResponse {
        document_id: doc.id.to_string(),
        title: doc.title,
        markdown,
        assets: extract_document_assets(doc.metadata.as_ref())?,
    })
}

#[utoipa::path(
    post,
    path = "/api/kb/documents/{id}/assets/presign",
    tag = "知识库",
    description = "批量获取文档解析资源的短期访问 URL",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn presign_assets(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<PresignDocumentAssetsRequest>,
) -> Result<Response> {
    let doc_id = parse_uuid(id)?;
    let doc = service::get_document(&ctx.db, doc_id, tc.tenant_id)
        .await
        .model_err()?;
    ensure_document_readable(&doc, tc.user_id)?;

    let registered_assets = extract_document_assets(doc.metadata.as_ref())?;
    let requested: HashSet<&str> = params.asset_keys.iter().map(String::as_str).collect();
    let allowed: HashSet<&str> = registered_assets
        .iter()
        .map(|asset| asset.storage_key.as_str())
        .collect();
    if !requested.is_subset(&allowed) {
        return Err(err_forbidden(
            "knowledge_base.asset_not_registered",
            "requested asset does not belong to this document",
        ));
    }

    let s3 = ctx.shared_store.get::<SharedS3Client>().ok_or_else(|| {
        err_internal(
            "storage.not_initialized",
            "S3 storage client is not initialized",
        )
    })?;
    let s3_config = ctx.shared_store.get::<SharedS3Config>().ok_or_else(|| {
        err_internal(
            "storage.config_not_initialized",
            "S3 storage config is not initialized",
        )
    })?;
    let expires_in = Duration::from_secs(ASSET_PRESIGN_TTL_SECONDS);
    let presign_config = PresigningConfig::expires_in(expires_in).map_err(|e| {
        err_internal(
            "knowledge_base.asset_presign_config_failed",
            format!("failed to create presign config: {e}"),
        )
    })?;

    let mut items = Vec::with_capacity(params.asset_keys.len());
    for asset_key in params.asset_keys {
        ensure_valid_asset_key(&asset_key)?;
        let request = s3
            .get_object()
            .bucket(&s3_config.bucket)
            .key(&asset_key)
            .presigned(presign_config.clone())
            .await
            .map_err(|e| {
                err_internal(
                    "knowledge_base.asset_presign_failed",
                    format!("failed to presign document asset: {e}"),
                )
            })?;
        items.push(PresignedDocumentAssetResponse {
            asset_key,
            url: request.uri().to_string(),
            expires_in: ASSET_PRESIGN_TTL_SECONDS,
        });
    }

    format::json(PresignDocumentAssetsResponse { items })
}

#[utoipa::path(
    delete,
    path = "/api/kb-documents/{id}",
    tag = "知识库",
    description = "删除文档",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn delete(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let doc_id = parse_uuid(id)?;

    // Verify document exists and belongs to tenant
    let _doc = service::get_document(&ctx.db, doc_id, tc.tenant_id)
        .await
        .model_err()?;

    // Delete from Qdrant
    let search_provider =
        ctx.shared_store
            .get::<SharedSearchProvider>()
            .ok_or_else(|| {
                crate::views::errors::err_internal(
                    "knowledge_base.search_provider_not_initialized",
                    "Search provider not initialized",
                )
            })?;
    search_provider
        .delete_by_document(doc_id, tc.tenant_id)
        .await
        .map_err(|e| KnowledgeBaseError::IndexingError(e.to_string()).to_err())?;

    // Delete chunks from PG
    kb_chunks::Entity::delete_many()
        .filter(kb_chunks::Column::DocumentId.eq(doc_id))
        .exec(&ctx.db)
        .await
        .model_err()?;

    // Delete document_lines from PG
    document_lines::Entity::delete_many()
        .filter(document_lines::Column::DocumentId.eq(doc_id))
        .exec(&ctx.db)
        .await
        .model_err()?;

    // Delete the document itself
    kb_documents::Entity::delete_by_id(doc_id)
        .exec(&ctx.db)
        .await
        .model_err()?;

    format::json(serde_json::json!({"success": true}))
}

fn ensure_document_readable(
    doc: &kb_documents::Model,
    user_id: uuid::Uuid,
) -> Result<()> {
    if doc.scope == "private" && doc.created_by != user_id {
        return Err(err_forbidden(
            "knowledge_base.document_forbidden",
            "document is private",
        ));
    }
    Ok(())
}

fn extract_document_assets(
    metadata: Option<&serde_json::Value>,
) -> Result<Vec<DocumentAssetResponse>> {
    let Some(assets) = metadata
        .and_then(|value| value.pointer("/parser/assets"))
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(Vec::new());
    };

    assets
        .iter()
        .cloned()
        .map(|value| {
            serde_json::from_value::<DocumentAssetResponse>(value).map_err(|e| {
                err_internal(
                    "knowledge_base.asset_metadata_invalid",
                    format!("invalid document asset metadata: {e}"),
                )
            })
        })
        .collect()
}

fn ensure_valid_asset_key(asset_key: &str) -> Result<()> {
    if !asset_key.starts_with("kb-assets/") {
        return Err(err_forbidden(
            "knowledge_base.asset_key_invalid",
            "invalid document asset key",
        ));
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/kb-documents/{id}/promote",
    tag = "知识库",
    description = "将文档从private提升为tenant共享",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn promote(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let doc_id = parse_uuid(id)?;

    let doc = service::promote_document(&ctx.db, doc_id, tc.tenant_id, tc.user_id)
        .await
        .model_err()?;

    let search_provider =
        ctx.shared_store
            .get::<SharedSearchProvider>()
            .ok_or_else(|| {
                crate::views::errors::err_internal(
                    "knowledge_base.search_provider_not_initialized",
                    "Search provider not initialized",
                )
            })?;
    search_provider
        .promote_document_scope(doc_id, tc.tenant_id)
        .await
        .map_err(|e| KnowledgeBaseError::IndexingError(e.to_string()).to_err())?;

    format::json(DocumentResponse::from_model(&doc))
}

#[utoipa::path(
    post,
    path = "/api/kb-documents/{id}/reindex",
    tag = "知识库",
    description = "重新索引文档",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn reindex(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    use crate::modules::knowledge_base::models::kb_documents as kd_models;
    use sea_orm::{ActiveModelTrait, ActiveValue};
    let doc_id = parse_uuid(id)?;

    // Verify document exists and belongs to tenant
    let _doc = service::get_document(&ctx.db, doc_id, tc.tenant_id)
        .await
        .model_err()?;

    // Delete existing chunks from PG
    kb_chunks::Entity::delete_many()
        .filter(kb_chunks::Column::DocumentId.eq(doc_id))
        .exec(&ctx.db)
        .await
        .model_err()?;

    // Delete from Qdrant
    let search_provider =
        ctx.shared_store
            .get::<SharedSearchProvider>()
            .ok_or_else(|| {
                crate::views::errors::err_internal(
                    "knowledge_base.search_provider_not_initialized",
                    "Search provider not initialized",
                )
            })?;
    search_provider
        .delete_by_document(doc_id, tc.tenant_id)
        .await
        .map_err(|e| KnowledgeBaseError::IndexingError(e.to_string()).to_err())?;

    // Reset status to 'pending' — bypass transition validation by direct update
    let settings: AppSettings = ctx
        .config
        .typed_settings()
        .map_err(|e| {
            KnowledgeBaseError::ConfigError(format!("invalid settings: {e}")).to_err()
        })?
        .ok_or_else(|| {
            KnowledgeBaseError::ConfigError("settings missing".into()).to_err()
        })?;
    let _kb_config = settings.knowledge_base.as_ref().ok_or_else(|| {
        KnowledgeBaseError::ConfigError("knowledge base not configured".into()).to_err()
    })?;

    // Delete document_lines too (they'll be regenerated by the worker)
    document_lines::Entity::delete_many()
        .filter(document_lines::Column::DocumentId.eq(doc_id))
        .exec(&ctx.db)
        .await
        .model_err()?;

    // Reset status: we need to go back to 'pending' regardless of current state.
    // document_service::update_status only allows valid transitions, so we do a direct update.
    let doc = kb_documents::Entity::find_by_id(doc_id)
        .one(&ctx.db)
        .await
        .model_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())?;

    let mut active: kd_models::ActiveModel = doc.into();
    active.status = ActiveValue::Set("pending".to_string());
    active.chunk_count = ActiveValue::Set(0);
    active.total_tokens = ActiveValue::Set(0);
    active.error_message = ActiveValue::Set(None);
    active.update(&ctx.db).await.model_err()?;

    // Enqueue indexing worker
    IndexingWorker::perform_later(
        &ctx,
        IndexingWorkerArgs {
            document_id: doc_id,
            tenant_id: tc.tenant_id,
            trace_id: None,
            parent_span_id: None,
        },
    )
    .await
    .model_err()?;

    format::json(serde_json::json!({"success": true}))
}
