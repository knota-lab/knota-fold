use loco_rs::prelude::*;
use sea_orm::{ActiveValue, DatabaseConnection};
use serde::{Deserialize, Serialize};
use tracing::Instrument;
use uuid::Uuid;

use crate::config::{AppSettings, ConfigExt};
use crate::initializers::knowledge_base::{SharedParserChain, SharedSearchProvider};
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

    // 3. Set status to 'indexing' and update full_text
    let full_text = doc.full_text.clone().ok_or_else(|| {
        KnowledgeBaseError::ParsingError("document has no full_text content".into())
            .to_err()
    })?;
    document_service::set_full_text(db, document_id, &full_text).await?;

    // 4-11. Run pipeline; mark as error on failure
    if let Err(e) = execute_pipeline(
        db,
        &PipelineParams {
            parser_chain: &parser_chain,
            embedding_client: &embedding_client,
            search_provider: &search_provider,
            document_id,
            tenant_id,
            full_text: &full_text,
            source_type: &doc.source_type,
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

/// Parameters for [`execute_pipeline`].
struct PipelineParams<'a> {
    parser_chain: &'a SharedParserChain,
    embedding_client: &'a SharedEmbeddingClient,
    search_provider: &'a SharedSearchProvider,
    document_id: Uuid,
    tenant_id: Uuid,
    full_text: &'a str,
    source_type: &'a str,
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
        .find(|pr| pr.supported_mime_types().contains(&p.source_type))
        .ok_or_else(|| {
            KnowledgeBaseError::UnsupportedFormat(format!(
                "no parser found for content type '{}'",
                p.source_type
            ))
            .to_err()
        })?;
    let parsed = parser
        .parse(p.full_text.as_bytes(), p.source_type, "document")
        .await
        .map_err(|e| {
            KnowledgeBaseError::ParsingError(format!("parsing failed: {e}")).to_err()
        })?;
    let markdown = &parsed.markdown;

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
