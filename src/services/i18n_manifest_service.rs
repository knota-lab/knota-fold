//! Manifest ingestion service — receives the extractor output and reconciles
//! `i18n_entries` + `i18n_entry_locations` accordingly. Always force-syncs the
//! zh-CN baseline translation from inline source defaults
//! (`t('Ns.key', '中文')`) — the frontend source code is the source of truth.
//!
//! Behavior contract (per `system-design/国际化.md` §13):
//!
//! 1. For every `(namespace, key)` in the manifest:
//!    - If the entry exists → update `description` (when provided),
//!      `last_seen_at = now`, and force `status = 'active'`.
//!    - Otherwise → insert a new entry with `status = 'active'`.
//! 2. Replace the entry's locations: delete all existing rows, then bulk
//!    insert the new ones.
//! 3. Any entry NOT mentioned in this manifest gets `status = 'stale'`
//!    (we never delete entries — translation history must survive).
//! 4. If an entry carries `source_text`, force-sync the global zh-CN row:
//!    - missing → INSERT; mismatched → UPDATE; identical → no-op.
//!      Tenant overrides and other locales are untouched. Frontend hardcoded
//!      Chinese is authoritative — admin edits to zh-CN will be overwritten on
//!      the next manifest upload.
//! 5. The whole operation runs in a single transaction so partial failures
//!    don't leave the manifest half-applied. Bundle revisions are bumped
//!    (and caches invalidated) once per affected `(locale, namespace)` pair
//!    after syncing.

use std::collections::{BTreeSet, HashSet};

use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, EntityTrait, QueryFilter, Statement, TransactionTrait,
};

use crate::models::_entities::{i18n_entries, i18n_entry_locations};
use crate::models::i18n_entries as entry_model;
use crate::services::{audit_service, i18n_service};
use crate::utils::error::{IntoAppError, IntoModelResult};
use crate::utils::id::generate_id;
use crate::views::audit_logs::{AuditAction, AuditContext};
use crate::views::errors::err_bad_request;
use crate::views::i18n::{
    ManifestEntryInput, ManifestUploadRequest, ManifestUploadResponse, BASE_LOCALE,
};

/// Hard cap to protect the DB. Way above any plausible real-world manifest.
const MAX_MANIFEST_ENTRIES: usize = 50_000;
const MAX_LOCATIONS_PER_ENTRY: usize = 64;
/// Mirror the extractor cap (`scripts/extract-i18n.mjs::MAX_SOURCE_TEXT_LEN`).
/// Keep them in sync — anything bigger is rejected to avoid storing entire
/// paragraphs as i18n keys.
const MAX_SOURCE_TEXT_LEN: usize = 2048;

