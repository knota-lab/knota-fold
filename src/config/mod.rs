//! Strongly-typed application settings.
//!
//! This module provides a single `AppSettings` struct that mirrors the YAML
//! `settings:` section.  Use [`ConfigExt::typed_settings`] to obtain it from
//! any `loco_rs::config::Config` reference — parsing happens once per call,
//! but callers are encouraged to cache the result (e.g. `OnceLock`).
//!
//! # Adding a new setting
//!
//! 1. Add an `Option<YourConfig>` field to [`AppSettings`].
//! 2. Define `YourConfig` struct right here (or re-export from its module).
//! 3. Update the YAML `settings:` block in `config/development.yaml`.
//!
//! That's it — `serde_json::from_value` handles the rest.

use serde::Deserialize;

// ── Re-exports from existing modules ──────────────────────────────

pub use crate::app_logs::config::AppLogsConfig;
pub use crate::initializers::s3::S3Config;
pub use crate::services::captcha_service::CaptchaConfig;

// ── Task scheduler ────────────────────────────────────────────────

/// Parsed `settings.task_scheduler` / `settings.worker_scheduler`.
///
/// Both keys are accepted for backward compatibility; `worker_scheduler`
/// takes precedence when both are present.
#[derive(Debug, Clone, Deserialize)]
pub struct SchedulerConfig {
    #[serde(default = "default_true_val")]
    pub enabled: bool,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_per_tenant: i32,
    #[serde(default = "default_retention_days")]
    pub execution_log_retention_days: i32,
    #[serde(default = "default_output_max_bytes")]
    pub output_max_bytes: i64,
}

fn default_true_val() -> bool {
    true
}
fn default_max_concurrent() -> i32 {
    3
}
fn default_retention_days() -> i32 {
    90
}
fn default_output_max_bytes() -> i64 {
    65_536
}

// ── API Key ──────────────────────────────────────────────────────

/// Parsed `settings.apiKey`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyConfig {
    /// Environment prefix for generated keys, e.g. "sk_live_" or "sk_test_".
    #[serde(default = "default_env_prefix")]
    pub env_prefix: String,
    /// Number of random bytes for key generation (default 32 → 256-bit).
    #[serde(default = "default_secret_bytes")]
    pub secret_bytes: usize,
    /// Default TTL in hours for exchange tokens. `None` = never expire.
    #[serde(default)]
    pub default_exchange_ttl_hours: Option<u64>,
    /// Maximum active keys per tenant.
    #[serde(default = "default_max_keys_per_tenant")]
    pub max_keys_per_tenant: i32,
    /// Maximum active exchange tokens per tenant.
    #[serde(default = "default_max_exchange_tokens_per_tenant")]
    pub max_exchange_tokens_per_tenant: i32,
    /// Base URL for the exchange page (frontend). Used to construct exchange URLs.
    /// e.g. "https://admin.example.com/api-keys/exchange"
    #[serde(default)]
    pub exchange_base_url: Option<String>,
}

impl Default for ApiKeyConfig {
    fn default() -> Self {
        Self {
            env_prefix: default_env_prefix(),
            secret_bytes: default_secret_bytes(),
            default_exchange_ttl_hours: None,
            max_keys_per_tenant: default_max_keys_per_tenant(),
            max_exchange_tokens_per_tenant: default_max_exchange_tokens_per_tenant(),
            exchange_base_url: None,
        }
    }
}

fn default_env_prefix() -> String {
    "sk_live_".to_string()
}
fn default_secret_bytes() -> usize {
    32
}
fn default_max_keys_per_tenant() -> i32 {
    20
}
fn default_max_exchange_tokens_per_tenant() -> i32 {
    50
}

// ── Knowledge Base ────────────────────────────────────────────────

/// Parsed `settings.knowledgeBase`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBaseConfig {
    #[serde(default)]
    pub enabled: bool,

    pub embedding: EmbeddingConfig,

    pub qdrant: QdrantConfig,

    #[serde(default)]
    pub chunking: ChunkingConfig,

    #[serde(default)]
    pub search: SearchConfig,

    #[serde(default)]
    pub qa: QaConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingConfig {
    /// Provider name. Phase 1 ignores this — always uses openai::Client.
    #[serde(default = "default_embedding_provider")]
    pub provider: String,

    pub model: String,

    pub api_key: String,

    #[serde(default = "default_embedding_base_url")]
    pub base_url: String,

    #[serde(default = "default_embedding_dimension")]
    pub dimension: usize,

    #[serde(default = "default_cache_size")]
    pub cache_size: usize,
}

