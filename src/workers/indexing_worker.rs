use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::time::Instant;

use futures_util::FutureExt;
use loco_rs::prelude::*;
use sea_orm::{ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::AsyncReadExt;
use tracing::Instrument;
use uuid::Uuid;

use aws_sdk_s3::primitives::ByteStream;

use crate::config::{AppSettings, ConfigExt};
use crate::initializers::knowledge_base::{SharedParserChain, SharedSearchProvider};
use crate::initializers::s3::{SharedS3Client, SharedS3Config};
use crate::models::_entities::{file_references, files, kb_documents};
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::models::{
    document_lines as dl_models, kb_chunks as kc_models,
};
use crate::modules::knowledge_base::providers::parser::{ParsedAsset, ParsedDocument};
use crate::modules::knowledge_base::providers::search::ChunkPoint;
use crate::modules::knowledge_base::providers::SharedEmbeddingClient;
use crate::modules::knowledge_base::service::numeric::embedding_vec_f64_to_f32;
use crate::modules::knowledge_base::service::{
    chunking_service, document_service, line_splitting_service,
};
use crate::services::resource_types::ResourceType;
use crate::services::worker_run_service::{
    WorkerRunStart, WorkerRunTracker, KNOWLEDGE_BASE_INDEXING_BUSINESS_TYPE,
    KNOWLEDGE_BASE_INDEXING_RUN_DEFINITION, KNOWLEDGE_BASE_INDEXING_WORKER_NAME,
};
use crate::utils::mime::detect_mime;
use rig::client::EmbeddingsClient;
use rig::embeddings::EmbeddingModel;

const MAX_INDEX_FILE_BYTES: i64 = 50 * 1024 * 1024;
const EMBEDDING_PROGRESS_BATCH_SIZE: usize = 16;
const LEGACY_TEXT_SOURCE_TYPES: &[&str] = &["kb_upload", "chat_upload", "api", "sync"];

macro_rules! log_index_stage_completed {
    ($document_id:expr, $stage:literal, $started_at:expr $(, $field:ident = $value:expr)* $(,)?) => {
        tracing::info!(
            document_id = %$document_id,
            stage = $stage,
            elapsed_ms = elapsed_ms($started_at),
            $($field = $value,)*
            "knowledge base indexing stage completed"
        );
    };
}

pub struct IndexingWorker {
    pub ctx: AppContext,
}

#[derive(Deserialize, Debug, Serialize, Clone)]
pub struct IndexingWorkerArgs {
    pub document_id: Uuid,
    pub tenant_id: Uuid,
    pub trace_id: Option<String>,
    pub parent_span_id: Option<String>,
}

#[async_trait]
impl BackgroundWorker<IndexingWorkerArgs> for IndexingWorker {
    fn build(ctx: &AppContext) -> Self {
        Self { ctx: ctx.clone() }
    }

    async fn perform(&self, args: IndexingWorkerArgs) -> Result<()> {
        let trace_id_str = args.trace_id.as_deref().unwrap_or("untraced");
        let parent_span_id_str = args.parent_span_id.as_deref().unwrap_or("");
        let span = tracing::info_span!(
            "indexing_worker",
            document_id = %args.document_id,
            tenant_id = %args.tenant_id,
            trace_id = %trace_id_str,
            parent_span_id = %parent_span_id_str,
        );

        let ctx = self.ctx.clone();
        let document_id = args.document_id;
        let tenant_id = args.tenant_id;

        async move {
            run_indexing_pipeline(&ctx, document_id, tenant_id, args.trace_id).await
        }
            .instrument(span)
            .await
    }
}

async fn run_indexing_pipeline(
    ctx: &AppContext,
    document_id: Uuid,
    tenant_id: Uuid,
    trace_id: Option<String>,
) -> Result<()> {
    let tracker = WorkerRunTracker::start(
        &ctx.db,
        WorkerRunStart {
            tenant_id: Some(tenant_id),
            worker_name: KNOWLEDGE_BASE_INDEXING_WORKER_NAME,
            business_type: KNOWLEDGE_BASE_INDEXING_BUSINESS_TYPE,
            business_id: document_id.to_string(),
            definition: KNOWLEDGE_BASE_INDEXING_RUN_DEFINITION,
            trace_id,
            metadata: None,
        },
    )
    .await?;
    let pipeline_result = AssertUnwindSafe(async {
        let db = &ctx.db;

        // 1. Get dependencies from shared_store
        let parser_chain =
            ctx.shared_store.get::<SharedParserChain>().ok_or_else(|| {
                KnowledgeBaseError::ConfigError("Parser chain not initialized".into())
                    .to_loco_error()
            })?;
        let embedding_client = ctx
            .shared_store
            .get::<SharedEmbeddingClient>()
            .ok_or_else(|| {
                KnowledgeBaseError::ConfigError("Embedding client not initialized".into())
                    .to_loco_error()
            })?;
        let search_provider =
            ctx.shared_store
                .get::<SharedSearchProvider>()
                .ok_or_else(|| {
                    KnowledgeBaseError::ConfigError(
                        "Search provider not initialized".into(),
                    )
                    .to_loco_error()
                })?;
        let s3_client = ctx.shared_store.get::<SharedS3Client>();
        let s3_config = ctx.shared_store.get::<SharedS3Config>();

        // Read KB config
        let settings: AppSettings = ctx
            .config
            .typed_settings()
            .map_err(|e| {
                KnowledgeBaseError::ConfigError(format!("invalid settings: {e}"))
                    .to_loco_error()
            })?
            .ok_or_else(|| {
                KnowledgeBaseError::ConfigError("settings missing".into()).to_loco_error()
            })?;
        let kb_config = settings.knowledge_base.as_ref().ok_or_else(|| {
            KnowledgeBaseError::ConfigError("knowledge base config missing".into())
                .to_loco_error()
        })?;
        let config = &kb_config.chunking;

        // 2. Get document from DB
        let doc = document_service::get_document(db, document_id, tenant_id).await?;
        document_service::start_indexing(db, document_id, tenant_id).await?;

        record_indexing_progress(
            &tracker,
            document_id,
            "load_file",
            Some("正在读取上传文件"),
        )
        .await;
        let input = load_pipeline_input(ctx, db, &doc).await?;
        if indexing_was_cancelled(db, document_id, tenant_id).await? {
            return Ok(());
        }
        execute_pipeline(
            db,
            &PipelineParams {
                parser_chain: &parser_chain,
                embedding_client: &embedding_client,
                search_provider: &search_provider,
                document_id,
                tenant_id,
                input_bytes: &input.bytes,
                mime_type: &input.mime_type,
                source_name: &input.source_name,
                s3_client: s3_client.as_ref(),
                s3_bucket: s3_config.as_ref().map(|cfg| cfg.bucket.as_str()),
                scope: &doc.scope,
                created_by: doc.created_by,
                library_id: doc.library_id,
                folder_id: doc.folder_id,
                config,
                embedding_model_name: &kb_config.embedding.model,
                tracker: &tracker,
            },
        )
        .await
    })
    .catch_unwind()
    .await;

    handle_pipeline_outcome(ctx, document_id, tenant_id, &tracker, pipeline_result).await
}

async fn handle_pipeline_outcome(
    ctx: &AppContext,
    document_id: Uuid,
    tenant_id: Uuid,
    tracker: &WorkerRunTracker,
    pipeline_result: std::result::Result<Result<()>, Box<dyn std::any::Any + Send>>,
) -> Result<()> {
    match pipeline_result {
        Ok(Ok(())) => {
            tracker.succeed().await?;
            Ok(())
        }
        Ok(Err(e)) => {
            cleanup_partial_index(ctx, document_id, tenant_id).await;
            let error_msg = indexing_error_message(&e);
            let _ = tracker.fail(&error_msg).await;
            mark_indexing_failed(&ctx.db, document_id, tenant_id, &error_msg).await;
            Err(e)
        }
        Err(panic_payload) => {
            let error_msg = panic_message(&panic_payload);
            cleanup_partial_index(ctx, document_id, tenant_id).await;
            let _ = tracker.fail(&error_msg).await;
            mark_indexing_failed(&ctx.db, document_id, tenant_id, &error_msg).await;
            Err(loco_rs::Error::string(&error_msg))
        }
    }
}

async fn cleanup_partial_index(ctx: &AppContext, document_id: Uuid, tenant_id: Uuid) {
    if let Err(error) =
        document_service::clear_index_records(&ctx.db, document_id, tenant_id).await
    {
        tracing::warn!(
            document_id = %document_id,
            error = %error,
            "failed to clean partial database index records"
        );
    }

    let Some(search_provider) = ctx.shared_store.get::<SharedSearchProvider>() else {
        return;
    };
    if let Err(error) = search_provider
        .delete_by_document(document_id, tenant_id)
        .await
    {
        tracing::warn!(
            document_id = %document_id,
            error = %error,
            "failed to clean partial vector index records"
        );
    }
}

async fn mark_indexing_failed(
    db: &DatabaseConnection,
    document_id: Uuid,
    tenant_id: Uuid,
    error_msg: &str,
) {
    tracing::error!(
        document_id = %document_id,
        error = %error_msg,
        "indexing pipeline failed"
    );
    if let Err(update_err) =
        document_service::mark_error(db, document_id, tenant_id, error_msg).await
    {
        tracing::error!(
            document_id = %document_id,
            error = %update_err,
            "failed to mark document indexing error"
        );
    }
}

fn indexing_error_message(error: &loco_rs::Error) -> String {
    if let loco_rs::Error::CustomError(_, detail) = error {
        if let Some(description) = detail.description.as_deref() {
            return description.to_string();
        }
        if let Some(code) = detail.error.as_deref() {
            return code.to_string();
        }
    }

    let display = error.to_string();
    if display.is_empty() {
        format!("{error:?}")
    } else {
        display
    }
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload.downcast_ref::<&str>().map_or_else(
        || {
            payload.downcast_ref::<String>().map_or_else(
                || "indexing pipeline panicked".to_string(),
                |message| format!("indexing pipeline panicked: {message}"),
            )
        },
        |message| format!("indexing pipeline panicked: {message}"),
    )
}

struct PipelineInput {
    bytes: Vec<u8>,
    mime_type: String,
    source_name: String,
}

async fn load_pipeline_input(
    ctx: &AppContext,
    db: &DatabaseConnection,
    doc: &kb_documents::Model,
) -> Result<PipelineInput> {
    if document_has_file_source(doc) {
        return load_file_pipeline_input(ctx, db, doc).await;
    }

    if let Some(full_text) = &doc.full_text {
        return Ok(PipelineInput {
            bytes: full_text.as_bytes().to_vec(),
            mime_type: normalise_source_mime(&doc.source_type),
            source_name: doc.title.clone(),
        });
    }

    load_file_pipeline_input(ctx, db, doc).await
}

const fn document_has_file_source(doc: &kb_documents::Model) -> bool {
    doc.file_reference_id.is_some() || doc.file_id.is_some()
}

async fn load_file_pipeline_input(
    ctx: &AppContext,
    db: &DatabaseConnection,
    doc: &kb_documents::Model,
) -> Result<PipelineInput> {
    let file_input = load_document_file_input(db, doc).await?;

    if file_input.file.size > MAX_INDEX_FILE_BYTES {
        return Err(KnowledgeBaseError::ParsingError(format!(
            "file is too large to index synchronously: {} bytes > {} bytes",
            file_input.file.size, MAX_INDEX_FILE_BYTES
        ))
        .to_loco_error());
    }

    let s3_client = ctx.shared_store.get::<SharedS3Client>().ok_or_else(|| {
        KnowledgeBaseError::ConfigError("S3 client not initialized".into())
            .to_loco_error()
    })?;
    let bytes = read_file_bytes(&s3_client, &file_input.file).await?;
    let (mime_type, source_name) =
        normalise_parser_input(&bytes, file_input.mime_type, file_input.source_name)
            .map_err(|message| {
                KnowledgeBaseError::ParsingError(message).to_loco_error()
            })?;

    Ok(PipelineInput {
        bytes,
        mime_type,
        source_name,
    })
}

struct FileInput {
    file: files::Model,
    mime_type: String,
    source_name: String,
}

async fn load_document_file_input(
    db: &DatabaseConnection,
    doc: &kb_documents::Model,
) -> Result<FileInput> {
    if let Some(reference_id) = doc.file_reference_id {
        let reference = file_references::Entity::find_by_id(reference_id)
            .filter(file_references::Column::TenantId.eq(doc.tenant_id))
            .filter(
                file_references::Column::ResourceType
                    .eq(ResourceType::KnowledgeBaseDocument.as_str()),
            )
            .filter(file_references::Column::ResourceId.eq(doc.id.to_string()))
            .filter(file_references::Column::DeletedAt.is_null())
            .one(db)
            .await
            .map_err(|e| {
                KnowledgeBaseError::ProviderError(e.to_string()).to_loco_error()
            })?
            .ok_or_else(|| KnowledgeBaseError::NotFound.to_loco_error())?;
        let file = load_active_file(db, doc.tenant_id, reference.file_id).await?;
        return Ok(FileInput {
            mime_type: resolve_reference_document_mime(
                &doc.source_type,
                &reference,
                &file,
            ),
            source_name: reference.display_name.unwrap_or_else(|| file.name.clone()),
            file,
        });
    }

    let file_id = doc.file_id.ok_or_else(|| {
        KnowledgeBaseError::ParsingError(
            "document has neither content nor file_id".into(),
        )
        .to_loco_error()
    })?;

    let file = load_active_file(db, doc.tenant_id, file_id).await?;
    Ok(FileInput {
        mime_type: resolve_file_document_mime(&doc.source_type, &file),
        source_name: file.name.clone(),
        file,
    })
}

async fn load_active_file(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    file_id: Uuid,
) -> Result<files::Model> {
    files::Entity::find_by_id(file_id)
        .filter(files::Column::TenantId.eq(tenant_id))
        .filter(files::Column::Status.eq("ACTIVE"))
        .filter(files::Column::DeletedAt.is_null())
        .one(db)
        .await
        .map_err(|e| KnowledgeBaseError::ProviderError(e.to_string()).to_loco_error())?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_loco_error())
}