#[tracing::instrument(skip_all)]
pub async fn apply_manifest(
    ctx: &AppContext,
    payload: &ManifestUploadRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<ManifestUploadResponse> {
    // ── Validate ────────────────────────────────────────────────────────────
    if payload.entries.is_empty() {
        return Err(err_bad_request(
            "i18n.manifest_entries_empty",
            "manifest entries 不能为空",
        ));
    }
    if payload.entries.len() > MAX_MANIFEST_ENTRIES {
        return Err(err_bad_request(
            "i18n.manifest_entries_too_many",
            format!("manifest entries 超过上限 {MAX_MANIFEST_ENTRIES}"),
        ));
    }

    // Deduplicate by (namespace, key) — extractor occasionally emits the same
    // call site under different macros. Keep the first occurrence.
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut deduped: Vec<&ManifestEntryInput> = Vec::with_capacity(payload.entries.len());
    for entry in &payload.entries {
        validate_entry(entry)?;
        if seen.insert((entry.namespace.clone(), entry.key.clone())) {
            deduped.push(entry);
        }
    }

    // ── Apply in a single transaction ───────────────────────────────────────
    let txn = ctx.db.begin().await.db_err()?;

    let mut created = 0u64;
    let mut updated = 0u64;
    let mut total_locations = 0u64;
    let mut synced_inserted = 0u64;
    let mut synced_overwritten = 0u64;
    let mut touched_ids: HashSet<uuid::Uuid> = HashSet::new();
    // (BASE_LOCALE, namespace) pairs whose bundle gained at least one inserted
    // or overwritten row — we bump their revision once after the per-entry
    // loop. `Unchanged` outcomes do NOT bump revision (no actual data change).
    let mut synced_namespaces: BTreeSet<String> = BTreeSet::new();

    for entry in &deduped {
        // 必须用 &txn，不能用 &ctx.db：事务持有池里的连接，循环里再向池
        // 申请第二条连接会和事务自身死锁，在 max_connections=1 下立刻
        // ConnectionAcquire(Timeout)，更大池子下也会让并发上传相互阻塞。
        let existing =
            entry_model::Model::find_by_namespace_key(&txn, &entry.namespace, &entry.key)
                .await
                .db_err()?;

        let now = chrono::Utc::now().fixed_offset();
        let entry_id = if let Some(row) = existing {
            let am = i18n_entries::ActiveModel {
                id: ActiveValue::Unchanged(row.id),
                description: ActiveValue::Set(
                    entry.description.clone().or(row.description.clone()),
                ),
                status: ActiveValue::Set(entry_model::STATUS_ACTIVE.to_string()),
                last_seen_at: ActiveValue::Set(now),
                ..Default::default()
            };
            am.update(&txn).await.db_err()?;
            updated += 1;
            row.id
        } else {
            let new_id = generate_id();
            let am = i18n_entries::ActiveModel {
                id: ActiveValue::Set(new_id),
                namespace: ActiveValue::Set(entry.namespace.clone()),
                key: ActiveValue::Set(entry.key.clone()),
                description: ActiveValue::Set(entry.description.clone()),
                status: ActiveValue::Set(entry_model::STATUS_ACTIVE.to_string()),
                last_seen_at: ActiveValue::Set(now),
                ..Default::default()
            };
            am.insert(&txn).await.db_err()?;
            created += 1;
            new_id
        };
        touched_ids.insert(entry_id);

        // Replace locations.
        i18n_entry_locations::Entity::delete_many()
            .filter(i18n_entry_locations::Column::EntryId.eq(entry_id))
            .exec(&txn)
            .await
            .db_err()?;

        for loc in &entry.locations {
            let am = i18n_entry_locations::ActiveModel {
                entry_id: ActiveValue::Set(entry_id),
                file_path: ActiveValue::Set(loc.file_path.clone()),
                line: ActiveValue::Set(loc.line),
                ..Default::default()
            };
            am.insert(&txn).await.db_err()?;
            total_locations += 1;
        }

        // Force-sync zh-CN baseline from inline source default
        // (`t('Ns.key', '中文')`). Frontend source is authoritative — admin
        // edits to zh-CN are overwritten on the next manifest upload.
        // Skipped silently when source_text is absent or empty.
        if let Some(source) = entry.source_text.as_deref() {
            if !source.is_empty() {
                let outcome = i18n_service::force_sync_global_in_txn(
                    &txn,
                    &entry.namespace,
                    &entry.key,
                    BASE_LOCALE,
                    source,
                )
                .await?;
                match outcome {
                    i18n_service::ForceSyncOutcome::Inserted => {
                        synced_inserted += 1;
                        synced_namespaces.insert(entry.namespace.clone());
                    }
                    i18n_service::ForceSyncOutcome::Updated => {
                        synced_overwritten += 1;
                        synced_namespaces.insert(entry.namespace.clone());
                    }
                    i18n_service::ForceSyncOutcome::Unchanged => {}
                }
            }
        }
    }

    // ── Mark stale: any active entry whose id is NOT in touched_ids ────────
    // We use a raw statement so we don't have to load every active row into
    // memory just to compute a set difference.
    let backend = txn.get_database_backend();
    let stale_result = if touched_ids.is_empty() {
        txn.execute(Statement::from_sql_and_values(
            backend,
            "UPDATE i18n_entries \
             SET status = 'stale', updated_at = CURRENT_TIMESTAMP \
             WHERE status = 'active'",
            [],
        ))
        .await
        .db_err()?
    } else {
        // Build placeholder list `($1,$2,...)`.
        let placeholders: Vec<String> =
            (1..=touched_ids.len()).map(|i| format!("${i}")).collect();
        let sql = format!(
            "UPDATE i18n_entries \
             SET status = 'stale', updated_at = CURRENT_TIMESTAMP \
             WHERE status = 'active' AND id NOT IN ({})",
            placeholders.join(",")
        );
        let values: Vec<sea_orm::Value> = touched_ids
            .iter()
            .map(|id| sea_orm::Value::Uuid(Some(Box::new(*id))))
            .collect();
        txn.execute(Statement::from_sql_and_values(backend, sql, values))
            .await
            .db_err()?
    };
    let stale = stale_result.rows_affected();

    // Bump global bundle revision once per namespace that received a sync
    // (insert OR overwrite). Cascades to existing tenant revision rows
    // automatically (see `i18n_service::bump_global_revision`).
    for ns in &synced_namespaces {
        i18n_service::bump_global_revision_pub(&txn, BASE_LOCALE, ns).await?;
    }

    txn.commit().await.db_err()?;

    // Cache invalidation must happen post-commit; otherwise readers could
    // repopulate the cache from the pre-commit snapshot.
    for ns in &synced_namespaces {
        i18n_service::invalidate_global_bundle_cache(ctx, BASE_LOCALE, ns).await;
    }

    // ── Audit ──────────────────────────────────────────────────────────────
    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Update,
        "i18n_manifest",
        payload.commit_sha.as_deref().unwrap_or("manifest"),
        None::<&serde_json::Value>,
        Some(&serde_json::json!({
            "createdEntries": created,
            "updatedEntries": updated,
            "staleEntries": stale,
            "totalLocations": total_locations,
            "syncedInserted": synced_inserted,
            "syncedOverwritten": synced_overwritten,
            "syncedNamespaces": synced_namespaces.iter().collect::<Vec<_>>(),
            "generatedAt": payload.generated_at,
            "commitSha": payload.commit_sha,
        })),
    )
    .await
    .model_err()?;

    Ok(ManifestUploadResponse {
        created_entries: created,
        updated_entries: updated,
        stale_entries: stale,
        total_locations,
        synced_inserted,
        synced_overwritten,
    })
}

