//! File reference service — Wave 5 D2.
//!
//! Owns the lifecycle of `file_references` rows: attach (idempotent +
//! revive-on-soft-deleted-key), detach (soft), bulk detach by resource
//! (used by business-table cascade), and the read paths consumed by UI
//! ("X 处使用" badges + business-row attachment lists) and by the
//! purge job ("file has zero active references").
//!
//! ## Invariants
//!
//! 1. **Idempotent attach.** Calling `attach` twice with the same
//!    `(tenant, file, resource_type, resource_id, field_name)` returns
//!    the same active row. The DB enforces this via the partial unique
//!    index `uq_file_refs_active` (WHERE deleted_at IS NULL).
//!
//! 2. **Revive on collision.** If a soft-deleted row exists for the
//!    same key, attach clears `deleted_at` and refreshes `created_by`
//!    + `display_name` instead of inserting. This preserves the audit
//!      chain (one row per logical attachment, with detach/reattach
//!      history readable via `audit_logs`) and keeps the partial unique
//!      index honest.
//!
//! 3. **Soft detach only.** `detach` writes `deleted_at = now()`. Rows
//!    are never hard-deleted, so the audit trail remains queryable.
//!    The purge job is what eventually removes the underlying file
//!    (D5), not this service.
//!
//! 4. **Tenant isolation.** Every read/write filters on `tenant_id` to
//!    prevent cross-tenant attach via forged `file_id` / `resource_id`.
//!
//! 5. **Audit hook stub.** `file_audit_service::log_reference` /
//!    `log_dereference` are still `todo!()` (Wave-2 skeleton). D2
//!    intentionally does **not** call them — adding hooks now would
//!    panic in production. The hooks will be wired in a follow-up
//!    audit-completion wave; this service's contract already accepts
//!    `&AuditContext` so the call sites do not have to change.
//!
//! 6. **Concurrency.** Attach uses an explicit transaction so the
//!    "lookup existing → insert or revive" sequence is atomic against
//!    a racing attach for the same key. The partial unique index is
//!    the ultimate backstop: a racing winner gets the row, the loser
//!    sees a unique-violation, retries the lookup, and returns the
//!    existing active row.

use axum::http::StatusCode;
use chrono::Utc;
use loco_rs::controller::ErrorDetail;
use loco_rs::prelude::model::query;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection,
    DbErr, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, RuntimeErr,
    TransactionTrait,
};
use std::collections::HashMap;
use uuid::Uuid;

use crate::models::_entities::{file_references, files};
use crate::services::resource_types::ResourceType;
use crate::utils::error::{IntoAppError, OptionErrInto};
use crate::utils::id::generate_id;
use crate::views::audit_logs::AuditContext;
use crate::views::file_references::{
    FileReferenceResponse, FileReferenceWithFileResponse,
};
use crate::views::files::FileResponse;
use crate::views::pagination::PaginatedResponse;

/// Caller-supplied attach payload. The controller layer is responsible
/// for translating the wire DTO (`AttachReferenceRequest`) into this
/// strongly-typed shape after validating `resource_type`.
#[derive(Debug, Clone)]
pub struct AttachRequest {
    pub file_id: Uuid,
    pub resource_type: ResourceType,
    pub resource_id: String,
    /// Discriminator for "same file under multiple form fields of the
    /// same business row". Empty string is the default (single slot).
    pub field_name: String,
    /// UI snapshot label. `None` falls back to the file's `name` at
    /// attach time so consumers always have something to render.
    pub display_name: Option<String>,
}

/// Sentinel value for [`AttachRequest::resource_id`] meaning "use the
/// new row's own primary key". Resolved inside the service before the
/// existing-row lookup so the caller never has to round-trip through
/// the DB to learn an id it just wants to assign.
///
/// Currently only used by the standalone-upload flow under
/// [`ResourceType::SystemAttachment`]: each upload event needs its own
/// `(file_id, resource_type, resource_id)` tuple so the partial unique
/// index `uq_file_refs_active` does not collapse repeated uploads of
/// the same physical file into a single row. Using the row's own id
/// guarantees uniqueness without forcing the caller to generate a
/// stable business id it has no other use for.
pub const SELF_RESOURCE_ID_SENTINEL: &str = "$self";

