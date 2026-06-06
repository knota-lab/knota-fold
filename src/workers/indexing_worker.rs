use loco_rs::prelude::*;
use sea_orm::{ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tracing::Instrument;
use uuid::Uuid;

use crate::config::{AppSettings, ConfigExt};
use crate::initializers::knowledge_base::{SharedParserChain, SharedSearchProvider};
use crate::initializers::s3::SharedS3Client;
use crate::models::_entities::{files, kb_documents};
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::models::{
    document_lines as dl_models, kb_chunks as kc_models,
};
use crate::modules::knowledge_base::providers::search::ChunkPoint;
use crate::modules::knowledge_base::providers::SharedEmbeddingClient;
use crate::modules::knowledge_base::service::numeric::embedding_vec_f64_to_f32;
use crate::modules::knowledge_base::service::{
    chunking_service, document_service, line_splitting_service,
};
use rig::client::EmbeddingsClient;
use rig::embeddings::EmbeddingModel;

const MAX_INDEX_FILE_BYTES: i64 = 50 * 1024 * 1024;
const LEGACY_TEXT_SOURCE_TYPES: &[&str] = &["kb_upload", "chat_upload", "api", "sync"];

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

        async move { run_indexing_pipeline(&ctx, document_id, tenant_id).await }
            .instrument(span)
            .await
    }
}

async fn run_indexing_pipeline(
    ctx: &AppContext,
    document_id: Uuid,
    tenant_id: Uuid,
) -> Result<()> {
    let db = &ctx.db;

    // 1. Get dependencies from shared_store
    let parser_chain = ctx.shared_store.get::<SharedParserChain>().ok_or_else(|| {
        KnowledgeBaseError::ConfigError("Parser chain not initialized".into()).to_err()
    })?;
    let embedding_client =
        ctx.shared_store
            .get::<SharedEmbeddingClient>()
            .ok_or_else(|| {
                KnowledgeBaseError::ConfigError("Embedding client not initialized".into())
                    .to_err()
            })?;
    let search_provider =
        ctx.shared_store
            .get::<SharedSearchProvider>()
            .ok_or_else(|| {
                KnowledgeBaseError::ConfigError("Search provider not initialized".into())
                    .to_err()
            })?;

    // Read KB config
    let settings: AppSettings = ctx
        .config
        .typed_settings()
        .map_err(|e| {
            KnowledgeBaseError::ConfigError(format!("invalid settings: {e}")).to_err()
        })?
        .ok_or_else(|| {
            KnowledgeBaseError::ConfigError("settings missing".into()).to_err()
        })?;
    let kb_config = settings.knowledge_base.as_ref().ok_or_else(|| {
        KnowledgeBaseError::ConfigError("knowledge base config missing".into()).to_err()
    })?;
    let config = &kb_config.chunking;

    // 2. Get document from DB
    let doc = document_service::get_document(db, document_id, tenant_id).await?;
    document_service::start_indexing(db, document_id, tenant_id).await?;

    let input = load_pipeline_input(ctx, db, &doc).await?;

    // 4-11. Run pipeline; mark as error on failure
    if let Err(e) = execute_pipeline(
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
            scope: &doc.scope,
            created_by: doc.created_by,
            config,
            embedding_model_name: &kb_config.embedding.model,
        },
    )
    .await
    {
        let error_msg = format!("{e}");
        tracing::error!(
            document_id = %document_id,
            error = %error_msg,
            "indexing pipeline failed"
        );
        let _ = document_service::update_status(
            db,
            document_id,
            tenant_id,
            "error",
            Some(&error_msg),
        )
        .await;
        return Err(e);
    }

    Ok(())
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
    if let Some(full_text) = &doc.full_text {
        return Ok(PipelineInput {
            bytes: full_text.as_bytes().to_vec(),
            mime_type: normalise_source_mime(&doc.source_type),
            source_name: doc.title.clone(),
        });
    }

    let file_id = doc.file_id.ok_or_else(|| {
        KnowledgeBaseError::ParsingError(
            "document has neither content nor file_id".into(),
        )
        .to_err()
    })?;

    let file = files::Entity::find_by_id(file_id)
        .filter(files::Column::TenantId.eq(doc.tenant_id))
        .filter(files::Column::Status.eq("ACTIVE"))
        .filter(files::Column::DeletedAt.is_null())
        .one(db)
        .await
        .map_err(|e| KnowledgeBaseError::ProviderError(e.to_string()).to_err())?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())?;

    if file.size > MAX_INDEX_FILE_BYTES {
        return Err(KnowledgeBaseError::ParsingError(format!(
            "file is too large to index synchronously: {} bytes > {} bytes",
            file.size, MAX_INDEX_FILE_BYTES
        ))
        .to_err());
    }

    let s3_client = ctx.shared_store.get::<SharedS3Client>().ok_or_else(|| {
        KnowledgeBaseError::ConfigError("S3 client not initialized".into()).to_err()
    })?;
    let bytes = read_file_bytes(&s3_client, &file).await?;

    Ok(PipelineInput {
        bytes,
        mime_type: file.mime_type,
        source_name: file.name,
    })
}