fn normalise_source_mime(source_type: &str) -> String {
    if LEGACY_TEXT_SOURCE_TYPES.contains(&source_type) {
        "text/plain".to_string()
    } else {
        source_type.to_string()
    }
}

fn resolve_file_document_mime(source_type: &str, file: &files::Model) -> String {
    if !LEGACY_TEXT_SOURCE_TYPES.contains(&source_type) {
        return source_type.to_string();
    }

    if file.mime_type != "application/octet-stream" {
        return file.mime_type.clone();
    }

    infer_mime_from_name(&file.name).unwrap_or_else(|| file.mime_type.clone())
}

fn resolve_reference_document_mime(
    source_type: &str,
    reference: &file_references::Model,
    file: &files::Model,
) -> String {
    reference
        .mime_type
        .as_deref()
        .filter(|mime| !mime.trim().is_empty())
        .map_or_else(
            || resolve_file_document_mime(source_type, file),
            str::to_string,
        )
}

fn infer_mime_from_name(name: &str) -> Option<String> {
    let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
    let mime = match extension.as_str() {
        "md" | "markdown" => "text/markdown",
        "txt" => "text/plain",
        "pdf" => "application/pdf",
        "docx" => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        "pptx" => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        }
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        _ => return None,
    };
    Some(mime.to_string())
}

