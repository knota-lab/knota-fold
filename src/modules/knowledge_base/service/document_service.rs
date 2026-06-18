use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter, QueryOrder,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::models::_entities::{document_lines, files, kb_chunks, kb_documents};
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::models::{
    document_lines as dl_models, kb_chunks as kc_models, kb_documents as kd_models,
};
use crate::utils::error::IntoAppError;

/// Parameters for [`create_document`].
#[derive(Debug)]
pub struct CreateDocumentParams {
    pub tenant_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub library_id: Option<Uuid>,
    pub folder_id: Option<Uuid>,
    pub source_type: String,
    pub scope: String,
    pub file_id: Option<Uuid>,
    pub file_reference_id: Option<Uuid>,
    pub created_by: Uuid,
}

/// Create a new `kb_documents` record with status='pending'.
/// Returns the created model.
#[tracing::instrument(skip(db))]
pub async fn create_document(
    db: &impl ConnectionTrait,
    params: &CreateDocumentParams,
) -> loco_rs::Result<kb_documents::Model> {
    let model = kd_models::ActiveModel {
        tenant_id: ActiveValue::Set(params.tenant_id),
        title: ActiveValue::Set(params.title.clone()),
        description: ActiveValue::Set(params.description.clone()),
        library_id: ActiveValue::Set(params.library_id),
        folder_id: ActiveValue::Set(params.folder_id),
        source_type: ActiveValue::Set(params.source_type.clone()),
        scope: ActiveValue::Set(params.scope.clone()),
        file_id: ActiveValue::Set(params.file_id),
        file_reference_id: ActiveValue::Set(params.file_reference_id),
        full_text: ActiveValue::Set(None),
        status: ActiveValue::Set("pending".to_string()),
        chunk_count: ActiveValue::Set(0),
        total_tokens: ActiveValue::Set(0),
        metadata: ActiveValue::Set(None),
        error_message: ActiveValue::Set(None),
        created_by: ActiveValue::Set(params.created_by),
        ..Default::default()
    };
    model.insert(db).await.db_err()
}

/// Find an existing active knowledge-base document backed by the same physical
/// file content in the same library/folder scope.
#[tracing::instrument(skip(db, file))]
pub async fn find_duplicate_file_document(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
    library_id: Option<Uuid>,
    folder_id: Option<Uuid>,
    file: &files::Model,
) -> loco_rs::Result<Option<kb_documents::Model>> {
    let file_ids = files::Entity::find()
        .filter(files::Column::TenantId.eq(tenant_id))
        .filter(files::Column::ContentHash.eq(&file.content_hash))
        .filter(files::Column::ContentHashAlgo.eq(&file.content_hash_algo))
        .filter(files::Column::Size.eq(file.size))
        .filter(files::Column::DeletedAt.is_null())
        .all(db)
        .await
        .db_err()?
        .into_iter()
        .map(|file| file.id)
        .collect::<Vec<_>>();

    if file_ids.is_empty() {
        return Ok(None);
    }

    let mut query = kb_documents::Entity::find()
        .filter(kb_documents::Column::TenantId.eq(tenant_id))
        .filter(kb_documents::Column::FileId.is_in(file_ids))
        .filter(kb_documents::Column::DeletedAt.is_null())
        .order_by_desc(kb_documents::Column::CreatedAt);

    query = match library_id {
        Some(id) => query.filter(kb_documents::Column::LibraryId.eq(id)),
        None => query.filter(kb_documents::Column::LibraryId.is_null()),
    };
    query = match folder_id {
        Some(id) => query.filter(kb_documents::Column::FolderId.eq(id)),
        None => query.filter(kb_documents::Column::FolderId.is_null()),
    };

    query.one(db).await.db_err()
}

