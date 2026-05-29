//! Entry management for super-admin — list, locate, and cascade-delete i18n entries.

use loco_rs::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use uuid::Uuid;

use crate::models::_entities::{i18n_entries, i18n_translations};
use crate::models::i18n_queries;
use crate::services::audit_service;
use crate::utils::error::{IntoAppError, IntoModelResult, OptionErrInto};
use crate::views::audit_logs::{AuditAction, AuditContext};
use crate::views::i18n::{EntryListResponse, EntryLocationResponse, EntryResponse};

pub async fn list_entries(
    db: &DatabaseConnection,
    namespace: Option<&str>,
    status: Option<&str>,
    page: u64,
    page_size: u64,
) -> loco_rs::Result<EntryListResponse> {
    let (rows, total) =
        i18n_queries::list_entries(db, namespace, status, page, page_size)
            .await
            .db_err()?;

    let items: Vec<EntryResponse> = rows.iter().map(EntryResponse::from).collect();

    Ok(EntryListResponse {
        items,
        total_items: total,
    })
}

pub async fn list_entry_locations(
    db: &DatabaseConnection,
    entry_id: Uuid,
) -> loco_rs::Result<Vec<EntryLocationResponse>> {
    let rows = i18n_queries::list_entry_locations(db, entry_id)
        .await
        .db_err()?;

    Ok(rows
        .into_iter()
        .map(|r| EntryLocationResponse {
            id: r.id.to_string(),
            file_path: r.file_path,
            line: r.line,
        })
        .collect())
}

/// Force-delete an entry and cascade its translations (global + all tenants)
/// and locations. Use with caution — this is a destructive admin operation.
pub async fn delete_entry_cascade(
    ctx: &AppContext,
    entry_id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    let entry = i18n_entries::Entity::find_by_id(entry_id)
        .one(&ctx.db)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    let before = serde_json::json!({
        "namespace": entry.namespace,
        "key": entry.key,
        "status": entry.status,
    });
    let ns = entry.namespace.clone();
    let key = entry.key.clone();

    let txn = ctx.db.begin().await.db_err()?;

    // Delete all translations (global + tenant) for this (namespace, key).
    i18n_translations::Entity::delete_many()
        .filter(i18n_translations::Column::Namespace.eq(&ns))
        .filter(i18n_translations::Column::Key.eq(&key))
        .exec(&txn)
        .await
        .db_err()?;

    // Delete the entry (locations cascade via FK ON DELETE CASCADE).
    i18n_entries::Entity::delete_by_id(entry_id)
        .exec(&txn)
        .await
        .db_err()?;

    txn.commit().await.db_err()?;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Delete,
        "i18n_entry",
        &format!("{ns}.{key}"),
        Some(&before),
        None::<&serde_json::Value>,
    )
    .await
    .model_err()?;

    Ok(())
}