fn normalise_parser_input(
    bytes: &[u8],
    declared_mime: String,
    source_name: String,
) -> std::result::Result<(String, String), String> {
    let detected_mime = detect_mime(bytes);
    let extension_mime = infer_mime_from_name(&source_name);
    let magic_prefix = bytes
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");

    tracing::debug!(
        declared_mime,
        detected_mime,
        extension_mime = extension_mime.as_deref().unwrap_or("unknown"),
        source_name,
        magic_prefix,
        "knowledge base parser input MIME detection result"
    );

    if is_supported_document_mime(detected_mime)
        && detected_mime != "application/octet-stream"
    {
        ensure_detected_mime_matches_metadata(
            detected_mime,
            &declared_mime,
            extension_mime.as_deref(),
            &source_name,
        )?;
        return Ok((
            effective_parser_mime(
                detected_mime,
                &declared_mime,
                extension_mime.as_deref(),
            ),
            source_name,
        ));
    }

    if extension_mime.as_deref().is_some_and(|mime| {
        is_supported_document_mime(mime) && !mime_matches_metadata(mime, &declared_mime)
    }) {
        return Err(format!(
            "文件名后缀与记录的 MIME 类型不一致: sourceName='{source_name}', extensionMime='{}', declaredMime='{declared_mime}'",
            extension_mime.as_deref().unwrap_or_default()
        ));
    }

    Ok((declared_mime, source_name))
}

fn ensure_detected_mime_matches_metadata(
    detected_mime: &str,
    declared_mime: &str,
    extension_mime: Option<&str>,
    source_name: &str,
) -> std::result::Result<(), String> {
    if is_supported_document_mime(declared_mime)
        && !mime_matches_metadata(detected_mime, declared_mime)
    {
        return Err(format!(
            "文件内容类型与记录的 MIME 类型不一致: detectedMime='{detected_mime}', declaredMime='{declared_mime}', sourceName='{source_name}'"
        ));
    }

    if extension_mime
        .filter(|mime| is_supported_document_mime(mime))
        .is_some_and(|mime| !mime_matches_metadata(detected_mime, mime))
    {
        return Err(format!(
            "文件内容类型与文件名后缀不一致: detectedMime='{detected_mime}', extensionMime='{}', sourceName='{source_name}'",
            extension_mime.unwrap_or_default()
        ));
    }

    Ok(())
}

fn mime_matches_metadata(actual_mime: &str, metadata_mime: &str) -> bool {
    actual_mime == metadata_mime
        || is_text_document_mime(actual_mime) && is_text_document_mime(metadata_mime)
}

