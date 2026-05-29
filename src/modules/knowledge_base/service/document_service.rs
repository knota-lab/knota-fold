use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait,
    QueryFilter,
};
use uuid::Uuid;

use crate::models::_entities::{document_lines, kb_chunks, kb_documents};
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::models::{
    document_lines as dl_models, kb_chunks as kc_models, kb_documents as kd_models,
};
use crate::utils::error::IntoAppError;

/// Create a new kb_documents record with status='pending'.
/// Returns the created model.
#[tracing::instrument(skip(db))]
pub async fn create_document(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    title: String,
    description: Option<String>,
    source_type: String,
    scope: String,
    file_id: Option<Uuid>,
    created_by: Uuid,
) -> loco_rs::Result<kb_documents::Model> {
    let model = kd_models::ActiveModel {
        tenant_id: ActiveValue::Set(tenant_id),
        title: ActiveValue::Set(title),
        description: ActiveValue::Set(description),
        source_type: ActiveValue::Set(source_type),
        scope: ActiveValue::Set(scope),
        file_id: ActiveValue::Set(file_id),
        full_text: ActiveValue::Set(None),
        status: ActiveValue::Set("pending".to_string()),
        chunk_count: ActiveValue::Set(0),
        total_tokens: ActiveValue::Set(0),
        metadata: ActiveValue::Set(None),
        error_message: ActiveValue::Set(None),
        created_by: ActiveValue::Set(created_by),
        ..Default::default()
    };
    model.insert(db).await.db_err()
}

/// Update document status. Validates allowed transitions:
/// pending -> indexing -> ready | error
#[tracing::instrument(skip(db))]
pub async fn update_status(
    db: &DatabaseConnection,
    document_id: Uuid,
    tenant_id: Uuid,
    new_status: &str,
    error_message: Option<&str>,
) -> loco_rs::Result<()> {
    let doc = kb_documents::Entity::find_by_id(document_id)
        .filter(kb_documents::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())?;

    let current = doc.status.as_str();
    let valid = match new_status {
        "indexing" => current == "pending",
        "ready" => current == "indexing",
        "error" => current == "indexing",
        _ => false,
    };
    if !valid {
        return Err(loco_rs::Error::Message(format!(
            "invalid status transition: {current} -> {new_status}"
        )));
    }

    let mut active: kd_models::ActiveModel = doc.into();
    active.status = ActiveValue::Set(new_status.to_string());
    active.error_message = ActiveValue::Set(error_message.map(|s| s.to_string()));
    active.updated_at = ActiveValue::Set(Utc::now().naive_utc());
    active.update(db).await.db_err()?;
    Ok(())
}

/// Update full_text and set status to 'indexing'.
#[tracing::instrument(skip(db, full_text))]
pub async fn set_full_text(
    db: &DatabaseConnection,
    document_id: Uuid,
    full_text: &str,
) -> loco_rs::Result<()> {
    let doc = kb_documents::Entity::find_by_id(document_id)
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())?;

    let mut active: kd_models::ActiveModel = doc.into();
    active.full_text = ActiveValue::Set(Some(full_text.to_string()));
    active.status = ActiveValue::Set("indexing".to_string());
    active.updated_at = ActiveValue::Set(Utc::now().naive_utc());
    active.update(db).await.db_err()?;
    Ok(())
}

/// Update chunk_count and total_tokens, set status to 'ready'.
#[tracing::instrument(skip(db))]
pub async fn mark_ready(
    db: &DatabaseConnection,
    document_id: Uuid,
    chunk_count: i32,
    total_tokens: i32,
) -> loco_rs::Result<()> {
    let doc = kb_documents::Entity::find_by_id(document_id)
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())?;

    let mut active: kd_models::ActiveModel = doc.into();
    active.status = ActiveValue::Set("ready".to_string());
    active.chunk_count = ActiveValue::Set(chunk_count);
    active.total_tokens = ActiveValue::Set(total_tokens);
    active.updated_at = ActiveValue::Set(Utc::now().naive_utc());
    active.update(db).await.db_err()?;
    Ok(())
}

/// Get document by ID, verifying tenant_id ownership.
#[tracing::instrument(skip(db))]
pub async fn get_document(
    db: &DatabaseConnection,
    document_id: Uuid,
    tenant_id: Uuid,
) -> loco_rs::Result<kb_documents::Model> {
    kb_documents::Entity::find_by_id(document_id)
        .filter(kb_documents::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())
}

/// Batch insert kb_chunks records.
#[tracing::instrument(skip(db, chunks))]
pub async fn insert_chunks(
    db: &DatabaseConnection,
    chunks: Vec<kc_models::ActiveModel>,
) -> loco_rs::Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }
    kb_chunks::Entity::insert_many(chunks)
        .exec(db)
        .await
        .db_err()?;
    Ok(())
}

/// Batch insert document_lines records.
#[tracing::instrument(skip(db, lines))]
pub async fn insert_lines(
    db: &DatabaseConnection,
    lines: Vec<dl_models::ActiveModel>,
) -> loco_rs::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    document_lines::Entity::insert_many(lines)
        .exec(db)
        .await
        .db_err()?;
    Ok(())
}

/// Promote document scope from private to tenant.
/// Validates that the document exists, belongs to the tenant, was created by the user,
/// and is currently private.
#[tracing::instrument(skip(db))]
pub async fn promote_document(
    db: &DatabaseConnection,
    document_id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
) -> loco_rs::Result<kb_documents::Model> {
    let doc = kb_documents::Entity::find_by_id(document_id)
        .filter(kb_documents::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())?;

    if doc.created_by != user_id {
        return Err(KnowledgeBaseError::Forbidden.to_err());
    }
    if doc.scope != "private" {
        return Err(crate::views::errors::err_bad_request(
            "knowledge_base.document_not_private",
            "document is not private",
        ));
    }

    let mut active: kd_models::ActiveModel = doc.into();
    active.scope = ActiveValue::Set("tenant".to_string());
    active.updated_at = ActiveValue::Set(Utc::now().naive_utc());
    let result = active.update(db).await.db_err()?;
    Ok(result)
}