// ── Validation ──────────────────────────────────────────────────────────────

fn validate_entry(entry: &ManifestEntryInput) -> loco_rs::Result<()> {
    if entry.namespace.is_empty() || entry.namespace.len() > 64 {
        return Err(err_bad_request(
            "i18n.manifest_namespace_length_invalid",
            format!("namespace 长度非法: '{}'", entry.namespace),
        ));
    }
    if !entry
        .namespace
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    {
        return Err(err_bad_request(
            "i18n.manifest_namespace_chars_invalid",
            format!("namespace 字符非法: '{}'", entry.namespace),
        ));
    }
    if entry.key.is_empty() || entry.key.len() > 256 {
        return Err(err_bad_request(
            "i18n.manifest_key_length_invalid",
            format!("key 长度非法: '{}.{}'", entry.namespace, entry.key),
        ));
    }
    if !entry
        .key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(err_bad_request(
            "i18n.manifest_key_chars_invalid",
            format!("key 字符非法: '{}.{}'", entry.namespace, entry.key),
        ));
    }
    if entry.locations.len() > MAX_LOCATIONS_PER_ENTRY {
        return Err(err_bad_request(
            "i18n.manifest_locations_too_many",
            format!(
                "'{}.{}' 的 locations 数超过上限 {MAX_LOCATIONS_PER_ENTRY}",
                entry.namespace, entry.key
            ),
        ));
    }
    for loc in &entry.locations {
        if loc.file_path.is_empty() || loc.file_path.len() > 512 {
            return Err(err_bad_request(
                "i18n.manifest_filepath_length_invalid",
                format!("'{}.{}' file_path 长度非法", entry.namespace, entry.key),
            ));
        }
        if loc.line < 0 {
            return Err(err_bad_request(
                "i18n.manifest_line_negative",
                format!("'{}.{}' line 不能为负数", entry.namespace, entry.key),
            ));
        }
    }
    if let Some(src) = &entry.source_text {
        if src.len() > MAX_SOURCE_TEXT_LEN {
            return Err(err_bad_request(
                "i18n.manifest_source_text_too_long",
                format!(
                    "'{}.{}' source_text 长度超过上限 {MAX_SOURCE_TEXT_LEN}",
                    entry.namespace, entry.key
                ),
            ));
        }
    }
    Ok(())
}
