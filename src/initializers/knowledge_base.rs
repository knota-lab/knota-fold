use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use loco_rs::{
    app::{AppContext, Initializer},
    Error, Result,
};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::config::{AppSettings, ConfigExt};
use crate::modules::knowledge_base::providers::parser::markdown_parser::MarkdownDirectParser;
use crate::modules::knowledge_base::providers::parser::mineru_parser::MineruParser;
use crate::modules::knowledge_base::providers::parser::plain_text_parser::PlainTextParser;
use crate::modules::knowledge_base::providers::search::qdrant::QdrantSearchProvider;
use crate::modules::knowledge_base::providers::search::SearchProvider;
use crate::modules::knowledge_base::providers::{
    create_rig_clients, DocumentParser, SharedEmbeddingClient, SharedQaClient,
};
use crate::modules::knowledge_base::service::memory_service;
use crate::modules::knowledge_base::service::tools::tool_result_broker::{
    in_process::InProcessBroker, ToolResultBroker,
};

pub type SharedSearchProvider = Arc<dyn SearchProvider>;
pub type SharedParserChain = Arc<Vec<Box<dyn DocumentParser>>>;

/// Per-session concurrency lock map. Serialises concurrent `process_qa_v3_stream`
/// calls for the same session so that only one request runs at a time.
pub type SessionLockMap = Arc<Mutex<HashMap<Uuid, Arc<Mutex<()>>>>>;

/// Tracks in-progress compaction tasks to prevent duplicate runs for the same session.
/// Key: `session_id`, Value: `true` while a compaction task is running.
pub type CompactionGuard = Arc<DashMap<Uuid, bool>>;

/// Wrapper holding the raw Qdrant client and collection name for the `chat_memory` collection.
/// The `memory_service` uses this directly because its payload schema differs from `kb_chunks`.
pub struct ChatMemoryStore {
    pub client: qdrant_client::Qdrant,
    pub collection_name: String,
}
pub type SharedMemoryStore = Arc<ChatMemoryStore>;

pub struct KnowledgeBaseInitializer;

#[async_trait]
impl Initializer for KnowledgeBaseInitializer {
    fn name(&self) -> String {
        "knowledge_base".to_string()
    }

    async fn before_run(&self, ctx: &AppContext) -> Result<()> {
        let settings: AppSettings = ctx
            .config
            .typed_settings()
            .map_err(|e| Error::Message(format!("invalid settings: {e}")))?
            .ok_or_else(|| Error::Message("settings missing".into()))?;

        let kb_config = match &settings.knowledge_base {
            Some(cfg) if cfg.enabled => cfg,
            _ => {
                tracing::info!("Knowledge base module disabled");
                return Ok(());
            }
        };

        // Create providers — parser chain ordered by specificity
        let mut parser_chain: Vec<Box<dyn DocumentParser>> =
            vec![Box::new(PlainTextParser), Box::new(MarkdownDirectParser)];
        if kb_config.parser.mineru.enabled {
            parser_chain.push(Box::new(
                MineruParser::new(kb_config.parser.mineru.clone()).map_err(|e| {
                    Error::Message(format!("MinerU parser init failed: {e}"))
                })?,
            ));
        }
        let (embedding_client, qa_client) = create_rig_clients(kb_config);
        let search_provider = QdrantSearchProvider::new(
            &kb_config.qdrant.url,
            kb_config.qdrant.api_key.as_deref(),
            kb_config.embedding.dimension,
            &kb_config.qdrant.collection_name,
        )
        .await
        .map_err(|e| Error::Message(format!("Qdrant init failed: {e}")))?;

        // Create chat memory store — raw Qdrant client pointing to chat_memory collection
        let chat_client = qdrant_client::Qdrant::from_url(&kb_config.qdrant.url)
            .api_key(kb_config.qdrant.api_key.as_deref())
            .build()
            .map_err(|e| Error::Message(format!("Qdrant client build failed: {e}")))?;

        memory_service::ensure_collection(
            &chat_client,
            &kb_config.qdrant.chat_collection_name,
            kb_config.embedding.dimension,
        )
        .await
        .map_err(|e| {
            Error::Message(format!("chat_memory collection init failed: {e}"))
        })?;

        // Inject into shared_store
        ctx.shared_store
            .insert::<SharedParserChain>(Arc::new(parser_chain));
        ctx.shared_store
            .insert::<SharedEmbeddingClient>(embedding_client);
        ctx.shared_store.insert::<SharedQaClient>(qa_client);
        ctx.shared_store
            .insert::<SharedSearchProvider>(Arc::new(search_provider));
        ctx.shared_store
            .insert::<SharedMemoryStore>(Arc::new(ChatMemoryStore {
                client: chat_client,
                collection_name: kb_config.qdrant.chat_collection_name.clone(),
            }));
        ctx.shared_store
            .insert::<SessionLockMap>(Arc::new(Mutex::new(HashMap::new())));
        ctx.shared_store
            .insert::<CompactionGuard>(Arc::new(DashMap::new()));

        // Register tool result broker for frontend tool execution
        let broker: Arc<dyn ToolResultBroker> = Arc::new(InProcessBroker::new());
        ctx.shared_store.insert::<Arc<dyn ToolResultBroker>>(broker);

        tracing::info!(
            url = %kb_config.qdrant.url,
            collection = %kb_config.qdrant.collection_name,
            chat_collection = %kb_config.qdrant.chat_collection_name,
            "Knowledge base module initialized (rig-core + Qdrant)"
        );

        Ok(())
    }
}
