use serde::{Deserialize, Serialize};

fn deserialize_page_contexts<'de, D>(
    deserializer: D,
) -> Result<Vec<PageContextMinimal>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum PageContextOrArray {
        Single(PageContextMinimal),
        Array(Vec<PageContextMinimal>),
    }

    match PageContextOrArray::deserialize(deserializer)? {
        PageContextOrArray::Single(ctx) => Ok(vec![ctx]),
        PageContextOrArray::Array(arr) => Ok(arr),
    }
}

/// QA request from the client.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QaRequest {
    pub instruction: String,
    #[serde(default)]
    pub material: MaterialInput,
    /// Session ID — first request omits this (server creates a new session).
    /// Subsequent requests within the same conversation pass the `session_id`.
    pub session_id: Option<uuid::Uuid>,
    /// Frontend-generated tool schemas (page tools).
    #[serde(default)]
    pub page_tools: Vec<PageToolDefinition>,
    /// Page contexts for multi-page conversation.
    /// Accepts both a single object (legacy) and an array (new).
    #[serde(default, deserialize_with = "deserialize_page_contexts")]
    pub page_context: Vec<PageContextMinimal>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialInput {
    #[serde(default = "default_use_knowledge_base")]
    pub use_knowledge_base: bool,
    pub inline: Option<String>,
    pub library_id: Option<uuid::Uuid>,
    pub folder_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub include_subfolders: bool,
    #[serde(default)]
    pub file_ids: Vec<uuid::Uuid>,
    #[serde(default)]
    pub document_ids: Vec<uuid::Uuid>,
}

const fn default_use_knowledge_base() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub document_id: uuid::Uuid,
    pub chunk_id: Option<uuid::Uuid>,
    pub content: String,
    pub score: f64,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub total_tokens: i32,
}

// ---------------------------------------------------------------------------
// Frontend tool schema types
// ---------------------------------------------------------------------------

/// A tool schema definition sent by the frontend for page-level tools.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Minimal page context injected into the system prompt (~30 tokens).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageContextMinimal {
    pub route: String,
    pub title: String,
    pub intent: String,
    #[serde(default)]
    pub active: bool,
}
