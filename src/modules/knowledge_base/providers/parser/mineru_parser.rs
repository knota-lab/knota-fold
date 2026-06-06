use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine};
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use uuid::Uuid;

use crate::config::MineruParserConfig;
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::providers::parser::{
    DocumentMetadata, DocumentParser, ParsedAsset, ParsedDocument,
};

pub struct MineruParser {
    client: reqwest::Client,
    config: MineruParserConfig,
}

const SUPPORTED_TYPES: &[&str] = &[
    "application/pdf",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/bmp",
    "image/tiff",
];

#[derive(Debug, Deserialize)]
struct MineruFileResult {
    #[serde(default, alias = "md_content", alias = "markdown")]
    md_content: Option<String>,
    #[serde(default)]
    images: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct MineruResponse {
    #[serde(default)]
    results: HashMap<String, MineruFileResult>,
}

impl MineruParser {
    pub fn new(config: MineruParserConfig) -> Result<Self, KnowledgeBaseError> {
        let timeout = Duration::from_secs(config.timeout_secs);
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| KnowledgeBaseError::ConfigError(e.to_string()))?;
        Ok(Self { client, config })
    }

    fn endpoint(&self) -> String {
        format!("{}/file_parse", self.config.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl DocumentParser for MineruParser {
    fn supported_mime_types(&self) -> &[&str] {
        SUPPORTED_TYPES
    }

    async fn parse(
        &self,
        content: &[u8],
        mime_type: &str,
        source_name: &str,
    ) -> Result<ParsedDocument, KnowledgeBaseError> {
        if i64::try_from(content.len()).unwrap_or(i64::MAX) > self.config.max_file_bytes {
            return Err(KnowledgeBaseError::ParsingError(format!(
                "file is too large for MinerU: {} bytes > {} bytes",
                content.len(),
                self.config.max_file_bytes
            )));
        }

        let part = Part::bytes(content.to_vec())
            .file_name(source_name.to_string())
            .mime_str(mime_type)
            .map_err(|e| KnowledgeBaseError::ParsingError(e.to_string()))?;
        let form = Form::new()
            .part("files", part)
            .text("backend", self.config.backend.clone())
            .text("parse_method", self.config.parse_method.clone())
            .text("lang_list", self.config.lang.clone())
            .text("return_md", "true")
            .text("return_images", "true")
            .text("response_format_zip", "false");

        let response = self
            .client
            .post(self.endpoint())
            .multipart(form)
            .send()
            .await
            .map_err(|e| {
                KnowledgeBaseError::ParsingError(format!("MinerU request failed: {e}"))
            })?;

        let status = response.status();
        let text = response.text().await.map_err(|e| {
            KnowledgeBaseError::ParsingError(format!("MinerU response read failed: {e}"))
        })?;
        if !status.is_success() {
            return Err(KnowledgeBaseError::ParsingError(format!(
                "MinerU returned HTTP {status}: {text}"
            )));
        }

        let parsed: MineruResponse = serde_json::from_str(&text).map_err(|e| {
            KnowledgeBaseError::ParsingError(format!(
                "MinerU response JSON parse failed: {e}; body={text}"
            ))
        })?;
        let result = parsed
            .results
            .get(source_name)
            .or_else(|| parsed.results.values().next())
            .ok_or_else(|| {
                KnowledgeBaseError::ParsingError(
                    "MinerU response has no results".to_string(),
                )
            })?;
        let markdown = result.md_content.clone().ok_or_else(|| {
            KnowledgeBaseError::ParsingError(
                "MinerU response has no markdown".to_string(),
            )
        })?;
        let assets = result
            .images
            .iter()
            .filter_map(|(name, data)| decode_asset(name, data))
            .collect::<Vec<_>>();

        let char_count = markdown.chars().count();
        Ok(ParsedDocument {
            id: Uuid::now_v7(),
            source_name: source_name.to_string(),
            mime_type: mime_type.to_string(),
            markdown,
            assets,
            metadata: DocumentMetadata {
                page_count: None,
                char_count,
                estimated_tokens: char_count / 2,
                warnings: Vec::new(),
            },
        })
    }

    fn name(&self) -> &'static str {
        "mineru"
    }
}

fn decode_asset(name: &str, encoded: &str) -> Option<ParsedAsset> {
    let (mime_type, data) = encoded
        .strip_prefix("data:")
        .and_then(|s| s.split_once(";base64,"))
        .map_or_else(
            || (mime_from_name(name), encoded),
            |(mime, data)| (mime.to_string(), data),
        );

    let bytes = general_purpose::STANDARD.decode(data).ok()?;
    Some(ParsedAsset {
        name: name.to_string(),
        mime_type,
        data: bytes,
    })
}

fn mime_from_name(name: &str) -> String {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default();
    if ext.eq_ignore_ascii_case("png") {
        "image/png"
    } else if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") {
        "image/jpeg"
    } else if ext.eq_ignore_ascii_case("webp") {
        "image/webp"
    } else if ext.eq_ignore_ascii_case("bmp") {
        "image/bmp"
    } else if ext.eq_ignore_ascii_case("tif") || ext.eq_ignore_ascii_case("tiff") {
        "image/tiff"
    } else {
        "application/octet-stream"
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::{decode_asset, mime_from_name, MineruResponse};

    #[test]
    fn decode_asset_accepts_data_url_images() {
        let asset = decode_asset("image-1.jpg", "data:image/jpeg;base64,aGVsbG8=")
            .expect("asset should decode");

        assert_eq!(asset.name, "image-1.jpg");
        assert_eq!(asset.mime_type, "image/jpeg");
        assert_eq!(asset.data, b"hello");
    }

    #[test]
    fn decode_asset_falls_back_to_extension_mime() {
        let asset = decode_asset("chart.webp", "aGVsbG8=").expect("asset should decode");

        assert_eq!(asset.mime_type, "image/webp");
        assert_eq!(asset.data, b"hello");
    }

    #[test]
    fn decode_asset_rejects_invalid_base64() {
        assert!(decode_asset("bad.png", "not base64").is_none());
    }

    #[test]
    fn mime_from_name_handles_known_image_extensions() {
        assert_eq!(mime_from_name("a.PNG"), "image/png");
        assert_eq!(mime_from_name("a.jpeg"), "image/jpeg");
        assert_eq!(mime_from_name("a.tiff"), "image/tiff");
        assert_eq!(mime_from_name("a.unknown"), "application/octet-stream");
    }

    #[test]
    fn mineru_response_accepts_markdown_alias() {
        let response: MineruResponse = serde_json::from_str(
            r##"{"results":{"sample":{"markdown":"# Title","images":{}}}}"##,
        )
        .expect("response should parse");

        let result = response
            .results
            .get("sample")
            .expect("sample result should exist");
        assert_eq!(result.md_content.as_deref(), Some("# Title"));
    }
}
