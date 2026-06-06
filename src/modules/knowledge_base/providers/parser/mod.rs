pub mod markdown_parser;
pub mod mineru_parser;
pub mod plain_text_parser;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::knowledge_base::errors::KnowledgeBaseError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedDocument {
    pub id: Uuid,
    pub source_name: String,
    pub mime_type: String,
    pub markdown: String,
    #[serde(default)]
    pub assets: Vec<ParsedAsset>,
    pub metadata: DocumentMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedAsset {
    pub name: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub page_count: Option<usize>,
    pub char_count: usize,
    pub estimated_tokens: usize,
    pub warnings: Vec<String>,
}

#[async_trait]
pub trait DocumentParser: Send + Sync {
    fn supported_mime_types(&self) -> &[&str];
    async fn parse(
        &self,
        content: &[u8],
        mime_type: &str,
        source_name: &str,
    ) -> Result<ParsedDocument, KnowledgeBaseError>;
    fn name(&self) -> &str;
}