/// Attach a file reference id to an existing document.
///
/// # Errors
///
/// Returns a DB error if the document cannot be updated.
#[tracing::instrument(skip(db))]
pub async fn set_file_reference(
    db: &impl ConnectionTrait,
    document_id: Uuid,
    tenant_id: Uuid,
    file_reference_id: Uuid,
) -> loco_rs::Result<kb_documents::Model> {
    let doc = kb_documents::Entity::find_by_id(document_id)
        .filter(kb_documents::Column::TenantId.eq(tenant_id))
        .filter(kb_documents::Column::DeletedAt.is_null())
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())?;
    let mut active: kd_models::ActiveModel = doc.into();
    active.file_reference_id = ActiveValue::Set(Some(file_reference_id));
    active.update(db).await.db_err()
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
        .filter(kb_documents::Column::DeletedAt.is_null())
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())?;

    let current = doc.status.as_str();
    let valid = match new_status {
        "indexing" => current == "pending",
        "ready" | "error" => current == "indexing",
        _ => false,
    };
    if !valid {
        return Err(KnowledgeBaseError::IndexingError(format!(
            "invalid status transition: {current} -> {new_status}"
        ))
        .to_err());
    }

    let mut active: kd_models::ActiveModel = doc.into();
    active.status = ActiveValue::Set(new_status.to_string());
    active.error_message =
        ActiveValue::Set(error_message.map(std::string::ToString::to_string));
    active.updated_at = ActiveValue::Set(Utc::now().naive_utc());
    active.update(db).await.db_err()?;
    Ok(())
}

/// Mark a document as indexing before the worker starts expensive I/O.
///
/// This is intentionally idempotent for `pending` and `indexing` so retrying a
/// queued worker does not fail before it can report the real indexing error.
#[tracing::instrument(skip(db))]
pub async fn start_indexing(
    db: &DatabaseConnection,
    document_id: Uuid,
    tenant_id: Uuid,
) -> loco_rs::Result<()> {
    let doc = kb_documents::Entity::find_by_id(document_id)
        .filter(kb_documents::Column::TenantId.eq(tenant_id))
        .filter(kb_documents::Column::DeletedAt.is_null())
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())?;

    if !matches!(doc.status.as_str(), "pending" | "indexing") {
        return Err(KnowledgeBaseError::IndexingError(format!(
            "cannot start indexing from status '{}'",
            doc.status
        ))
        .to_err());
    }

    let mut active: kd_models::ActiveModel = doc.into();
    active.status = ActiveValue::Set("indexing".to_string());
    active.error_message = ActiveValue::Set(None);
    active.updated_at = ActiveValue::Set(Utc::now().naive_utc());
    active.update(db).await.db_err()?;
    Ok(())
}

/// Mark a document as error after a background indexing failure.
///
/// This intentionally accepts both `pending` and `indexing`: a worker may fail
/// before or after flipping the status, and either way the UI should not be
/// left with a permanently running document.
#[tracing::instrument(skip(db))]
pub async fn mark_error(
    db: &DatabaseConnection,
    document_id: Uuid,
    tenant_id: Uuid,
    error_message: &str,
) -> loco_rs::Result<()> {
    let doc = kb_documents::Entity::find_by_id(document_id)
        .filter(kb_documents::Column::TenantId.eq(tenant_id))
        .filter(kb_documents::Column::DeletedAt.is_null())
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())?;

    if !matches!(doc.status.as_str(), "pending" | "indexing") {
        return Ok(());
    }

    let mut active: kd_models::ActiveModel = doc.into();
    active.status = ActiveValue::Set("error".to_string());
    active.error_message = ActiveValue::Set(Some(error_message.to_string()));
    active.updated_at = ActiveValue::Set(Utc::now().naive_utc());
    active.update(db).await.db_err()?;
    Ok(())
}

/// Update `full_text` and set status to 'indexing'.
#[tracing::instrument(skip(db, full_text))]
pub async fn set_full_text(
    db: &DatabaseConnection,
    document_id: Uuid,
    full_text: &str,
) -> loco_rs::Result<()> {
    let doc = kb_documents::Entity::find_by_id(document_id)
        .filter(kb_documents::Column::DeletedAt.is_null())
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

#[tracing::instrument(skip(db, full_text, metadata))]
pub async fn set_parsed_content(
    db: &DatabaseConnection,
    document_id: Uuid,
    full_text: &str,
    metadata: serde_json::Value,
) -> loco_rs::Result<()> {
    let doc = kb_documents::Entity::find_by_id(document_id)
        .filter(kb_documents::Column::DeletedAt.is_null())
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())?;

    let mut active: kd_models::ActiveModel = doc.into();
    active.full_text = ActiveValue::Set(Some(full_text.to_string()));
    active.metadata =
        ActiveValue::Set(Some(merge_metadata(active.metadata.clone(), metadata)));
    active.status = ActiveValue::Set("indexing".to_string());
    active.updated_at = ActiveValue::Set(Utc::now().naive_utc());
    active.update(db).await.db_err()?;
    Ok(())
}

