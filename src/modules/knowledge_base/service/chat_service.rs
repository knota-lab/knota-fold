use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect,
};
use uuid::Uuid;

use crate::initializers::knowledge_base::SharedMemoryStore;
use crate::models::_entities::{chat_messages, chat_sessions};
use crate::modules::knowledge_base::models::{
    chat_messages as cm_models, chat_sessions as cs_models,
};
use crate::modules::knowledge_base::service::memory_service;
use crate::utils::error::IntoAppError;

/// Create a new chat session.
#[tracing::instrument(skip(db))]
pub async fn create_session(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    title: Option<String>,
) -> loco_rs::Result<chat_sessions::Model> {
    let model = cs_models::ActiveModel {
        tenant_id: ActiveValue::Set(tenant_id),
        user_id: ActiveValue::Set(user_id),
        title: ActiveValue::Set(title),
        ..Default::default()
    };
    model.insert(db).await.db_err()
}

/// Get a session by ID, verifying tenant and user ownership.
#[tracing::instrument(skip(db))]
pub async fn get_session(
    db: &DatabaseConnection,
    session_id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
) -> loco_rs::Result<chat_sessions::Model> {
    chat_sessions::Entity::find_by_id(session_id)
        .filter(chat_sessions::Column::TenantId.eq(tenant_id))
        .filter(chat_sessions::Column::UserId.eq(user_id))
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| {
            crate::views::errors::err_not_found(
                "knowledge_base.session_not_found",
                "session not found",
            )
        })
}

/// List sessions for a user in a tenant, ordered by most recent first.
#[tracing::instrument(skip(db))]
pub async fn list_sessions(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    limit: u64,
) -> loco_rs::Result<Vec<chat_sessions::Model>> {
    chat_sessions::Entity::find()
        .filter(chat_sessions::Column::TenantId.eq(tenant_id))
        .filter(chat_sessions::Column::UserId.eq(user_id))
        .order_by_desc(chat_sessions::Column::UpdatedAt)
        .limit(limit)
        .all(db)
        .await
        .db_err()
}

/// Delete a session, all its messages, and associated chat_memory vectors.
#[tracing::instrument(skip(db, memory_store))]
pub async fn delete_session(
    db: &DatabaseConnection,
    memory_store: &SharedMemoryStore,
    session_id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
) -> loco_rs::Result<()> {
    // Verify ownership
    let _session = get_session(db, session_id, tenant_id, user_id).await?;

    // Delete messages first
    chat_messages::Entity::delete_many()
        .filter(chat_messages::Column::SessionId.eq(session_id))
        .exec(db)
        .await
        .db_err()?;

    // Delete session
    chat_sessions::Entity::delete_by_id(session_id)
        .exec(db)
        .await
        .db_err()?;

    // Clean up Qdrant chat_memory vectors (best-effort, don't fail the request)
    if let Err(e) = memory_service::delete_by_session(
        &memory_store.client,
        &memory_store.collection_name,
        session_id,
        tenant_id,
    )
    .await
    {
        tracing::warn!(
            session_id = %session_id,
            error = %e,
            "Failed to delete chat_memory vectors (session DB records already deleted)"
        );
    }

    Ok(())
}

/// Parameters for [`create_message`].
#[derive(Debug)]
pub struct CreateMessageParams {
    pub session_id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub content: String,
    pub material_refs: Option<serde_json::Value>,
    pub intent: Option<String>,
    pub strategy: Option<String>,
    pub token_usage: Option<serde_json::Value>,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub total_tokens: i32,
}

/// Create a message in a session.
#[tracing::instrument(skip(db, params))]
pub async fn create_message(
    db: &DatabaseConnection,
    params: &CreateMessageParams,
) -> loco_rs::Result<chat_messages::Model> {
    let model = cm_models::ActiveModel {
        session_id: ActiveValue::Set(params.session_id),
        tenant_id: ActiveValue::Set(params.tenant_id),
        user_id: ActiveValue::Set(params.user_id),
        role: ActiveValue::Set(params.role.clone()),
        content: ActiveValue::Set(params.content.clone()),
        material_refs: ActiveValue::Set(params.material_refs.clone()),
        intent: ActiveValue::Set(params.intent.clone()),
        strategy: ActiveValue::Set(params.strategy.clone()),
        token_usage: ActiveValue::Set(params.token_usage.clone()),
        prompt_tokens: ActiveValue::Set(params.prompt_tokens),
        completion_tokens: ActiveValue::Set(params.completion_tokens),
        total_tokens: ActiveValue::Set(params.total_tokens),
        ..Default::default()
    };
    model.insert(db).await.db_err()
}

/// Get all messages for a session, ordered chronologically.
#[tracing::instrument(skip(db))]
pub async fn get_session_messages(
    db: &DatabaseConnection,
    session_id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
) -> loco_rs::Result<Vec<chat_messages::Model>> {
    // Verify session ownership
    let _session = get_session(db, session_id, tenant_id, user_id).await?;

    chat_messages::Entity::find()
        .filter(chat_messages::Column::SessionId.eq(session_id))
        .order_by_asc(chat_messages::Column::CreatedAt)
        .all(db)
        .await
        .db_err()
}

/// Update session title (e.g., auto-generate from first message).
#[tracing::instrument(skip(db))]
pub async fn update_session_title(
    db: &DatabaseConnection,
    session_id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
    title: &str,
) -> loco_rs::Result<()> {
    // Verify ownership before updating
    let _session = get_session(db, session_id, tenant_id, user_id).await?;

    let session = chat_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| {
            crate::views::errors::err_not_found(
                "knowledge_base.session_not_found",
                "session not found",
            )
        })?;

    let mut active: cs_models::ActiveModel = session.into();
    active.title = ActiveValue::Set(Some(title.to_string()));
    active.update(db).await.db_err()?;
    Ok(())
}
