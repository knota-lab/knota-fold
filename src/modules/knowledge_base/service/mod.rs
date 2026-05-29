pub mod chat_service;
pub mod chunking_service;
pub mod document_service;
pub mod line_splitting_service;
pub mod memory_service;
pub mod qa_compaction_service;
pub mod qa_stream_types;
pub mod qa_types;
pub mod qa_v3_service;
pub mod search_service;
pub mod tools;

pub use chat_service::{
    create_message, create_session, delete_session, get_session, get_session_messages,
    list_sessions, update_session_title,
};
pub use chunking_service::{chunk_markdown, RawChunk};
pub use document_service::{
    create_document, get_document, insert_chunks, insert_lines, mark_ready,
    promote_document, set_full_text, update_status,
};
pub use line_splitting_service::{split_lines, RawLine};
pub use qa_stream_types::{QaEvent, QaPhase, QaStreamResponse};
pub use qa_types::{Citation, MaterialInput, QaRequest, TokenUsage};
pub use qa_v3_service::process_qa_v3_stream;
pub use search_service::{hybrid_search, results_to_citations};
pub use tools::{
    DocumentContent as ToolDocumentContent, InlineText, ListMaterialsTool,
    MaterialRegistry, MaterialSummary, ReadMaterialTool,
};