/// Build a default self-referencing [`AttachRequest`] for upload flows
/// where the caller did not supply an explicit `attachTo` payload.
///
/// Wave 5 D7b: a standalone upload (admin /files page, generic API
/// client without business context) must still produce a
/// `file_references` row so the cleanup task does not garbage-collect
/// the underlying file. Each such upload becomes one
/// `system:attachment` row whose `resource_id` resolves to the row's
/// own primary key (see [`SELF_RESOURCE_ID_SENTINEL`]). `file_id` is a
/// placeholder; the upload service overwrites it after the file row
/// is materialized.
pub fn default_self_attach() -> AttachRequest {
    AttachRequest {
        file_id: Uuid::nil(),
        resource_type: ResourceType::SystemAttachment,
        resource_id: SELF_RESOURCE_ID_SENTINEL.to_owned(),
        field_name: String::new(),
        display_name: None,
    }
}

// ---------------------------------------------------------------------------
// attach
// ---------------------------------------------------------------------------

/// Attach `file_id` to `(resource_type, resource_id[, field_name])`.
///
/// Idempotent + revive-aware:
/// - Existing active row → return as-is (200, no DB write).
/// - Existing soft-deleted row for the same key → revive (clear
///   `deleted_at`, refresh `created_by` + `display_name`).
/// - No existing row → insert.
///
/// Validates that the file belongs to `audit_ctx.tenant_id` and is not
/// hard-purged (soft-deleted is OK; UI may attach to a file currently
/// in grace, the restore + attach order is the caller's choice).
///
/// # Errors
///
/// - [`Error::NotFound`] if `file_id` does not exist for this tenant.
/// - [`Error::Any`] for unexpected DB errors.
#[tracing::instrument(skip_all, fields(file_id = %req.file_id, resource_type = %req.resource_type, resource_id = %req.resource_id))]
pub async fn attach(
    db: &DatabaseConnection,
    audit_ctx: &AuditContext,
    req: AttachRequest,
) -> Result<file_references::Model> {
    // Standalone-txn entry point: opens its own transaction so we can
    // recover from a partial-unique race (rollback + re-read the winner)
    // without trampling on a caller's enclosing txn. Used by the HTTP
    // attach controllers.
    let tenant_id = audit_ctx.tenant_id;
    let user_id = audit_ctx.user_id.ok_or_else(|| {
        Error::CustomError(
            StatusCode::FORBIDDEN,
            ErrorDetail::new("file.auth_required", "操作需要已认证用户"),
        )
    })?;

    let txn = db.begin().await.db_err()?;

    match attach_in_txn_inner(&txn, tenant_id, user_id, &req).await {
        Ok(model) => {
            txn.commit().await.db_err()?;
            Ok(model)
        }
        Err(AttachError::UniqueRace) => {
            // Lost the race against a concurrent attach for the same key.
            // Roll back and re-read; the winning row is now visible and
            // active. We treat this as idempotent success.
            txn.rollback().await.db_err()?;
            find_by_key(
                db,
                tenant_id,
                req.file_id,
                req.resource_type,
                &req.resource_id,
                &req.field_name,
            )
            .await?
            .ok_or_else(|| {
                Error::CustomError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetail::new(
                        "file_ref.unique_race_retry_failed",
                        "unique-violation on attach but no active row found on retry",
                    ),
                )
            })
        }
        Err(AttachError::Loco(e)) => {
            let _ = txn.rollback().await;
            Err(e)
        }
    }
}