fn effective_parser_mime(
    detected_mime: &str,
    declared_mime: &str,
    extension_mime: Option<&str>,
) -> String {
    if is_text_document_mime(detected_mime)
        && (extension_mime == Some("text/markdown") || declared_mime == "text/markdown")
    {
        return "text/markdown".to_string();
    }

    detected_mime.to_string()
}

fn is_text_document_mime(mime_type: &str) -> bool {
    matches!(mime_type, "text/plain" | "text/markdown")
}

fn is_supported_document_mime(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "text/plain"
            | "text/markdown"
            | "application/pdf"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "image/png"
            | "image/jpeg"
            | "image/webp"
            | "image/bmp"
            | "image/tiff"
    )
}

async fn read_file_bytes(
    s3_client: &SharedS3Client,
    file: &files::Model,
) -> Result<Vec<u8>> {
    let response = s3_client
        .get_object()
        .bucket(&file.bucket)
        .key(&file.storage_key)
        .send()
        .await
        .map_err(|e| {
            KnowledgeBaseError::ProviderError(format!(
                "failed to fetch file object from storage: {e}"
            ))
            .to_loco_error()
        })?;

    let capacity = usize::try_from(file.size.max(0)).unwrap_or_default();
    let mut bytes = Vec::with_capacity(capacity);
    let mut reader = response.body.into_async_read();
    reader.read_to_end(&mut bytes).await.map_err(|e| {
        KnowledgeBaseError::ProviderError(format!(
            "failed to read file object from storage: {e}"
        ))
        .to_loco_error()
    })?;
    Ok(bytes)
}

/// Parameters for [`execute_pipeline`].
struct PipelineParams<'a> {
    parser_chain: &'a SharedParserChain,
    embedding_client: &'a SharedEmbeddingClient,
    search_provider: &'a SharedSearchProvider,
    document_id: Uuid,
    tenant_id: Uuid,
    input_bytes: &'a [u8],
    mime_type: &'a str,
    source_name: &'a str,
    s3_client: Option<&'a SharedS3Client>,
    s3_bucket: Option<&'a str>,
    scope: &'a str,
    created_by: Uuid,
    library_id: Option<Uuid>,
    folder_id: Option<Uuid>,
    config: &'a crate::config::ChunkingConfig,
    embedding_model_name: &'a str,
    tracker: &'a WorkerRunTracker,
}

fn build_chunk_points_and_models(
    chunks: &[chunking_service::RawChunk],
    embeddings: &[rig::embeddings::Embedding],
    p: &PipelineParams<'_>,
) -> (Vec<ChunkPoint>, Vec<kc_models::ActiveModel>, i32) {
    let mut chunk_points = Vec::with_capacity(chunks.len());
    let mut chunk_models = Vec::with_capacity(chunks.len());
    let mut total_tokens: i32 = 0;

    for (i, (chunk, embedding)) in chunks.iter().zip(embeddings.iter()).enumerate() {
        let chunk_id = Uuid::now_v7();
        let embedding_f32 = embedding_vec_f64_to_f32(&embedding.vec);
        total_tokens += chunk.token_count;

        chunk_points.push(ChunkPoint {
            chunk_id,
            document_id: p.document_id,
            chunk_index: i32::try_from(i).unwrap_or(i32::MAX),
            content: chunk.content.clone(),
            heading_path: chunk.heading_path.clone(),
            page_number: None,
            char_start: Some(chunk.char_start),
            char_end: Some(chunk.char_end),
            token_count: chunk.token_count,
            embedding: embedding_f32,
            library_id: p.library_id,
            folder_id: p.folder_id,
            scope: p.scope.to_string(),
            created_by: p.created_by,
        });

        chunk_models.push(kc_models::ActiveModel {
            id: ActiveValue::Set(chunk_id),
            document_id: ActiveValue::Set(p.document_id),
            tenant_id: ActiveValue::Set(p.tenant_id),
            chunk_index: ActiveValue::Set(i32::try_from(i).unwrap_or(i32::MAX)),
            content: ActiveValue::Set(chunk.content.clone()),
            heading_path: ActiveValue::Set(chunk.heading_path.clone()),
            page_number: ActiveValue::Set(None),
            token_count: ActiveValue::Set(chunk.token_count),
            char_start: ActiveValue::Set(Some(chunk.char_start)),
            char_end: ActiveValue::Set(Some(chunk.char_end)),
            ..Default::default()
        });
    }

    (chunk_points, chunk_models, total_tokens)
}

async fn execute_pipeline(db: &DatabaseConnection, p: &PipelineParams<'_>) -> Result<()> {
    let pipeline_started_at = Instant::now();

    let index_markdown = parse_prepare_and_save(db, p).await?;
    if indexing_was_cancelled(db, p.document_id, p.tenant_id).await? {
        return Ok(());
    }

    // 5. Chunk the markdown
    record_pipeline_progress(p, "chunk", Some("正在切分索引内容")).await;
    let chunk_started_at = Instant::now();
    let chunks = chunk_index_markdown(p, &index_markdown)?;
    log_index_stage_completed!(
        p.document_id,
        "chunk",
        chunk_started_at,
        chunk_count = chunks.len(),
        total_tokens = chunks.iter().map(|chunk| chunk.token_count).sum::<i32>(),
    );
    if indexing_was_cancelled(db, p.document_id, p.tenant_id).await? {
        return Ok(());
    }

    // 6. Split lines and insert
    record_pipeline_progress(p, "lines", Some("正在保存原文行号")).await;
    let lines_started_at = Instant::now();
    insert_document_lines(db, p, &index_markdown).await?;
    log_index_stage_completed!(p.document_id, "lines", lines_started_at);
    if indexing_was_cancelled(db, p.document_id, p.tenant_id).await? {
        return Ok(());
    }

    // 7. Generate embeddings
    record_pipeline_progress(p, "embedding", Some("正在为文档分块生成向量")).await;
    let embedding_started_at = Instant::now();
    let embeddings = generate_embeddings(db, p, &chunks).await?;
    log_index_stage_completed!(
        p.document_id,
        "embedding",
        embedding_started_at,
        embedding_count = embeddings.len(),
    );
    if indexing_was_cancelled(db, p.document_id, p.tenant_id).await? {
        return Ok(());
    }

    // 8-10. Build chunks, save chunks and upsert vectors
    record_pipeline_progress(p, "persist", Some("正在写入分块和向量索引")).await;
    let persist_started_at = Instant::now();
    let total_tokens = persist_chunks_and_vectors(db, p, &chunks, &embeddings).await?;
    log_index_stage_completed!(
        p.document_id,
        "persist",
        persist_started_at,
        chunk_count = chunks.len(),
        total_tokens = total_tokens,
    );
    if indexing_was_cancelled(db, p.document_id, p.tenant_id).await? {
        return Ok(());
    }

    // 11. Mark document as ready
    record_pipeline_progress(p, "mark_ready", Some("正在更新文档状态")).await;
    let ready_started_at = Instant::now();
    mark_document_ready(db, p, chunks.len(), total_tokens).await?;
    log_index_stage_completed!(p.document_id, "mark_ready", ready_started_at);

    tracing::info!(
        document_id = %p.document_id,
        chunk_count = chunks.len(),
        total_tokens = total_tokens,
        elapsed_ms = elapsed_ms(pipeline_started_at),
        "indexing pipeline completed"
    );

    Ok(())
}