fn normalise_source_mime(source_type: &str) -> String {
    if LEGACY_TEXT_SOURCE_TYPES.contains(&source_type) {
        "text/plain".to_string()
    } else {
        source_type.to_string()
    }
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
            .to_err()
        })?;

    let capacity = usize::try_from(file.size.max(0)).unwrap_or_default();
    let mut bytes = Vec::with_capacity(capacity);
    let mut reader = response.body.into_async_read();
    reader.read_to_end(&mut bytes).await.map_err(|e| {
        KnowledgeBaseError::ProviderError(format!(
            "failed to read file object from storage: {e}"
        ))
        .to_err()
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
    scope: &'a str,
    created_by: Uuid,
    config: &'a crate::config::ChunkingConfig,
    embedding_model_name: &'a str,
}

fn build_chunk_points_and_models(
    chunks: &[crate::modules::knowledge_base::service::chunking_service::RawChunk],
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
    // 4. Parse document — select parser by MIME type from source_type
    let parser = p
        .parser_chain
        .iter()
        .find(|pr| pr.supported_mime_types().contains(&p.mime_type))
        .ok_or_else(|| {
            KnowledgeBaseError::UnsupportedFormat(format!(
                "no parser found for content type '{}'",
                p.mime_type
            ))
            .to_err()
        })?;
    let parsed = parser
        .parse(p.input_bytes, p.mime_type, p.source_name)
        .await
        .map_err(|e| {
            KnowledgeBaseError::ParsingError(format!("parsing failed: {e}")).to_err()
        })?;
    let markdown = &parsed.markdown;
    document_service::set_full_text(db, p.document_id, markdown).await?;

    // 5. Chunk the markdown
    let chunks = chunking_service::chunk_markdown(
        markdown,
        p.config.max_chunk_tokens,
        p.config.min_chunk_tokens,
        p.config.split_by_heading,
        p.config.min_heading_level,
        p.config.max_heading_level,
    );
    if chunks.is_empty() {
        return Err(KnowledgeBaseError::IndexingError(
            "chunking produced no chunks".into(),
        )
        .to_err());
    }

    // 6. Split lines and insert
    let raw_lines = line_splitting_service::split_lines(markdown);
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
    document_service::insert_lines(db, line_models).await?;

    // 7. Generate embeddings
    let model = p.embedding_client.0.embedding_model(p.embedding_model_name);
    let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
    let embeddings: Vec<rig::embeddings::Embedding> =
        model.embed_texts(texts).await.map_err(|e| {
            KnowledgeBaseError::EmbeddingError(format!("embedding failed: {e}")).to_err()
        })?;

    if embeddings.len() != chunks.len() {
        return Err(KnowledgeBaseError::EmbeddingError(format!(
            "embedding count mismatch: got {} embeddings for {} chunks",
            embeddings.len(),
            chunks.len()
        ))
        .to_err());
    }

    // 8. Build ChunkPoints + kb_chunks ActiveModels
    let (chunk_points, chunk_models, total_tokens) =
        build_chunk_points_and_models(&chunks, &embeddings, p);

    // 9. Write chunks to PG
    document_service::insert_chunks(db, chunk_models).await?;

    // 10. Write vectors to Qdrant
    p.search_provider
        .upsert_chunks(&chunk_points, p.tenant_id)
        .await
        .map_err(|e| {
            KnowledgeBaseError::IndexingError(format!("Qdrant upsert failed: {e}"))
                .to_err()
        })?;

    // 11. Mark document as ready
    document_service::mark_ready(
        db,
        p.document_id,
        i32::try_from(chunks.len()).unwrap_or(i32::MAX),
        total_tokens,
    )
    .await?;

    tracing::info!(
        document_id = %p.document_id,
        chunk_count = chunks.len(),
        total_tokens = total_tokens,
        "indexing pipeline completed"
    );

    Ok(())
}