/// Same as [`attach`] but runs inside the caller's existing transaction.
///
/// **Use this when attach must be atomic with another write** — e.g. a
/// freshly-uploaded file row + its initial attachment must succeed or
/// fail together, so a botched attach rolls back the upload.
///
/// Differs from [`attach`] in one important way: a unique-violation
/// (concurrent attach race) is propagated as an error rather than being
/// recovered from, because we cannot rollback a caller-owned txn
/// without destroying the caller's other writes. In the upload path
/// this is fine — a brand-new file id cannot collide with anything.
///
/// # Errors
///
/// - [`Error::NotFound`] if `file_id` does not exist for this tenant.
/// - [`Error::Any`] for unexpected DB errors, including unique-violation.
#[tracing::instrument(skip_all, fields(file_id = %req.file_id, resource_type = %req.resource_type, resource_id = %req.resource_id))]
pub async fn attach_in_txn<C: ConnectionTrait>(
    txn: &C,
    audit_ctx: &AuditContext,
    req: AttachRequest,
) -> Result<file_references::Model> {
    let tenant_id = audit_ctx.tenant_id;
    let user_id = audit_ctx.user_id.ok_or_else(|| {
        Error::CustomError(
            StatusCode::FORBIDDEN,
            ErrorDetail::new("file.auth_required", "操作需要已认证用户"),
        )
    })?;
    match attach_in_txn_inner(txn, tenant_id, user_id, &req).await {
        Ok(m) => Ok(m),
        Err(AttachError::UniqueRace) => Err(Error::CustomError(
            StatusCode::CONFLICT,
            ErrorDetail::new(
                "file_ref.concurrent_attach",
                "file reference already attached by a concurrent transaction",
            ),
        )),
        Err(AttachError::Loco(e)) => Err(e),
    }
}

/// Internal error type lets the standalone wrapper distinguish a
/// recoverable race from a real failure without parsing strings twice.
enum AttachError {
    UniqueRace,
    Loco(Error),
}

impl From<Error> for AttachError {
    fn from(e: Error) -> Self {
        Self::Loco(e)
    }
}