async fn parse_prepare_and_save(
    db: &DatabaseConnection,
    p: &PipelineParams<'_>,
) -> Result<String> {
    record_pipeline_progress(p, "parse", Some("正在解析文档内容")).await;
    let parse_started_at = Instant::now();
    let parsed = parse_document(p).await?;
    log_index_stage_completed!(
        p.document_id,
        "parse",
        parse_started_at,
        markdown_chars = parsed.markdown.chars().count(),
        asset_count = parsed.assets.len(),
    );
    if indexing_was_cancelled(db, p.document_id, p.tenant_id).await? {
        return Ok(String::new());
    }

    let assets_started_at = Instant::now();
    record_pipeline_progress(p, "assets", Some("正在处理解析出的图片资源")).await;
    let parsed_markdown = prepare_parsed_markdown(p, &parsed.markdown, &parsed.assets)
        .instrument(tracing::info_span!(
            "kb.index.assets",
            document_id = %p.document_id,
            asset_count = parsed.assets.len(),
        ))
        .await?;
    log_index_stage_completed!(
        p.document_id,
        "assets",
        assets_started_at,
        preview_chars = parsed_markdown.preview_markdown.chars().count(),
        index_chars = parsed_markdown.index_markdown.chars().count(),
    );
    if indexing_was_cancelled(db, p.document_id, p.tenant_id).await? {
        return Ok(String::new());
    }

    save_parsed_markdown(db, p, &parsed_markdown).await?;
    Ok(parsed_markdown.index_markdown)
}

async fn save_parsed_markdown(
    db: &DatabaseConnection,
    p: &PipelineParams<'_>,
    parsed_markdown: &ParsedMarkdown,
) -> Result<()> {
    let started_at = Instant::now();
    record_pipeline_progress(p, "save_parsed", Some("正在保存预览内容")).await;
    async {
        document_service::set_parsed_content(
            db,
            p.document_id,
            &parsed_markdown.preview_markdown,
            parsed_markdown.asset_metadata.clone(),
        )
        .await
    }
    .instrument(tracing::info_span!(
        "kb.index.save_parsed",
        document_id = %p.document_id,
        preview_chars = parsed_markdown.preview_markdown.chars().count(),
        index_chars = parsed_markdown.index_markdown.chars().count(),
    ))
    .await?;
    log_index_stage_completed!(p.document_id, "save_parsed", started_at);
    Ok(())
}

async fn record_pipeline_progress(
    p: &PipelineParams<'_>,
    stage: &str,
    message: Option<&str>,
) {
    record_indexing_progress(p.tracker, p.document_id, stage, message).await;
}

async fn record_indexing_progress(
    tracker: &WorkerRunTracker,
    document_id: Uuid,
    stage: &str,
    message: Option<&str>,
) {
    if let Err(error) = tracker.stage(stage, message).await {
        tracing::warn!(
            document_id = %document_id,
            stage,
            error = %error,
            "failed to update indexing progress"
        );
    }
}

async fn indexing_was_cancelled(
    db: &DatabaseConnection,
    document_id: Uuid,
    tenant_id: Uuid,
) -> Result<bool> {
    let active = document_service::is_indexing_active(db, document_id, tenant_id).await?;
    if !active {
        tracing::info!(
            document_id = %document_id,
            "indexing pipeline stopped because document is no longer active"
        );
    }
    Ok(!active)
}

async fn parse_document(p: &PipelineParams<'_>) -> Result<ParsedDocument> {
    let parser = p
        .parser_chain
        .iter()
        .find(|pr| pr.supported_mime_types().contains(&p.mime_type))
        .ok_or_else(|| {
            KnowledgeBaseError::UnsupportedFormat(format!(
                "no parser found for content type '{}'",
                p.mime_type
            ))
            .to_loco_error()
        })?;

    async {
        parser
            .parse(p.input_bytes, p.mime_type, p.source_name)
            .await
            .map_err(|e| {
                KnowledgeBaseError::ParsingError(format!("parsing failed: {e}"))
                    .to_loco_error()
            })
    }
    .instrument(tracing::info_span!(
        "kb.index.parse",
        document_id = %p.document_id,
        mime_type = p.mime_type,
        source_name = p.source_name,
        input_bytes = p.input_bytes.len(),
    ))
    .await
}

