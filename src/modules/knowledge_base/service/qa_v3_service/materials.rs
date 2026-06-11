use std::sync::Arc;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::models::_entities::{chat_messages, chat_sessions, kb_documents};
use crate::modules::knowledge_base::service::qa_stream_types::{QaEvent, QaPhase};
use crate::modules::knowledge_base::service::tools::{
    DocumentContent, InlineText, MaterialRegistry,
};

use super::{save_user_turn, send_event, send_event_blocking, MaterialPrep, QaStreamCtx};

pub(super) async fn prepare_materials(
    ctx: &QaStreamCtx<'_>,
    session: &chat_sessions::Model,
    history: &[chat_messages::Model],
) -> Result<MaterialPrep, ()> {
    let material_span = tracing::Span::current();
    send_event(
        ctx.tx,
        QaEvent::PhaseChanged {
            phase: QaPhase::MaterialProcessing {
                strategy: "v3_registry".to_string(),
                total_chunks: None,
            },
        },
    )
    .await?;

    let mut registry = MaterialRegistry::default();
    let mut current_turn_materials = Vec::new();
    let mut inline_material_id: Option<String> = None;
    register_request_materials(
        ctx,
        &mut registry,
        &mut current_turn_materials,
        &mut inline_material_id,
    )
    .await?;
    recover_history_materials(ctx, history, &mut registry).await;

    let registry = Arc::new(registry);
    let material_count = registry.all_materials().len();
    material_span.record("material_count", material_count);
    tracing::info!(
        material_count,
        "Materials registered: count={}",
        material_count
    );
    save_user_turn(ctx, session, history, inline_material_id.as_deref()).await?;

    Ok(MaterialPrep {
        registry,
        current_turn_materials,
    })
}

async fn register_request_materials(
    ctx: &QaStreamCtx<'_>,
    registry: &mut MaterialRegistry,
    current_turn_materials: &mut Vec<String>,
    inline_material_id: &mut Option<String>,
) -> Result<(), ()> {
    register_inline_material(ctx, registry, current_turn_materials, inline_material_id);
    register_document_materials(ctx, registry, current_turn_materials).await?;
    register_file_materials(ctx, registry, current_turn_materials).await
}

fn register_inline_material(
    ctx: &QaStreamCtx<'_>,
    registry: &mut MaterialRegistry,
    current_turn_materials: &mut Vec<String>,
    inline_material_id: &mut Option<String>,
) {
    let Some(inline_text) = ctx.request.material.inline.as_ref() else {
        return;
    };
    let content = if inline_text.len() > ctx.config.max_inline_chars {
        tracing::warn!(
            len = inline_text.len(),
            max = ctx.config.max_inline_chars,
            "Inline material exceeds size limit, truncating"
        );
        inline_text
            .chars()
            .take(ctx.config.max_inline_chars)
            .collect::<String>()
    } else {
        inline_text.clone()
    };
    let id = format!("inline-{}", Uuid::now_v7().simple());
    let total_lines = content.lines().count();
    *inline_material_id = Some(id.clone());
    registry.register_inline(InlineText {
        id: id.clone(),
        label: "用户粘贴文本".to_string(),
        content,
        total_lines,
    });
    current_turn_materials.push(format!("{id} 用户粘贴文本 ({total_lines}行)"));
}

async fn register_document_materials(
    ctx: &QaStreamCtx<'_>,
    registry: &mut MaterialRegistry,
    current_turn_materials: &mut Vec<String>,
) -> Result<(), ()> {
    if ctx.request.material.document_ids.is_empty() {
        return Ok(());
    }

    let requested_ids: std::collections::HashSet<Uuid> =
        ctx.request.material.document_ids.iter().copied().collect();
    let docs = kb_documents::Entity::find()
        .filter(kb_documents::Column::TenantId.eq(ctx.tenant_id))
        .filter(kb_documents::Column::Id.is_in(ctx.request.material.document_ids.clone()))
        .filter(kb_documents::Column::DeletedAt.is_null())
        .all(ctx.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to fetch documents by ID");
            let _ = send_event_blocking(
                ctx.tx,
                &QaEvent::Error {
                    message: format!("Failed to fetch documents: {e}"),
                },
            );
        })?;

    let found_ids: std::collections::HashSet<Uuid> = docs.iter().map(|d| d.id).collect();
    let missing_ids: Vec<&Uuid> = requested_ids.difference(&found_ids).collect();
    if !missing_ids.is_empty() {
        tracing::warn!(
            ?missing_ids,
            "Some requested documents not found or not accessible for this tenant"
        );
    }
    add_docs_to_registry(registry, current_turn_materials, &docs);
    Ok(())
}

