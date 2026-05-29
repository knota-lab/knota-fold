use async_trait::async_trait;
use uuid::Uuid;

use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::providers::parser::{
    DocumentMetadata, DocumentParser, ParsedDocument,
};

pub struct MarkdownDirectParser;

const SUPPORTED_TYPES: &[&str] = &["text/markdown"];

#[async_trait]
impl DocumentParser for MarkdownDirectParser {
    fn supported_mime_types(&self) -> &[&str] {
        SUPPORTED_TYPES
    }

    async fn parse(
        &self,
        content: &[u8],
        _mime_type: &str,
        source_name: &str,
    ) -> Result<ParsedDocument, KnowledgeBaseError> {
        let markdown = String::from_utf8(content.to_vec()).map_err(|e| {
            KnowledgeBaseError::ParsingError(format!("UTF-8 decode failed: {e}"))
        })?;

        let char_count = markdown.len();
        let estimated_tokens = char_count / 2;

        Ok(ParsedDocument {
            id: Uuid::now_v7(),
            source_name: source_name.to_string(),
            mime_type: "text/markdown".to_string(),
            markdown,
            metadata: DocumentMetadata {
                page_count: None,
                char_count,
                estimated_tokens,
                warnings: Vec::new(),
            },
        })
    }

    fn name(&self) -> &str {
        "markdown_direct"
    }
}
