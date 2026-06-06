use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};

use crate::config::{AppSettings, ConfigExt};
use crate::extractors::TenantContext;
use crate::initializers::knowledge_base::SharedSearchProvider;
use crate::models::_entities::{document_lines, kb_chunks, kb_documents};
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::service;
use crate::modules::knowledge_base::views::{
    CreateDocumentRequest, DocumentListQuery, DocumentResponse,
};
use crate::utils::error::IntoModelResult;
use crate::views::errors::{err_bad_request, parse_uuid};
use crate::views::pagination::PaginatedResponse;
use crate::workers::indexing_worker::{IndexingWorker, IndexingWorkerArgs};

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

    let doc = service::document_service::create_document(
        &ctx.db,
        &service::document_service::CreateDocumentParams {
            tenant_id: tc.tenant_id,
            title: params.title,
            description: params.description,
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
