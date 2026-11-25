use std::time::Duration;

use rig::providers::openai::wire::DEEPSEEK;
use rig::providers::openai::{OpenAI, OpenAIConfig};

use crate::config::KnowledgeBaseConfig;

// Newtype wrappers so shared_store (TypeId-based) can distinguish them.
// Without these, both clients are the same `OpenAI` type and the
// second insert() overwrites the first.

#[derive(Clone)]
pub struct EmbeddingClient(pub OpenAI);

// Both Ollama and DeepSeek are OpenAI-compatible at /chat/completions.
// The DeepSeek dialect uses the standard chat completions protocol (NOT
// /responses), so it works with any OpenAI-compatible provider including
// Ollama. The base URL is overridden per deployment.
#[derive(Clone)]
pub struct QaClient(pub OpenAI);

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
        timeout_secs = config.embedding.timeout_secs,
        "Creating embedding client"
    );

    // rig 0.47 moved the bundled transport into `rig-reqwest`: build a
    // reqwest 0.13 client and wrap it as the rig HTTP client.
    let embedding_http = rig::rig_reqwest::reqwest::Client::builder()
        .timeout(Duration::from_secs(config.embedding.timeout_secs))
        .build()
        .expect("failed to create embedding HTTP client");
    let embedding_client = OpenAIConfig::new(config.embedding.api_key.clone())
        .with_base_url(config.embedding.base_url.clone())
        .connect(rig::http_client::ReqwestClient::from(embedding_http));

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
    let qa_client = OpenAIConfig::with_key(&DEEPSEEK, qa_api_key.to_owned())
        .with_base_url(qa_base_url.to_owned())
        .client();

    (EmbeddingClient(embedding_client), QaClient(qa_client))
}
