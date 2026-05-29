use crate::config::KnowledgeBaseConfig;

// Newtype wrappers so shared_store (TypeId-based) can distinguish them.
// Without these, both clients are the same `openai::Client` type and the
// second insert() overwrites the first.

#[derive(Clone)]
pub struct EmbeddingClient(pub rig::providers::openai::Client);

// Both Ollama and DeepSeek are OpenAI-compatible at /chat/completions.
// deepseek::Client uses the standard chat completions protocol (NOT /responses),
// so it works with any OpenAI-compatible provider including Ollama.
#[derive(Clone)]
pub struct QaClient(pub rig::providers::deepseek::Client);

pub type SharedEmbeddingClient = EmbeddingClient;
pub type SharedQaClient = QaClient;

/// Create embedding + QA clients from config.
#[tracing::instrument(skip_all)]
pub fn create_rig_clients(
    config: &KnowledgeBaseConfig,
) -> (SharedEmbeddingClient, SharedQaClient) {
    tracing::info!(
        base_url = %config.embedding.base_url,
        model = %config.embedding.model,
        "Creating embedding client"
    );
    let embedding_client = rig::providers::openai::Client::builder()
        .api_key(&config.embedding.api_key)
        .base_url(&config.embedding.base_url)
        .build()
        .expect("failed to create embedding client");

    let qa_api_key = config
        .qa
        .api_key
        .as_deref()
        .unwrap_or(&config.embedding.api_key);
    let qa_base_url = config
        .qa
        .base_url
        .as_deref()
        .unwrap_or(&config.embedding.base_url);

    tracing::info!(
        base_url = %qa_base_url,
        model = %config.qa.model,
        provider = %config.qa.provider,
        "Creating QA client (chat completions API)"
    );
    let qa_client = rig::providers::deepseek::Client::builder()
        .api_key(qa_api_key)
        .base_url(qa_base_url)
        .build()
        .expect("failed to create QA client");

    (EmbeddingClient(embedding_client), QaClient(qa_client))
}
