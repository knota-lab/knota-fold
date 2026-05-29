use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use rig::completion::ToolDefinition;
use rig::tool::Tool;

use super::MaterialRegistry;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ListMaterialsError(pub String);

impl fmt::Display for ListMaterialsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "list materials error: {}", self.0)
    }
}

impl std::error::Error for ListMaterialsError {}

// ---------------------------------------------------------------------------
// Args (empty — no parameters needed)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ListMaterialsArgs {}

// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ListMaterialsTool {
    #[serde(skip)]
    pub registry: Arc<MaterialRegistry>,
}

// ---------------------------------------------------------------------------
// Tool trait implementation
// ---------------------------------------------------------------------------

impl Tool for ListMaterialsTool {
    const NAME: &'static str = "list_materials";

    type Error = ListMaterialsError;
    type Args = ListMaterialsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "列出当前会话中所有可用的材料（文档和内联文本）。返回每份材料的 ID、标题、字符数、行数和类型。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    #[tracing::instrument(skip(self, _args))]
    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let materials = self.registry.all_materials();

        if materials.is_empty() {
            return Ok("当前会话中没有可用材料。".to_string());
        }

        let mut lines = Vec::with_capacity(materials.len() + 1);
        lines.push(format!("可用材料（共 {} 份）:", materials.len()));

        for (i, m) in materials.iter().enumerate() {
            lines.push(format!(
                "{}. [{}] {} ({} 字符, {} 行, 类型: {})\n   预览: {}",
                i + 1,
                m.id,
                m.title,
                m.size_chars,
                m.total_lines,
                m.material_type,
                m.preview,
            ));
        }

        Ok(lines.join("\n"))
    }
}
