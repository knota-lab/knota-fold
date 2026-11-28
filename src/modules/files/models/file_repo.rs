use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::models::_entities::files;
use crate::utils::error::IntoAppError;

/// # Errors
///
/// Returns a database error if the query fails.
pub async fn find_active_by_fast_hash_and_size(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    fast_hash: &str,
    size: i64,
) -> loco_rs::Result<Vec<files::Model>> {
    files::Entity::find()
        .filter(files::Column::TenantId.eq(tenant_id))
        .filter(files::Column::ContentHashFast.eq(fast_hash))
        .filter(files::Column::Size.eq(size))
        .filter(files::Column::DeletedAt.is_null())
        .all(db)
        .await
        .db_err()
}

/// Look up a same-tenant file matching the given full hash and size,
/// **including soft-deleted rows**.
///
/// Used by the instant-upload (秒传) path: when an active candidate is
/// missing we may revive a soft-deleted row instead of forcing a re-upload.
///
/// Ordering: active rows (`deleted_at IS NULL`) win over soft-deleted ones,
/// and within each group the newest (`created_at DESC`) is preferred.
///
/// # Errors
///
/// Returns a database error if the query fails.
pub async fn find_any_by_hash_and_size(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    content_hash: &str,
    size: i64,
) -> loco_rs::Result<Option<files::Model>> {
    let mut rows = files::Entity::find()
        .filter(files::Column::TenantId.eq(tenant_id))
        .filter(files::Column::ContentHash.eq(content_hash))
        .filter(files::Column::Size.eq(size))
        .order_by_desc(files::Column::CreatedAt)
        .all(db)
        .await
        .db_err()?;

    // Stable in-memory partition: active rows first, then soft-deleted.
    rows.sort_by_key(|row| i32::from(row.deleted_at.is_some()));
    Ok(rows.into_iter().next())
}
