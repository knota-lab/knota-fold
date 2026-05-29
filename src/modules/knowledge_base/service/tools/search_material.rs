use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use rig::completion::ToolDefinition;
use rig::tool::Tool;

use super::MaterialRegistry;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct SearchMaterialError(pub String);

impl fmt::Display for SearchMaterialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "search material error: {}", self.0)
    }
}

impl std::error::Error for SearchMaterialError {}

// ---------------------------------------------------------------------------
// Args
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchMaterialArgs {
    /// Search keyword
    pub query: String,
    /// Limit to a specific material (optional)
    pub material_id: Option<String>,
    /// Max results (default 10)
    pub top_k: Option<u32>,
}

// ---------------------------------------------------------------------------
// Internal enum to abstract over document / inline text
// ---------------------------------------------------------------------------

struct MaterialRef<'a> {
    id: String,
    title: String,
    content: &'a str,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DEFAULT_TOP_K: u32 = 10;
const MAX_TOP_K: u32 = 20;
const CONTEXT_LINES: usize = 3;

// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchMaterialTool {
    #[serde(skip)]
    pub registry: Arc<MaterialRegistry>,
}

// ---------------------------------------------------------------------------
// Tool trait implementation
// ---------------------------------------------------------------------------

impl Tool for SearchMaterialTool {
    const NAME: &'static str = "search_material";

    type Error = SearchMaterialError;
    type Args = SearchMaterialArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "在当前会话的材料中搜索关键词。返回匹配的行及其上下文（前后各 3 行）和行号。如果材料很长，先搜索定位再精确读取效率更高。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "搜索关键词"
                    },
                    "material_id": {
                        "type": "string",
                        "description": "限定搜索的材料 ID（文档 UUID 或内联文本 ID），不指定则搜索全部材料"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "最大返回结果数（默认 10，最大 20）"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    #[tracing::instrument(
        skip(self, args),
        fields(tool = "search_material", query = %args.query)
    )]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let top_k = args.top_k.unwrap_or(DEFAULT_TOP_K).min(MAX_TOP_K) as usize;
        let query_lower = args.query.to_lowercase();

        // Resolve target materials
        let targets = if let Some(ref mid) = args.material_id {
            // Specific material requested
            let mat = match Uuid::parse_str(mid) {
                Ok(uuid) => self
                    .registry
                    .get_document(&uuid)
                    .map(|d| MaterialRef {
                        id: d.id.to_string(),
                        title: d.title.clone(),
                        content: &d.content,
                    })
                    .or_else(|| {
                        self.registry.get_inline(mid).map(|t| MaterialRef {
                            id: t.id.clone(),
                            title: t.label.clone(),
                            content: &t.content,
                        })
                    }),
                Err(_) => self.registry.get_inline(mid).map(|t| MaterialRef {
                    id: t.id.clone(),
                    title: t.label.clone(),
                    content: &t.content,
                }),
            };
            match mat {
                Some(m) => vec![m],
                None => {
                    return Err(SearchMaterialError(format!("材料 {mid} 不在当前会话中")))
                }
            }
        } else {
            // Search all materials, ordered by registration_order
            let ordered_ids = self.registry.registration_order_ids();
            let mut all: Vec<MaterialRef<'_>> = self
                .registry
                .documents
                .values()
                .map(|d| MaterialRef {
                    id: d.id.to_string(),
                    title: d.title.clone(),
                    content: &d.content,
                })
                .chain(self.registry.inline_texts.iter().map(|t| MaterialRef {
                    id: t.id.clone(),
                    title: t.label.clone(),
                    content: &t.content,
                }))
                .collect();

            // Sort by registration order (materials not in the order list go to the end)
            all.sort_by_key(|m| {
                ordered_ids
                    .iter()
                    .position(|id| *id == m.id)
                    .unwrap_or(usize::MAX)
            });
            all
        };

        let total_materials = targets.len();

        // Search through materials
        let mut results = Vec::new();
        let mut total_matches = 0;

        for mat in &targets {
            let lines: Vec<&str> = mat.content.lines().collect();
            let mut mat_matches: Vec<String> = Vec::new();

            for (i, line) in lines.iter().enumerate() {
                if line.to_lowercase().contains(&query_lower) {
                    let line_num = i + 1; // 1-indexed

                    // Collect context lines (before)
                    let mut context = Vec::new();
                    let ctx_start = i.saturating_sub(CONTEXT_LINES);
                    for (ci, ctx_line) in lines.iter().enumerate().take(i).skip(ctx_start)
                    {
                        context.push(format!("    {}: {}", ci + 1, ctx_line));
                    }

                    // The matched line
                    let matched = format!("  {line_num}: {line}");

                    // Collect context lines (after)
                    let ctx_end = (i + CONTEXT_LINES + 1).min(lines.len());
                    for (ci, ctx_line) in
                        lines.iter().enumerate().take(ctx_end).skip(i + 1)
                    {
                        context.push(format!("    {}: {}", ci + 1, ctx_line));
                    }

                    mat_matches.push(format!("{}\n{}", matched, context.join("\n")));
                    total_matches += 1;

                    if total_matches >= top_k {
                        break;
                    }
                }
                if total_matches >= top_k {
                    break;
                }
            }

            if !mat_matches.is_empty() {
                results.push(format!(
                    "=== 材料: {} (ID: {}) ===\n{}",
                    mat.title,
                    mat.id,
                    mat_matches.join("\n\n")
                ));
            }

            if total_matches >= top_k {
                break;
            }
        }

        if results.is_empty() {
            return Ok(format!(
                "在 {} 份材料中搜索 \"{}\"，未找到匹配。",
                total_materials, args.query
            ));
        }

        Ok(format!(
            "在 {} 份材料中搜索 \"{}\"，找到 {} 个匹配：\n\n{}",
            total_materials,
            args.query,
            total_matches,
            results.join("\n\n")
        ))
    }
}