async fn register_file_materials(
    ctx: &QaStreamCtx<'_>,
    registry: &mut MaterialRegistry,
    current_turn_materials: &mut Vec<String>,
) -> Result<(), ()> {
    if ctx.request.material.file_ids.is_empty() {
        return Ok(());
    }

    let docs = kb_documents::Entity::find()
        .filter(kb_documents::Column::TenantId.eq(ctx.tenant_id))
        .filter(kb_documents::Column::FileId.is_in(ctx.request.material.file_ids.clone()))
        .filter(kb_documents::Column::DeletedAt.is_null())
        .all(ctx.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to resolve file_ids to documents");
            let _ = send_event_blocking(
                ctx.tx,
                &QaEvent::Error {
                    message: format!("Failed to resolve file_ids: {e}"),
                },
            );
        })?;

    add_docs_to_registry(registry, current_turn_materials, &docs);
    Ok(())
}

fn add_docs_to_registry(
    registry: &mut MaterialRegistry,
    current_turn_materials: &mut Vec<String>,
    docs: &[kb_documents::Model],
) {
    for doc in docs {
        register_doc_from_model(registry, doc);
        let total_lines = doc.full_text.as_deref().map_or(0, |t| t.lines().count());
        current_turn_materials
            .push(format!("{} {} ({}行)", doc.id, doc.title, total_lines));
    }
}

async fn recover_history_materials(
    ctx: &QaStreamCtx<'_>,
    history: &[chat_messages::Model],
    registry: &mut MaterialRegistry,
) {
    for msg in history {
        let Some(ref refs) = msg.material_refs else {
            continue;
        };
        recover_history_document_refs(ctx, registry, refs).await;
        recover_history_file_refs(ctx, registry, refs).await;
        recover_history_inline_ref(registry, refs);
    }
}

async fn recover_history_document_refs(
    ctx: &QaStreamCtx<'_>,
    registry: &mut MaterialRegistry,
    refs: &serde_json::Value,
) {
    let Some(doc_ids) = refs.get("documentIds").and_then(|v| v.as_array()) else {
        return;
    };
    let ids: Vec<Uuid> = doc_ids
        .iter()
        .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
        .collect();
    if ids.is_empty() {
        return;
    }
    let docs = kb_documents::Entity::find()
        .filter(kb_documents::Column::TenantId.eq(ctx.tenant_id))
        .filter(kb_documents::Column::Id.is_in(ids))
        .filter(kb_documents::Column::DeletedAt.is_null())
        .all(ctx.db)
        .await
        .unwrap_or_default();
    for doc in &docs {
        register_doc_from_model(registry, doc);
    }
}

async fn recover_history_file_refs(
    ctx: &QaStreamCtx<'_>,
    registry: &mut MaterialRegistry,
    refs: &serde_json::Value,
) {
    let Some(file_ids) = refs.get("fileIds").and_then(|v| v.as_array()) else {
        return;
    };
    let ids: Vec<Uuid> = file_ids
        .iter()
        .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
        .collect();
    if ids.is_empty() {
        return;
    }
    let docs = kb_documents::Entity::find()
        .filter(kb_documents::Column::TenantId.eq(ctx.tenant_id))
        .filter(kb_documents::Column::FileId.is_in(ids))
        .filter(kb_documents::Column::DeletedAt.is_null())
        .all(ctx.db)
        .await
        .unwrap_or_default();
    for doc in &docs {
        register_doc_from_model(registry, doc);
    }
}

fn recover_history_inline_ref(registry: &mut MaterialRegistry, refs: &serde_json::Value) {
    let Some(inline_obj) = refs.get("inline").and_then(|v| v.as_object()) else {
        return;
    };
    let Some(content) = inline_obj.get("content").and_then(|v| v.as_str()) else {
        return;
    };
    let id = inline_obj
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("recovered-inline")
        .to_string();
    if registry.get_inline(&id).is_some() {
        return;
    }
    registry.register_inline(InlineText {
        id,
        label: inline_obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("历史粘贴文本")
            .to_string(),
        content: content.to_string(),
        total_lines: content.lines().count(),
    });
}

/// Register a document from the DB into the registry.
fn register_doc_from_model(registry: &mut MaterialRegistry, doc: &kb_documents::Model) {
    let content = doc.full_text.clone().unwrap_or_default();
    registry.register_document(DocumentContent {
        id: doc.id,
        title: doc.title.clone(),
        content,
        doc_type: doc.source_type.clone(),
        total_lines: doc.full_text.as_deref().map_or(0, |t| t.lines().count()),
    });
}
