use std::collections::HashMap;

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
use crate::models::_entities::{files, kb_documents};
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::models::{
    document_lines as dl_models, kb_chunks as kc_models,
};
use crate::modules::knowledge_base::providers::parser::ParsedAsset;
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
    let s3_client = ctx.shared_store.get::<SharedS3Client>();
    let s3_config = ctx.shared_store.get::<SharedS3Config>();

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

    // 4-11. Run pipeline; mark as error on failure
    let pipeline_result = async {
        let input = load_pipeline_input(ctx, db, &doc).await?;
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
            },
        )
        .await
    }
    .await;

    if let Err(e) = pipeline_result {
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
    s3_client: Option<&'a SharedS3Client>,
    s3_bucket: Option<&'a str>,
    scope: &'a str,
    created_by: Uuid,
    library_id: Option<Uuid>,
    folder_id: Option<Uuid>,
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
    let ParsedMarkdown {
        preview_markdown,
        index_markdown,
        asset_metadata,
    } = prepare_parsed_markdown(p, &parsed.markdown, &parsed.assets).await?;
    document_service::set_parsed_content(
        db,
        p.document_id,
        &preview_markdown,
        asset_metadata,
    )
    .await?;

    // 5. Chunk the markdown
    let chunks = chunking_service::chunk_markdown(
        &index_markdown,
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
    let raw_lines = line_splitting_service::split_lines(&index_markdown);
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
        .to_err()
    })?;
    let bucket = p.s3_bucket.ok_or_else(|| {
        KnowledgeBaseError::ConfigError(
            "S3 config is required to persist parsed document assets".into(),
        )
        .to_err()
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
                .to_err()
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
        extension_for_mime, normalize_asset_target, rewrite_markdown_image_targets,
        strip_markdown_images,
    };

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
}