/// Pure in-txn body shared by [`attach`] and [`attach_in_txn`].
async fn attach_in_txn_inner<C: ConnectionTrait>(
    txn: &C,
    tenant_id: Uuid,
    user_id: Uuid,
    req: &AttachRequest,
) -> std::result::Result<file_references::Model, AttachError> {
    // Validate file belongs to tenant. Includes soft-deleted files so
    // attach-during-grace remains legal; hard-purged files are gone
    // from the table entirely and surface as NotFound here.
    let file = files::Entity::find_by_id(req.file_id)
        .filter(files::Column::TenantId.eq(tenant_id))
        .one(txn)
        .await
        .map_err(|e| AttachError::Loco(Error::Any(e.into())))?
        .ok_or(AttachError::Loco(crate::views::errors::err_not_found(
            "file.not_found",
            "文件不存在",
        )))?;

    let display = req
        .display_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| Some(file.name.clone()));

    let now = Utc::now().fixed_offset();

    // Resolve the self-reference sentinel before the unique-key lookup
    // so revive / dedup logic always sees the final stored value. With
    // the sentinel the row id IS the resource_id, so we pre-generate
    // it here instead of letting `Default::generate_id()` fire inside
    // the ActiveModel build below. A non-sentinel resource_id is left
    // untouched and a fresh row id is generated normally.
    let (row_id, resource_id_resolved): (Uuid, String) =
        if req.resource_id == SELF_RESOURCE_ID_SENTINEL {
            let id = generate_id();
            (id, id.to_string())
        } else {
            (generate_id(), req.resource_id.clone())
        };

    // Look up any existing row (active or soft-deleted) for this key.
    // The partial unique index only covers active rows, so a soft-deleted
    // row plus a fresh insert would NOT collide at the index level —
    // we rely on this explicit lookup to revive instead of duplicating.
    //
    // For self-referenced rows the lookup is effectively guaranteed to
    // miss (resource_id == row_id which we just generated), so each
    // standalone upload becomes a fresh row. That is the intended
    // semantic: "user uploaded the same physical file twice" must
    // surface as two distinct attachment events on the admin /files
    // page, not silently dedupe to one.
    let existing = find_by_key(
        txn,
        tenant_id,
        req.file_id,
        req.resource_type,
        &resource_id_resolved,
        &req.field_name,
    )
    .await
    .map_err(AttachError::Loco)?;

    if let Some(row) = existing {
        if row.deleted_at.is_none() {
            // Already attached. Idempotent return; no audit, nothing changed.
            return Ok(row);
        }

        // Revive the soft-deleted row.
        let mut am: file_references::ActiveModel = row.into();
        am.deleted_at = ActiveValue::Set(None);
        am.created_by = ActiveValue::Set(user_id);
        am.created_at = ActiveValue::Set(now);
        am.display_name = ActiveValue::Set(display);
        let revived = am
            .update(txn)
            .await
            .map_err(|e| AttachError::Loco(Error::Any(e.into())))?;
        return Ok(revived);
    }

    // No existing row. Insert. The partial unique index will reject
    // races where another transaction inserted the same active key
    // between our lookup and insert — caller decides how to recover.
    let am = file_references::ActiveModel {
        id: ActiveValue::Set(row_id),
        tenant_id: ActiveValue::Set(tenant_id),
        file_id: ActiveValue::Set(req.file_id),
        resource_type: ActiveValue::Set(req.resource_type.as_str().to_owned()),
        resource_id: ActiveValue::Set(resource_id_resolved),
        field_name: ActiveValue::Set(req.field_name.clone()),
        display_name: ActiveValue::Set(display),
        created_by: ActiveValue::Set(user_id),
        created_at: ActiveValue::Set(now),
        deleted_at: ActiveValue::Set(None),
    };

    match am.insert(txn).await {
        Ok(model) => Ok(model),
        Err(e) if is_unique_violation(&e) => Err(AttachError::UniqueRace),
        Err(e) => Err(AttachError::Loco(Error::Any(e.into()))),
    }
}

// ---------------------------------------------------------------------------
// detach (single)
// ---------------------------------------------------------------------------

/// Soft-detach a single reference by id. Idempotent: detaching an
/// already-deleted row is a no-op success.
///
/// # Errors
///
/// - [`Error::NotFound`] if the row does not exist for this tenant.
/// - [`Error::Any`] for unexpected DB errors.
#[tracing::instrument(skip_all, fields(reference_id = %reference_id))]
pub async fn detach(
    db: &DatabaseConnection,
    audit_ctx: &AuditContext,
    reference_id: Uuid,
) -> Result<()> {
    let tenant_id = audit_ctx.tenant_id;

    let row = file_references::Entity::find_by_id(reference_id)
        .filter(file_references::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    if row.deleted_at.is_some() {
        // Already detached. Idempotent.
        return Ok(());
    }

    let mut am: file_references::ActiveModel = row.into();
    am.deleted_at = ActiveValue::Set(Some(Utc::now().fixed_offset()));
    am.update(db).await.db_err()?;

    let _ = audit_ctx; // audit hook deferred (file_audit_service Wave-2 todo!())
    Ok(())
}

// ---------------------------------------------------------------------------
// detach_by_resource (cascade)
// ---------------------------------------------------------------------------

/// Soft-detach **all active** references pointing at a given business
/// row. Returns the number of rows transitioned from active → deleted.
///
/// This is the cascade entry point for business-side soft-delete: a
/// caller deleting a `dict_items` row passes its own transaction handle
/// in so the detach + business soft-delete commit atomically. Generic
/// over [`ConnectionTrait`] for exactly that reason.
///
/// Already-deleted rows are skipped (the WHERE deleted_at IS NULL
/// filter is part of the update predicate).
///
/// # Errors
///
/// - [`Error::Any`] for unexpected DB errors.
#[tracing::instrument(skip_all, fields(resource_type = %resource_type, resource_id = %resource_id))]
pub async fn detach_by_resource<C: ConnectionTrait>(
    db: &C,
    audit_ctx: &AuditContext,
    resource_type: ResourceType,
    resource_id: &str,
) -> Result<u64> {
    let tenant_id = audit_ctx.tenant_id;
    let now = Utc::now().fixed_offset();

    let res = file_references::Entity::update_many()
        .col_expr(
            file_references::Column::DeletedAt,
            sea_orm::sea_query::Expr::value(now),
        )
        .filter(file_references::Column::TenantId.eq(tenant_id))
        .filter(file_references::Column::ResourceType.eq(resource_type.as_str()))
        .filter(file_references::Column::ResourceId.eq(resource_id))
        .filter(file_references::Column::DeletedAt.is_null())
        .exec(db)
        .await
        .db_err()?;

    let _ = audit_ctx; // per-row audit deferred; aggregate logging will live with file_audit_service
    Ok(res.rows_affected)
}

// ---------------------------------------------------------------------------
// reads
// ---------------------------------------------------------------------------

/// Number of **active** references targeting `file_id`. Used by the
/// purge job to decide whether a soft-deleted file is safe to hard
/// delete. O(1) on `idx_file_refs_active_file` (partial index).
///
/// # Errors
///
/// - [`Error::Any`] for unexpected DB errors.
#[tracing::instrument(skip_all, fields(file_id = %file_id))]
pub async fn count_active<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    file_id: Uuid,
) -> Result<u64> {
    file_references::Entity::find()
        .filter(file_references::Column::TenantId.eq(tenant_id))
        .filter(file_references::Column::FileId.eq(file_id))
        .filter(file_references::Column::DeletedAt.is_null())
        .count(db)
        .await
        .db_err()
}

