use async_trait::async_trait;
use uuid::Uuid;

use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::providers::parser::{
    DocumentMetadata, DocumentParser, ParsedDocument,
};

/// Plain-text parser that normalises raw text into a Markdown-compatible
/// representation suitable for downstream chunking.
///
/// Normalisations applied:
/// - UTF-8 BOM removal
/// - CRLF / CR → LF normalisation
/// - Trailing whitespace trim
/// - Multiple consecutive blank lines collapsed to two newlines
///   (preserves paragraph breaks for the heading-aware chunker)
pub struct PlainTextParser;

const SUPPORTED_TYPES: &[&str] = &["text/plain"];

#[async_trait]
impl DocumentParser for PlainTextParser {
    fn supported_mime_types(&self) -> &[&str] {
        SUPPORTED_TYPES
    }

    async fn parse(
        &self,
        content: &[u8],
        _mime_type: &str,
        source_name: &str,
    ) -> Result<ParsedDocument, KnowledgeBaseError> {
        // Decode UTF-8
        let raw = String::from_utf8(content.to_vec()).map_err(|e| {
            KnowledgeBaseError::ParsingError(format!(
                "UTF-8 decode failed for '{source_name}': {e}"
            ))
        })?;

        let normalised = normalise_plain_text(&raw);

        let char_count = normalised.len();
        let estimated_tokens = char_count / 2;

        Ok(ParsedDocument {
            id: Uuid::now_v7(),
            source_name: source_name.to_string(),
            mime_type: "text/plain".to_string(),
            markdown: normalised,
            assets: Vec::new(),
            metadata: DocumentMetadata {
                page_count: None,
                char_count,
                estimated_tokens,
                warnings: Vec::new(),
            },
        })
    }

    fn name(&self) -> &'static str {
        "plain_text"
    }
}

/// Strip BOM, normalise line-endings, collapse blank lines.
fn normalise_plain_text(raw: &str) -> String {
    // 1. Strip UTF-8 BOM if present
    let s = raw.strip_suffix('\u{FEFF}').unwrap_or(raw);
    let s = s.strip_prefix('\u{FEFF}').unwrap_or(s);

    // 2. Normalise line endings: CRLF / CR → LF
    let s = s.replace("\r\n", "\n").replace('\r', "\n");

    // 3. Collapse 3+ consecutive newlines into 2 (preserve paragraph breaks)
    let mut result = String::with_capacity(s.len());
    let mut blank_run = 0usize;
    for ch in s.chars() {
        if ch == '\n' {
            blank_run += 1;
            if blank_run <= 2 {
                result.push(ch);
            }
        } else {
            blank_run = 0;
            result.push(ch);
        }
    }

    // 4. Trim trailing whitespace
    let trimmed = result.trim_end();

    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_bom() {
        let input = "\u{FEFF}hello world";
        assert_eq!(normalise_plain_text(input), "hello world");
    }

    #[test]
    fn normalises_crlf() {
        let input = "line1\r\nline2\r\nline3";
        assert_eq!(normalise_plain_text(input), "line1\nline2\nline3");
    }

    #[test]
    fn normalises_bare_cr() {
        let input = "line1\rline2";
        assert_eq!(normalise_plain_text(input), "line1\nline2");
    }

    #[test]
    fn collapses_blank_lines() {
        let input = "para1\n\n\n\n\npara2";
        assert_eq!(normalise_plain_text(input), "para1\n\npara2");
    }

    #[test]
    fn trims_trailing_whitespace() {
        let input = "hello   \n\n";
        assert_eq!(normalise_plain_text(input), "hello");
    }

    #[test]
    fn preserves_paragraph_breaks() {
        let input = "para1\n\npara2";
        assert_eq!(normalise_plain_text(input), "para1\n\npara2");
    }
}
