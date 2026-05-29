use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use rig::completion::ToolDefinition;
use rig::tool::Tool;

use super::{DocumentContent, InlineText, MaterialRegistry};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ReadMaterialError(pub String);

impl fmt::Display for ReadMaterialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "read material error: {}", self.0)
    }
}

impl std::error::Error for ReadMaterialError {}

// ---------------------------------------------------------------------------
// Args
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReadMaterialArgs {
    pub material_id: String,
    /// 1-indexed start line (defaults to 1).
    pub start_line: Option<u32>,
    /// 1-indexed end line (defaults to start_line + 500, capped at total).
    pub end_line: Option<u32>,
}

// ---------------------------------------------------------------------------
// Internal enum to abstract over document / inline text
// ---------------------------------------------------------------------------

enum MaterialRef<'a> {
    Doc(&'a DocumentContent),
    Inline(&'a InlineText),
}

impl MaterialRef<'_> {
    fn id(&self) -> String {
        match self {
            MaterialRef::Doc(d) => d.id.to_string(),
            MaterialRef::Inline(t) => t.id.clone(),
        }
    }

    fn content(&self) -> &str {
        match self {
            MaterialRef::Doc(d) => &d.content,
            MaterialRef::Inline(t) => &t.content,
        }
    }

    fn total_lines(&self) -> usize {
        match self {
            MaterialRef::Doc(d) => d.total_lines,
            MaterialRef::Inline(t) => t.total_lines,
        }
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_LINES_PER_READ: u32 = 500;

// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReadMaterialTool {
    #[serde(skip)]
    pub registry: Arc<MaterialRegistry>,
}

// ---------------------------------------------------------------------------
// Tool trait implementation
// ---------------------------------------------------------------------------

impl Tool for ReadMaterialTool {
    const NAME: &'static str = "read_material";

    type Error = ReadMaterialError;
    type Args = ReadMaterialArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "读取指定材料的内容。可以指定起始行和结束行来分页浏览，每次最多读取 500 行。行号为 1-indexed。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "material_id": {
                        "type": "string",
                        "description": "材料的 ID（文档 UUID 或内联文本 ID）"
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "起始行号（1-indexed，默认为 1）"
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "结束行号（1-indexed，默认为 start_line + 500）"
                    }
                },
                "required": ["material_id"]
            }),
        }
    }

    #[tracing::instrument(
        skip(self, args),
        fields(tool = "read_material", material_id = %args.material_id)
    )]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Resolve material: try UUID first, then inline ID
        let material = match Uuid::parse_str(&args.material_id) {
            Ok(uuid) => self
                .registry
                .get_document(&uuid)
                .map(MaterialRef::Doc)
                .or_else(|| {
                    self.registry
                        .get_inline(&args.material_id)
                        .map(MaterialRef::Inline)
                }),
            Err(_) => self
                .registry
                .get_inline(&args.material_id)
                .map(MaterialRef::Inline),
        };

        let material = material.ok_or_else(|| {
            ReadMaterialError(format!("材料 {} 不在当前会话中", args.material_id))
        })?;

        let total = material.total_lines();

        // 1-indexed, default start = 1
        let start = args.start_line.unwrap_or(1).max(1) as usize;

        // Out-of-range check
        if start > total {
            return Ok(format!(
                "材料 {} 共 {} 行，请求的起始行 {} 超出范围。",
                material.id(),
                total,
                start,
            ));
        }

        let default_end = start as u32 + MAX_LINES_PER_READ;
        let end = args
            .end_line
            .unwrap_or(default_end)
            .min(start as u32 + MAX_LINES_PER_READ) as usize;

        let actual_end = end.min(total).max(start);

        let mut output = Vec::with_capacity(actual_end - start + 2);
        output.push(format!(
            "材料 {}（共 {} 行，显示第 {}-{} 行）:",
            material.id(),
            total,
            start,
            actual_end,
        ));

        // Read only the needed line range using skip/take — avoids collecting all lines.
        for (i, line) in material
            .content()
            .lines()
            .enumerate()
            .skip(start - 1)
            .take(actual_end - start + 1)
        {
            output.push(format!("{}: {}", i + 1, line));
        }

        Ok(output.join("\n"))
    }
}