/// All active references for a given file (UI: "X 处使用" detail
/// drawer). Ordered newest-first.
///
/// # Errors
///
/// - [`Error::Any`] for unexpected DB errors.
#[tracing::instrument(skip_all, fields(file_id = %file_id))]
pub async fn list_by_file(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    file_id: Uuid,
) -> Result<Vec<file_references::Model>> {
    file_references::Entity::find()
        .filter(file_references::Column::TenantId.eq(tenant_id))
        .filter(file_references::Column::FileId.eq(file_id))
        .filter(file_references::Column::DeletedAt.is_null())
        .order_by_desc(file_references::Column::CreatedAt)
        .all(db)
        .await
        .db_err()
}

/// All active references attached to a given business row (business
/// detail page: "this row's attachments"). Ordered newest-first.
///
/// # Errors
///
/// - [`Error::Any`] for unexpected DB errors.
#[tracing::instrument(skip_all, fields(resource_type = %resource_type, resource_id = %resource_id))]
pub async fn list_by_resource<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    resource_type: ResourceType,
    resource_id: &str,
) -> Result<Vec<file_references::Model>> {
    file_references::Entity::find()
        .filter(file_references::Column::TenantId.eq(tenant_id))
        .filter(file_references::Column::ResourceType.eq(resource_type.as_str()))
        .filter(file_references::Column::ResourceId.eq(resource_id))
        .filter(file_references::Column::DeletedAt.is_null())
        .order_by_desc(file_references::Column::CreatedAt)
        .all(db)
        .await
        .db_err()
}