fn chunk_index_markdown(
    p: &PipelineParams<'_>,
    index_markdown: &str,
) -> Result<Vec<chunking_service::RawChunk>> {
    let chunk_span = tracing::info_span!(
        "kb.index.chunk",
        document_id = %p.document_id,
        chunk_count = tracing::field::Empty,
        total_tokens = tracing::field::Empty,
    );
    let chunks = {
        let _guard = chunk_span.enter();
        chunking_service::chunk_markdown(
            index_markdown,
            chunking_service::ChunkMarkdownOptions {
                max_tokens: p.config.max_chunk_tokens,
                min_tokens: p.config.min_chunk_tokens,
                overlap_sentences: p.config.overlap_sentences,
                split_by_heading: p.config.split_by_heading,
                min_heading_level: p.config.min_heading_level,
                max_heading_level: p.config.max_heading_level,
            },
        )
    };
    if chunks.is_empty() {
        return Err(KnowledgeBaseError::IndexingError(
            "chunking produced no chunks".into(),
        )
        .to_loco_error());
    }
    let chunk_tokens: i32 = chunks.iter().map(|chunk| chunk.token_count).sum();
    chunk_span.record("chunk_count", chunks.len());
    chunk_span.record("total_tokens", chunk_tokens);
    Ok(chunks)
}

async fn insert_document_lines(
    db: &DatabaseConnection,
    p: &PipelineParams<'_>,
    index_markdown: &str,
) -> Result<()> {
    let line_span = tracing::info_span!(
        "kb.index.lines",
        document_id = %p.document_id,
        line_count = tracing::field::Empty,
    );
    let line_span_for_record = line_span.clone();
    async {
        let raw_lines = line_splitting_service::split_lines(index_markdown);
        line_span_for_record.record("line_count", raw_lines.len());
        let line_models: Vec<dl_models::ActiveModel> = raw_lines
            .into_iter()
            .map(|line| dl_models::ActiveModel {
                tenant_id: ActiveValue::Set(p.tenant_id),
                document_id: ActiveValue::Set(p.document_id),
                line_number: ActiveValue::Set(line.line_number),
                line_text: ActiveValue::Set(line.line_text),
                line_chars: ActiveValue::Set(line.line_chars),
                cumulative_chars: ActiveValue::Set(line.cumulative_chars),
                ..Default::default()
            })
            .collect();
        document_service::insert_lines(db, line_models).await
    }
    .instrument(line_span)
    .await
}

async fn generate_embeddings(
    db: &DatabaseConnection,
    p: &PipelineParams<'_>,
    chunks: &[chunking_service::RawChunk],
) -> Result<Vec<rig::embeddings::Embedding>> {
    let model = p.embedding_client.0.embedding_model(p.embedding_model_name);
    let embedding_span = tracing::info_span!(
        "kb.index.embedding",
        document_id = %p.document_id,
        chunk_count = chunks.len(),
        model = p.embedding_model_name,
    );
    let _guard = embedding_span.enter();
    let total_chunks = chunks.len();
    update_embedding_progress(db, p, 0, total_chunks).await;

    let mut embeddings = Vec::with_capacity(total_chunks);
    for chunk_batch in chunks.chunks(EMBEDDING_PROGRESS_BATCH_SIZE) {
        if indexing_was_cancelled(db, p.document_id, p.tenant_id).await? {
            return Ok(embeddings);
        }

        let texts: Vec<String> = chunk_batch
            .iter()
            .map(|chunk| chunk.content.clone())
            .collect();
        let batch_embeddings = model.embed_texts(texts).await.map_err(|e| {
            KnowledgeBaseError::EmbeddingError(format!("embedding failed: {e}"))
                .to_loco_error()
        })?;
        embeddings.extend(batch_embeddings);
        update_embedding_progress(db, p, embeddings.len(), total_chunks).await;
    }

    if embeddings.len() != chunks.len() {
        return Err(KnowledgeBaseError::EmbeddingError(format!(
            "embedding count mismatch: got {} embeddings for {} chunks",
            embeddings.len(),
            chunks.len()
        ))
        .to_loco_error());
    }
    Ok(embeddings)
}

async fn update_embedding_progress(
    _db: &DatabaseConnection,
    p: &PipelineParams<'_>,
    current: usize,
    total: usize,
) {
    let current_i32 = i32::try_from(current).unwrap_or(i32::MAX);
    let total_i32 = i32::try_from(total).unwrap_or(i32::MAX);
    let message = format!("正在生成向量：{current_i32}/{total_i32} 个分块");
    if let Err(error) = p
        .tracker
        .progress("embedding", current_i32, total_i32, Some(&message))
        .await
    {
        tracing::warn!(
            document_id = %p.document_id,
            error = %error,
            "failed to update embedding progress"
        );
    }
}

async fn persist_chunks_and_vectors(
    db: &DatabaseConnection,
    p: &PipelineParams<'_>,
    chunks: &[chunking_service::RawChunk],
    embeddings: &[rig::embeddings::Embedding],
) -> Result<i32> {
    if indexing_was_cancelled(db, p.document_id, p.tenant_id).await? {
        return Ok(0);
    }

    let build_span = tracing::info_span!(
        "kb.index.build_chunks",
        document_id = %p.document_id,
        chunk_count = tracing::field::Empty,
        total_tokens = tracing::field::Empty,
    );
    let (chunk_points, chunk_models, total_tokens) = {
        let _guard = build_span.enter();
        build_chunk_points_and_models(chunks, embeddings, p)
    };
    build_span.record("chunk_count", chunk_points.len());
    build_span.record("total_tokens", total_tokens);

    // 9. Write chunks to PG
    async { document_service::insert_chunks(db, chunk_models).await }
        .instrument(tracing::info_span!(
            "kb.index.save_chunks",
            document_id = %p.document_id,
            chunk_count = chunk_points.len(),
        ))
        .await?;

    if indexing_was_cancelled(db, p.document_id, p.tenant_id).await? {
        return Ok(total_tokens);
    }

    // 10. Write vectors to Qdrant
    async {
        p.search_provider
            .upsert_chunks(&chunk_points, p.tenant_id)
            .await
            .map_err(|e| {
                KnowledgeBaseError::IndexingError(format!("Qdrant upsert failed: {e}"))
                    .to_loco_error()
            })
    }
    .instrument(tracing::info_span!(
        "kb.index.upsert_vectors",
        document_id = %p.document_id,
        chunk_count = chunk_points.len(),
    ))
    .await
    .map(|()| total_tokens)
}