#[tracing::instrument(skip(db))]
pub async fn is_indexing_active(
    db: &DatabaseConnection,
    document_id: Uuid,
    tenant_id: Uuid,
) -> loco_rs::Result<bool> {
    let Some(doc) = kb_documents::Entity::find_by_id(document_id)
        .filter(kb_documents::Column::TenantId.eq(tenant_id))
        .filter(kb_documents::Column::DeletedAt.is_null())
        .one(db)
        .await
        .db_err()?
    else {
        return Ok(false);
    };

    Ok(matches!(doc.status.as_str(), "pending" | "indexing"))
}

/// Update `chunk_count` and `total_tokens`, set status to 'ready'.
#[tracing::instrument(skip(db))]
pub async fn mark_ready(
    db: &DatabaseConnection,
    document_id: Uuid,
    chunk_count: i32,
    total_tokens: i32,
) -> loco_rs::Result<()> {
    let doc = kb_documents::Entity::find_by_id(document_id)
        .filter(kb_documents::Column::DeletedAt.is_null())
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

fn merge_metadata(existing: ActiveValue<Option<Value>>, incoming: Value) -> Value {
    let mut base = match existing {
        ActiveValue::Set(Some(value)) | ActiveValue::Unchanged(Some(value)) => value,
        _ => json!({}),
    };

    match (&mut base, incoming) {
        (Value::Object(base_map), Value::Object(incoming_map)) => {
            base_map.extend(incoming_map);
            base
        }
        (_, incoming_value) => incoming_value,
    }
}

/// Get document by ID, verifying `tenant_id` ownership.
#[tracing::instrument(skip(db))]
pub async fn get_document(
    db: &impl ConnectionTrait,
    document_id: Uuid,
    tenant_id: Uuid,
) -> loco_rs::Result<kb_documents::Model> {
    kb_documents::Entity::find_by_id(document_id)
        .filter(kb_documents::Column::TenantId.eq(tenant_id))
        .filter(kb_documents::Column::DeletedAt.is_null())
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())
}

#[tracing::instrument(skip(db))]
pub async fn soft_delete_document(
    db: &impl ConnectionTrait,
    document_id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
) -> loco_rs::Result<kb_documents::Model> {
    let doc = get_document(db, document_id, tenant_id).await?;
    let mut active: kd_models::ActiveModel = doc.into();
    active.status = ActiveValue::Set("deleted".to_string());
    active.chunk_count = ActiveValue::Set(0);
    active.total_tokens = ActiveValue::Set(0);
    active.error_message = ActiveValue::Set(None);
    active.deleted_at = ActiveValue::Set(Some(Utc::now().naive_utc()));
    active.deleted_by = ActiveValue::Set(Some(user_id));
    active.updated_at = ActiveValue::Set(Utc::now().naive_utc());
    active.update(db).await.db_err()
}

/// Batch insert `kb_chunks` records.
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

/// Batch insert `document_lines` records.
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

#[tracing::instrument(skip(db))]
pub async fn clear_index_records(
    db: &impl ConnectionTrait,
    document_id: Uuid,
    tenant_id: Uuid,
) -> loco_rs::Result<()> {
    kb_chunks::Entity::delete_many()
        .filter(kb_chunks::Column::DocumentId.eq(document_id))
        .filter(kb_chunks::Column::TenantId.eq(tenant_id))
        .exec(db)
        .await
        .db_err()?;

    document_lines::Entity::delete_many()
        .filter(document_lines::Column::DocumentId.eq(document_id))
        .filter(document_lines::Column::TenantId.eq(tenant_id))
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
        .filter(kb_documents::Column::DeletedAt.is_null())
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