/// Tenant-wide paginated list of active file references, joined with
/// the underlying `files` row for UI consumers (admin attachments
/// page). Optional `resource_type_filter` narrows to a single business
/// kind; `None` returns all kinds.
///
/// Two queries: paginate `file_references` first, then bulk-load all
/// referenced `files` for the page in one `WHERE id IN (..)` call. We
/// avoid `find_also_related` because the entity has no compile-time
/// relation defined and adding one churns the generated entity file.
///
/// Soft-deleted references are excluded (purge-friendly).
///
/// # Errors
///
/// - [`Error::Any`] for unexpected DB errors.
#[tracing::instrument(skip_all, fields(tenant_id = %tenant_id))]
pub async fn list_for_tenant_paginated(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    resource_type_filter: Option<ResourceType>,
    pagination: &query::PaginationQuery,
) -> Result<PaginatedResponse<FileReferenceWithFileResponse>> {
    // Step 1: paginate references.
    let mut base = file_references::Entity::find()
        .filter(file_references::Column::TenantId.eq(tenant_id))
        .filter(file_references::Column::DeletedAt.is_null());
    if let Some(rt) = resource_type_filter {
        base = base.filter(file_references::Column::ResourceType.eq(rt.as_str()));
    }
    let base = base.order_by_desc(file_references::Column::CreatedAt);

    let page_response = query::paginate(db, base, None, pagination).await?;

    // Step 2: bulk-load files for this page (deleted files included so
    // the UI can still show "this file was hard-deleted but a stale
    // reference is here" if it ever happens; the active-only filter is
    // on references, not on files).
    let file_ids: Vec<Uuid> = page_response
        .page
        .iter()
        .map(|r| r.file_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let files_map: HashMap<Uuid, files::Model> = if file_ids.is_empty() {
        HashMap::new()
    } else {
        files::Entity::find()
            .filter(files::Column::TenantId.eq(tenant_id))
            .filter(files::Column::Id.is_in(file_ids))
            .all(db)
            .await
            .db_err()?
            .into_iter()
            .map(|m| (m.id, m))
            .collect()
    };

    // Step 3: zip into the joined DTO via PaginatedResponse mapper.
    Ok(PaginatedResponse::from_page_response(
        &page_response,
        pagination,
        |model| FileReferenceWithFileResponse {
            reference: FileReferenceResponse::from(model.clone()),
            file: files_map
                .get(&model.file_id)
                .cloned()
                .map(FileResponse::from),
        },
    ))
}

// ---------------------------------------------------------------------------
// internals
// ---------------------------------------------------------------------------

/// Look up by the logical key (tenant, file, resource, field). Returns
/// either the active or the soft-deleted row for that key — at most
/// one of each can exist by construction (partial unique index covers
/// active; soft-deleted rows are guaranteed unique because we only
/// ever revive, never insert a duplicate alongside one).
///
/// Returns the active row if both somehow coexist (defensive).
async fn find_by_key<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    file_id: Uuid,
    resource_type: ResourceType,
    resource_id: &str,
    field_name: &str,
) -> Result<Option<file_references::Model>> {
    let mut rows = file_references::Entity::find()
        .filter(file_references::Column::TenantId.eq(tenant_id))
        .filter(file_references::Column::FileId.eq(file_id))
        .filter(file_references::Column::ResourceType.eq(resource_type.as_str()))
        .filter(file_references::Column::ResourceId.eq(resource_id))
        .filter(file_references::Column::FieldName.eq(field_name))
        .all(db)
        .await
        .db_err()?;

    // Prefer active; otherwise the soft-deleted row.
    rows.sort_by_key(|r| r.deleted_at.is_some());
    Ok(rows.into_iter().next())
}

/// Detect a unique-constraint violation across PG / SQLite / MySQL
/// without coupling to one driver's error code. The caller treats this
/// as "race on the partial unique index, retry the read".
fn is_unique_violation(e: &DbErr) -> bool {
    let msg_contains = |needle: &str| -> bool {
        match e {
            DbErr::Exec(RuntimeErr::SqlxError(err))
            | DbErr::Query(RuntimeErr::SqlxError(err)) => {
                err.to_string().to_ascii_lowercase().contains(needle)
            }
            _ => format!("{e}").to_ascii_lowercase().contains(needle),
        }
    };
    // "unique" covers PG ("duplicate key value violates unique constraint"),
    // SQLite ("UNIQUE constraint failed"), and MySQL ("Duplicate entry").
    msg_contains("unique") || msg_contains("duplicate")
}