async fn mark_document_ready(
    db: &DatabaseConnection,
    p: &PipelineParams<'_>,
    chunk_count: usize,
    total_tokens: i32,
) -> Result<()> {
    async {
        document_service::mark_ready(
            db,
            p.document_id,
            i32::try_from(chunk_count).unwrap_or(i32::MAX),
            total_tokens,
        )
        .await
    }
    .instrument(tracing::info_span!(
        "kb.index.mark_ready",
        document_id = %p.document_id,
    ))
    .await
}

fn elapsed_ms(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

struct ParsedMarkdown {
    preview_markdown: String,
    index_markdown: String,
    asset_metadata: serde_json::Value,
}

async fn prepare_parsed_markdown(
    p: &PipelineParams<'_>,
    markdown: &str,
    assets: &[ParsedAsset],
) -> Result<ParsedMarkdown> {
    if assets.is_empty() {
        return Ok(ParsedMarkdown {
            preview_markdown: markdown.to_string(),
            index_markdown: strip_markdown_images(markdown),
            asset_metadata: json!({
                "parser": {
                    "mimeType": p.mime_type,
                    "sourceName": p.source_name,
                    "assets": []
                }
            }),
        });
    }

    let s3_client = p.s3_client.ok_or_else(|| {
        KnowledgeBaseError::ConfigError(
            "S3 client is required to persist parsed document assets".into(),
        )
        .to_loco_error()
    })?;
    let bucket = p.s3_bucket.ok_or_else(|| {
        KnowledgeBaseError::ConfigError(
            "S3 config is required to persist parsed document assets".into(),
        )
        .to_loco_error()
    })?;

    let mut asset_refs = Vec::with_capacity(assets.len());
    let mut asset_records = Vec::with_capacity(assets.len());
    for asset in assets {
        let asset_id = Uuid::now_v7();
        let ext = extension_for_mime(&asset.mime_type);
        let key = format!(
            "kb-assets/{}/{}/{}.{}",
            p.tenant_id, p.document_id, asset_id, ext
        );
        s3_client
            .put_object()
            .bucket(bucket)
            .key(&key)
            .content_type(asset.mime_type.clone())
            .body(ByteStream::from(asset.data.clone()))
            .send()
            .await
            .map_err(|e| {
                KnowledgeBaseError::ProviderError(format!(
                    "failed to upload parsed asset '{}': {e}",
                    asset.name
                ))
                .to_loco_error()
            })?;

        asset_refs.push((asset.name.clone(), key.clone()));
        asset_records.push(json!({
            "id": asset_id.to_string(),
            "name": asset.name,
            "mimeType": asset.mime_type,
            "storageKey": key,
            "size": asset.data.len()
        }));
    }

    let preview_markdown = rewrite_markdown_image_targets(markdown, &asset_refs);
    Ok(ParsedMarkdown {
        index_markdown: strip_markdown_images(&preview_markdown),
        preview_markdown,
        asset_metadata: json!({
            "parser": {
                "mimeType": p.mime_type,
                "sourceName": p.source_name,
                "assets": asset_records
            }
        }),
    })
}

fn rewrite_markdown_image_targets(
    markdown: &str,
    asset_refs: &[(String, String)],
) -> String {
    static IMAGE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = IMAGE_RE.get_or_init(|| {
        regex::Regex::new(r#"!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#)
            .expect("markdown image regex must compile")
    });
    let asset_keys = asset_refs
        .iter()
        .map(|(name, key)| (normalize_asset_target(name), key.as_str()))
        .collect::<HashMap<_, _>>();

    re.replace_all(markdown, |captures: &regex::Captures<'_>| {
        let Some(target) = captures.get(2).map(|m| normalize_asset_target(m.as_str()))
        else {
            return captures[0].to_string();
        };
        let key = asset_keys.get(target.as_str()).or_else(|| {
            asset_keys.iter().find_map(|(name, key)| {
                target.ends_with(&format!("/{name}")).then_some(key)
            })
        });

        key.map_or_else(
            || captures[0].to_string(),
            |storage_key| format!("![{}](kb-asset://{storage_key})", &captures[1]),
        )
    })
    .to_string()
}

fn normalize_asset_target(target: &str) -> String {
    target
        .trim()
        .trim_matches('"')
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

fn strip_markdown_images(markdown: &str) -> String {
    static IMAGE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = IMAGE_RE.get_or_init(|| {
        regex::Regex::new(r"!\[([^\]]*)\]\([^)]+\)")
            .expect("markdown image regex must compile")
    });
    re.replace_all(markdown, "$1").to_string()
}

fn extension_for_mime(mime_type: &str) -> &'static str {
    match mime_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        document_has_file_source, extension_for_mime, indexing_error_message,
        normalise_parser_input, normalize_asset_target, resolve_file_document_mime,
        resolve_reference_document_mime, rewrite_markdown_image_targets,
        strip_markdown_images,
    };
    use crate::models::_entities::{file_references, files, kb_documents};
    use uuid::Uuid;

    #[test]
    fn rewrite_markdown_image_targets_replaces_only_image_links() {
        let markdown = r#"
正文里的 chart.png 不应该被替换。

![图表](images/chart.png)
![照片](./photo.jpg "标题")
[普通链接](images/chart.png)
"#;
        let rewritten = rewrite_markdown_image_targets(
            markdown,
            &[
                (
                    "chart.png".to_string(),
                    "kb-assets/t/doc/chart.png".to_string(),
                ),
                (
                    "photo.jpg".to_string(),
                    "kb-assets/t/doc/photo.jpg".to_string(),
                ),
            ],
        );

        assert!(rewritten.contains("正文里的 chart.png 不应该被替换。"));
        assert!(rewritten.contains("![图表](kb-asset://kb-assets/t/doc/chart.png)"));
        assert!(rewritten.contains("![照片](kb-asset://kb-assets/t/doc/photo.jpg)"));
        assert!(rewritten.contains("[普通链接](images/chart.png)"));
    }

    #[test]
    fn rewrite_markdown_image_targets_keeps_unknown_images() {
        let markdown = "![missing](images/missing.png)";
        let rewritten = rewrite_markdown_image_targets(
            markdown,
            &[(
                "chart.png".to_string(),
                "kb-assets/t/doc/chart.png".to_string(),
            )],
        );

        assert_eq!(rewritten, markdown);
    }

    #[test]
    fn strip_markdown_images_keeps_alt_text_for_indexing() {
        let markdown = "前文\n![结构图](kb-asset://kb-assets/t/doc/a.png)\n后文";

        assert_eq!(strip_markdown_images(markdown), "前文\n结构图\n后文");
    }

    #[test]
    fn normalize_asset_target_handles_relative_windows_paths() {
        assert_eq!(
            normalize_asset_target(r#".\images\chart.png"#),
            "images/chart.png"
        );
        assert_eq!(normalize_asset_target(r#""./chart.png""#), "chart.png");
    }

    #[test]
    fn extension_for_mime_uses_supported_image_extensions() {
        assert_eq!(extension_for_mime("image/png"), "png");
        assert_eq!(extension_for_mime("image/jpeg"), "jpg");
        assert_eq!(extension_for_mime("image/webp"), "webp");
        assert_eq!(extension_for_mime("application/octet-stream"), "bin");
    }

    #[test]
    fn document_mime_prefers_reference_mime_type() {
        let file = file_model("stored.bin", "application/octet-stream");
        let reference = file_reference_model(Some("application/pdf"));

        assert_eq!(
            resolve_reference_document_mime("kb_upload", &reference, &file),
            "application/pdf"
        );
    }

    #[test]
    fn document_mime_falls_back_to_file_name_when_stored_as_octet_stream() {
        let file = file_model("知识库-v4.pdf", "application/octet-stream");
        let reference = file_reference_model(None);

        assert_eq!(
            resolve_reference_document_mime("kb_upload", &reference, &file),
            "application/pdf"
        );
        assert_eq!(
            resolve_file_document_mime("kb_upload", &file),
            "application/pdf"
        );
    }

    #[test]
    fn parser_input_rejects_pdf_when_extension_was_changed() {
        let err = normalise_parser_input(
            b"%PDF-1.7\nbody",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                .to_string(),
            "知识库-v4 - 副本.docx".to_string(),
        )
        .unwrap_err();

        assert!(err.contains("detectedMime='application/pdf'"));
        assert!(err.contains("declaredMime='application/vnd.openxmlformats-officedocument.wordprocessingml.document'"));
    }

    #[test]
    fn parser_input_rejects_plain_text_when_extension_was_changed() {
        let err = normalise_parser_input(
            b"plain text",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                .to_string(),
            "notes.docx".to_string(),
        )
        .unwrap_err();

        assert!(err.contains("detectedMime='text/plain'"));
        assert!(err.contains("declaredMime='application/vnd.openxmlformats-officedocument.wordprocessingml.document'"));
    }

    #[test]
    fn parser_input_accepts_matching_pdf_metadata() {
        let (mime_type, source_name) = normalise_parser_input(
            b"%PDF-1.7\nbody",
            "application/pdf".to_string(),
            "知识库-v4.pdf".to_string(),
        )
        .unwrap();

        assert_eq!(mime_type, "application/pdf");
        assert_eq!(source_name, "知识库-v4.pdf");
    }

    #[test]
    fn parser_input_accepts_markdown_detected_as_plain_text() {
        let (mime_type, source_name) = normalise_parser_input(
            b"# Title\n\n| A | B |\n| - | - |\n| 1 | 2 |\n",
            "text/markdown".to_string(),
            "国际化.md".to_string(),
        )
        .unwrap();

        assert_eq!(mime_type, "text/markdown");
        assert_eq!(source_name, "国际化.md");
    }

    #[test]
    fn document_with_parsed_text_and_file_reference_still_uses_file_source() {
        let mut doc = kb_document_model();
        doc.full_text = Some("# parsed markdown".to_string());
        doc.file_reference_id = Some(Uuid::now_v7());

        assert!(document_has_file_source(&doc));
    }

    #[test]
    fn indexing_error_message_keeps_custom_description() {
        let err =
            crate::modules::knowledge_base::errors::KnowledgeBaseError::EmbeddingError(
                "embedding request timed out".to_string(),
            )
            .to_loco_error();

        assert_eq!(
            indexing_error_message(&err),
            "嵌入生成失败: embedding request timed out"
        );
    }

    fn file_model(name: &str, mime_type: &str) -> files::Model {
        let now = chrono::Utc::now().fixed_offset();
        files::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            name: name.to_string(),
            mime_type: mime_type.to_string(),
            size: 1,
            content_hash: "hash".to_string(),
            content_hash_algo: "b3".to_string(),
            content_hash_fast: None,
            storage_backend: "minio".to_string(),
            bucket: "bucket".to_string(),
            storage_key: "key".to_string(),
            multipart_upload_id: None,
            status: "ACTIVE".to_string(),
            status_reason: None,
            deleted_at: None,
            purge_at: None,
            deleted_by: None,
            uploaded_by: Uuid::now_v7(),
            created_at: now,
            updated_at: now,
            created_by: Uuid::now_v7(),
            updated_by: Uuid::now_v7(),
        }
    }

    fn file_reference_model(mime_type: Option<&str>) -> file_references::Model {
        file_references::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            file_id: Uuid::now_v7(),
            resource_type: "knowledge_base:document".to_string(),
            resource_id: Uuid::now_v7().to_string(),
            field_name: String::new(),
            display_name: Some("uploaded-name.pdf".to_string()),
            mime_type: mime_type.map(str::to_string),
            created_by: Uuid::now_v7(),
            created_at: chrono::Utc::now().fixed_offset(),
            deleted_at: None,
        }
    }

    fn kb_document_model() -> kb_documents::Model {
        let now = chrono::Utc::now().naive_utc();
        kb_documents::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            title: "doc".to_string(),
            description: None,
            library_id: None,
            folder_id: None,
            source_type: "application/pdf".to_string(),
            file_id: None,
            file_reference_id: None,
            full_text: None,
            status: "ready".to_string(),
            scope: "tenant".to_string(),
            chunk_count: 0,
            total_tokens: 0,
            metadata: None,
            error_message: None,
            deleted_at: None,
            deleted_by: None,
            created_by: Uuid::now_v7(),
            created_at: now,
            updated_at: now,
        }
    }
}