fn default_embedding_provider() -> String {
    "openai".to_string()
}
fn default_embedding_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}
fn default_embedding_dimension() -> usize {
    1536
}
fn default_cache_size() -> usize {
    10000
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QdrantConfig {
    /// gRPC URL (recommended over REST for performance).
    #[serde(default = "default_qdrant_url")]
    pub url: String,

    #[serde(default)]
    pub api_key: Option<String>,

    #[serde(default = "default_qdrant_collection")]
    pub collection_name: String,

    /// Collection for chat memory vector storage (§19.7).
    #[serde(default = "default_chat_collection")]
    pub chat_collection_name: String,
}

fn default_qdrant_url() -> String {
    "http://localhost:6334".to_string()
}
fn default_qdrant_collection() -> String {
    "kb_chunks".to_string()
}
fn default_chat_collection() -> String {
    "chat_memory".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkingConfig {
    #[serde(default = "default_max_chunk_tokens")]
    pub max_chunk_tokens: i32,

    #[serde(default = "default_min_chunk_tokens")]
    pub min_chunk_tokens: i32,

    #[serde(default = "default_overlap_sentences")]
    pub overlap_sentences: i32,

    #[serde(default = "default_true_val")]
    pub split_by_heading: bool,

    #[serde(default = "default_min_heading_level")]
    pub min_heading_level: i32,

    #[serde(default = "default_max_heading_level")]
    pub max_heading_level: i32,
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            max_chunk_tokens: default_max_chunk_tokens(),
            min_chunk_tokens: default_min_chunk_tokens(),
            overlap_sentences: default_overlap_sentences(),
            split_by_heading: true,
            min_heading_level: default_min_heading_level(),
            max_heading_level: default_max_heading_level(),
        }
    }
}

fn default_max_chunk_tokens() -> i32 {
    800
}
fn default_min_chunk_tokens() -> i32 {
    100
}
fn default_overlap_sentences() -> i32 {
    2
}
fn default_min_heading_level() -> i32 {
    1
}
fn default_max_heading_level() -> i32 {
    4
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchConfig {
    #[serde(default = "default_search_limit")]
    pub default_limit: i32,

    #[serde(default = "default_rrf_k")]
    pub rrf_k: i32,

    #[serde(default = "default_pre_fusion_limit")]
    pub pre_fusion_limit: i32,

    #[serde(default)]
    pub min_score: f64,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_limit: default_search_limit(),
            rrf_k: default_rrf_k(),
            pre_fusion_limit: default_pre_fusion_limit(),
            min_score: 0.0,
        }
    }
}

fn default_search_limit() -> i32 {
    10
}
fn default_rrf_k() -> i32 {
    60
}
fn default_pre_fusion_limit() -> i32 {
    50
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QaConfig {
    /// "auto" | "strong" | "weak"
    #[serde(default = "default_qa_mode")]
    pub mode: String,

    /// Model name. "auto" resolves based on mode.
    #[serde(default = "default_qa_model")]
    pub model: String,

    /// LLM provider: "ollama" (default) or "deepseek".
    /// Controls provider-specific params (e.g. num_ctx for Ollama).
    #[serde(default = "default_qa_provider")]
    pub provider: String,

    /// Optional API key override for QA model.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Optional base URL override for QA model.
    #[serde(default)]
    pub base_url: Option<String>,

    #[serde(default = "default_qa_temperature")]
    pub temperature: f64,

    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: i32,

    #[serde(default = "default_response_reserve_tokens")]
    pub response_reserve_tokens: i32,

    /// Summary model. "auto" = same as model.
    #[serde(default = "default_qa_model")]
    pub summary_model: String,

    /// Phase 4: compaction threshold (message count safety net)
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: usize,

    /// Phase 4: fixed recent window size for compaction
    #[serde(default = "default_compaction_recent_turns")]
    pub compaction_recent_turns: usize,

    /// Phase 4: token reserve for compaction trigger
    #[serde(default = "default_compaction_reserve_tokens")]
    pub compaction_reserve_tokens: usize,

    /// Phase 4: semantic recall strategy
    #[serde(default = "default_history_strategy")]
    pub history_strategy: String,

    /// Phase 4: semantic recall top-K
    #[serde(default = "default_semantic_top_k")]
    pub semantic_top_k: usize,

    /// Phase 4: inline material size limit
    #[serde(default = "default_max_inline_chars")]
    pub max_inline_chars: usize,

    /// Intent recognition config for strong mode.
    #[serde(default)]
    pub intent: IntentConfig,
}

impl Default for QaConfig {
    fn default() -> Self {
        Self {
            mode: default_qa_mode(),
            model: default_qa_model(),
            provider: default_qa_provider(),
            api_key: None,
            base_url: None,
            temperature: default_qa_temperature(),
            max_context_tokens: default_max_context_tokens(),
            response_reserve_tokens: default_response_reserve_tokens(),
            summary_model: default_qa_model(),
            compaction_threshold: default_compaction_threshold(),
            compaction_recent_turns: default_compaction_recent_turns(),
            compaction_reserve_tokens: default_compaction_reserve_tokens(),
            history_strategy: default_history_strategy(),
            semantic_top_k: default_semantic_top_k(),
            max_inline_chars: default_max_inline_chars(),
            intent: IntentConfig::default(),
        }
    }
}

fn default_qa_mode() -> String {
    "auto".to_string()
}
fn default_qa_provider() -> String {
    "ollama".to_string()
}
fn default_qa_model() -> String {
    "auto".to_string()
}
fn default_qa_temperature() -> f64 {
    0.2
}
fn default_max_context_tokens() -> i32 {
    100000
}
fn default_response_reserve_tokens() -> i32 {
    4096
}
fn default_compaction_threshold() -> usize {
    20
}
fn default_compaction_recent_turns() -> usize {
    12
}
fn default_compaction_reserve_tokens() -> usize {
    16384
}
fn default_history_strategy() -> String {
    "retrieve".to_string()
}
fn default_semantic_top_k() -> usize {
    10
}
fn default_max_inline_chars() -> usize {
    100000
}

// ── Intent Recognition ─────────────────────────────────────────────

/// Intent classification settings for strong-mode QA.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentConfig {
    #[serde(default = "default_true_val")]
    pub enabled: bool,
    #[serde(default = "default_intent_model")]
    pub model: String,
    /// Confidence threshold below which we fall back to weak mode.
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f64,
}

