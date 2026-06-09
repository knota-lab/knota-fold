pub mod frontend_tool_stub;
pub mod list_conversation_history;
pub mod list_knowledge_base_documents;
pub mod list_knowledge_base_scope;
pub mod list_materials;
pub mod qa_v3_hook;
pub mod read_conversation_turn;
pub mod read_material;
pub mod search_knowledge_base;
pub mod search_material;
pub mod tool_result_broker;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Shared types used by both tools
// ---------------------------------------------------------------------------

/// A document fetched from the knowledge base and loaded into the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentContent {
    pub id: Uuid,
    pub title: String,
    pub content: String,
    pub doc_type: String,
    pub total_lines: usize,
}

/// An inline text block pasted by the user (not persisted to the DB).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineText {
    pub id: String,
    pub label: String,
    pub content: String,
    pub total_lines: usize,
}

/// A lightweight summary returned by `MaterialRegistry::all_materials()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialSummary {
    pub id: String,
    pub title: String,
    pub size_chars: usize,
    pub total_lines: usize,
    pub material_type: String,
    /// First N characters of the material content for quick preview.
    pub preview: String,
}

/// Tracks the order in which materials were registered.
#[derive(Debug, Clone)]
enum MaterialEntry {
    Doc(Uuid),
    Inline(String),
}

/// Holds all materials available in the current session so that tools can
/// look them up by ID without hitting the database.
#[derive(Debug, Clone, Default)]
pub struct MaterialRegistry {
    pub documents: HashMap<Uuid, DocumentContent>,
    pub inline_texts: Vec<InlineText>,
    /// Insertion order — determines the order in `list_materials` output.
    registration_order: Vec<MaterialEntry>,
}

const PREVIEW_MAX_CHARS: usize = 100;

fn truncate_preview(content: &str) -> String {
    let truncated: String = content.chars().take(PREVIEW_MAX_CHARS).collect();
    if content.chars().count() > PREVIEW_MAX_CHARS {
        format!("{truncated}…")
    } else {
        truncated
    }
}

impl MaterialRegistry {
    #[must_use]
    pub fn get_document(&self, id: &Uuid) -> Option<&DocumentContent> {
        self.documents.get(id)
    }

    #[must_use]
    pub fn get_inline(&self, id: &str) -> Option<&InlineText> {
        self.inline_texts.iter().find(|t| t.id == id)
    }

    #[must_use]
    pub fn all_materials(&self) -> Vec<MaterialSummary> {
        self.registration_order
            .iter()
            .filter_map(|entry| match entry {
                MaterialEntry::Doc(id) => {
                    self.documents.get(id).map(|d| MaterialSummary {
                        id: d.id.to_string(),
                        title: d.title.clone(),
                        size_chars: d.content.len(),
                        total_lines: d.total_lines,
                        material_type: d.doc_type.clone(),
                        preview: truncate_preview(&d.content),
                    })
                }
                MaterialEntry::Inline(id) => {
                    self.inline_texts.iter().find(|t| t.id == *id).map(|t| {
                        MaterialSummary {
                            id: t.id.clone(),
                            title: t.label.clone(),
                            size_chars: t.content.len(),
                            total_lines: t.total_lines,
                            material_type: "inline".to_string(),
                            preview: truncate_preview(&t.content),
                        }
                    })
                }
            })
            .collect()
    }

    #[must_use]
    pub fn registration_order_ids(&self) -> Vec<String> {
        self.registration_order
            .iter()
            .map(|entry| match entry {
                MaterialEntry::Doc(id) => id.to_string(),
                MaterialEntry::Inline(id) => id.clone(),
            })
            .collect()
    }

    pub fn register_document(&mut self, doc: DocumentContent) {
        let id = doc.id;
        // Only record order for new registrations (skip on recovery)
        if !self.documents.contains_key(&id) {
            self.registration_order.push(MaterialEntry::Doc(id));
        }
        self.documents.insert(id, doc);
    }

    pub fn register_inline(&mut self, text: InlineText) {
        let id = text.id.clone();
        // Skip entirely if already registered (prevents duplicate data in inline_texts)
        if self.inline_texts.iter().any(|t| t.id == id) {
            return;
        }
        self.registration_order.push(MaterialEntry::Inline(id));
        self.inline_texts.push(text);
    }
}

pub use frontend_tool_stub::FrontendToolStub;
pub use list_conversation_history::ListConversationHistoryTool;
pub use list_knowledge_base_documents::ListKnowledgeBaseDocumentsTool;
pub use list_knowledge_base_scope::ListKnowledgeBaseScopeTool;
pub use list_materials::ListMaterialsTool;
pub use qa_v3_hook::QaV3Hook;
pub use read_conversation_turn::ReadConversationTurnTool;
pub use read_material::ReadMaterialTool;
pub use search_knowledge_base::SearchKnowledgeBaseTool;
pub use search_material::SearchMaterialTool;
pub use tool_result_broker::in_process::InProcessBroker;
pub use tool_result_broker::{ToolResult, ToolResultBroker};