impl Default for IntentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: default_intent_model(),
            confidence_threshold: default_confidence_threshold(),
        }
    }
}

fn default_intent_model() -> String {
    "gpt-4o-mini".to_string()
}
fn default_confidence_threshold() -> f64 {
    0.6
}

// ── Root settings struct ──────────────────────────────────────────

/// Strongly-typed mirror of the YAML `settings:` block.
///
/// Every field is `Option<T>` so that individual sections can be omitted
/// without breaking the parse — the consumer decides whether a missing
/// section is an error.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub s3: Option<S3Config>,
    pub captcha: Option<CaptchaConfig>,
    #[serde(rename = "appLogs")]
    pub app_logs: Option<AppLogsConfig>,
    /// Primary key. `worker_scheduler` is the preferred name; if absent,
    /// falls back to `task_scheduler` for backward compatibility.
    #[serde(alias = "task_scheduler", alias = "worker_scheduler")]
    pub worker_scheduler: Option<SchedulerConfig>,
    #[serde(rename = "apiKey", default)]
    pub api_key: ApiKeyConfig,
    pub knowledge_base: Option<KnowledgeBaseConfig>,
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let errors = Vec::new();

        let loco_env =
            std::env::var("LOCO_ENV").unwrap_or_else(|_| "development".to_string());
        if self.api_key.env_prefix == "sk_live_"
            && loco_env.eq_ignore_ascii_case("development")
        {
            tracing::warn!(
                env_prefix = %self.api_key.env_prefix,
                "API key env_prefix uses live prefix in development; consider using a non-production prefix"
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Helper: max concurrent executions per tenant (with sensible default).
    pub fn max_concurrent_per_tenant(&self) -> i32 {
        self.worker_scheduler
            .as_ref()
            .and_then(|s| {
                let v = s.max_concurrent_per_tenant;
                (v > 0).then_some(v)
            })
            .unwrap_or(3)
    }

    /// Helper: output truncation ceiling in bytes.
    pub fn output_max_bytes(&self) -> usize {
        self.worker_scheduler
            .as_ref()
            .map(|s| s.output_max_bytes as usize)
            .filter(|&v| v > 0)
            .unwrap_or(65_536)
    }

    /// Helper: whether the scheduler tick should run.
    pub fn scheduler_enabled(&self) -> bool {
        self.worker_scheduler
            .as_ref()
            .map(|s| s.enabled)
            .unwrap_or(true)
    }
}

// ── ConfigExt trait ───────────────────────────────────────────────

/// Extension trait that adds typed settings parsing to `loco_rs::config::Config`.
pub trait ConfigExt {
    /// Parse the `settings` section into a strongly-typed [`AppSettings`].
    ///
    /// Returns `Ok(None)` when the `settings` key is absent (not an error —
    /// some environments may not need any settings). Returns `Err` when the
    /// section exists but cannot be deserialized (typo, wrong type, etc.).
    fn typed_settings(&self) -> Result<Option<AppSettings>, serde_json::Error>;
}

impl ConfigExt for loco_rs::config::Config {
    fn typed_settings(&self) -> Result<Option<AppSettings>, serde_json::Error> {
        self.settings
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
    }
}
