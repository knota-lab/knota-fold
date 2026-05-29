//! Single-record CRUD for global and tenant translations, plus internal helpers.

use std::collections::HashSet;

use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, TransactionTrait,
};
use uuid::Uuid;

use crate::models::_entities::i18n_translations;
use crate::models::i18n_translations as trans_model;
use crate::services::audit_service;
use crate::utils::error::{IntoAppError, IntoModelResult, OptionErrInto};
use crate::views::audit_logs::{AuditAction, AuditContext};
use crate::views::errors::err_bad_request;
use crate::views::i18n::{
    ImportEntry, ImportResponse, TranslationResponse, UpdateTranslationRequest,
    UpsertGlobalTranslationRequest, UpsertTenantOverrideRequest, IMPORT_MAX_ENTRIES,
};

use super::i18n_bundle::{
    bump_global_revision, bump_tenant_revision, invalidate_bundle_cache,
};
use super::i18n_validation::{
    ensure_tenant_namespace_allowed, load_tenant_namespace_policy, validate_key,
    validate_locale, validate_namespace,
};

// ── Translation upsert (global) ─────────────────────────────────────────────

#[tracing::instrument(skip_all)]
pub async fn upsert_global_translation(
    ctx: &AppContext,
    user_id: Uuid,
    params: &UpsertGlobalTranslationRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<TranslationResponse> {
    validate_namespace(&params.namespace)?;
    validate_key(&params.key)?;
    validate_locale(&params.locale)?;
    if params.value.is_empty() {
        return Err(err_bad_request("i18n.value_empty", "value 不能为空"));
    }

    let txn = ctx.db.begin().await.db_err()?;

    let existing = trans_model::Model::find_global(
        &txn,
        &params.namespace,
        &params.key,
        &params.locale,
    )
    .await
    .db_err()?;

    let before = existing
        .as_ref()
        .map(|m| serde_json::json!({ "value": m.value }));

    let model = if let Some(row) = existing {
        let am = i18n_translations::ActiveModel {
            id: ActiveValue::Unchanged(row.id),
            value: ActiveValue::Set(params.value.clone()),
            updated_by: ActiveValue::Set(Some(user_id)),
            ..Default::default()
        };
        am.update(&txn).await.db_err()?
    } else {
        let am = i18n_translations::ActiveModel {
            namespace: ActiveValue::Set(params.namespace.clone()),
            key: ActiveValue::Set(params.key.clone()),
            locale: ActiveValue::Set(params.locale.clone()),
            value: ActiveValue::Set(params.value.clone()),
            scope: ActiveValue::Set(trans_model::SCOPE_GLOBAL.to_string()),
            tenant_id: ActiveValue::Set(None),
            updated_by: ActiveValue::Set(Some(user_id)),
            ..Default::default()
        };
        am.insert(&txn).await.db_err()?
    };

    bump_global_revision(&txn, &params.locale, &params.namespace).await?;
    txn.commit().await.db_err()?;

    invalidate_bundle_cache(ctx, &params.locale, &params.namespace, None).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        if before.is_some() {
            AuditAction::Update
        } else {
            AuditAction::Create
        },
        "i18n_translation_global",
        &format!("{}.{}::{}", params.namespace, params.key, params.locale),
        before.as_ref(),
        Some(&serde_json::json!({
            "namespace": model.namespace,
            "key": model.key,
            "locale": model.locale,
            "value": model.value,
        })),
    )
    .await
    .model_err()?;

    Ok(TranslationResponse::from(&model))
}

#[tracing::instrument(skip_all)]
pub async fn update_global_translation_by_id(
    ctx: &AppContext,
    id: Uuid,
    user_id: Uuid,
    params: &UpdateTranslationRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<TranslationResponse> {
    let existing = i18n_translations::Entity::find_by_id(id)
        .one(&ctx.db)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    if existing.tenant_id.is_some() {
        return Err(err_bad_request(
            "i18n.global_update_only",
            "该接口仅可更新全局翻译，请使用租户接口",
        ));
    }

    let before = serde_json::json!({ "value": existing.value });
    let (locale, namespace) = (existing.locale.clone(), existing.namespace.clone());

    let txn = ctx.db.begin().await.db_err()?;
    let am = i18n_translations::ActiveModel {
        id: ActiveValue::Unchanged(existing.id),
        value: ActiveValue::Set(params.value.clone()),
        updated_by: ActiveValue::Set(Some(user_id)),
        ..Default::default()
    };
    let model = am.update(&txn).await.db_err()?;
    bump_global_revision(&txn, &locale, &namespace).await?;
    txn.commit().await.db_err()?;

    invalidate_bundle_cache(ctx, &locale, &namespace, None).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Update,
        "i18n_translation_global",
        &format!("{namespace}.{}::{locale}", model.key),
        Some(&before),
        Some(&serde_json::json!({ "value": model.value })),
    )
    .await
    .model_err()?;

    Ok(TranslationResponse::from(&model))
}

#[tracing::instrument(skip_all)]
pub async fn delete_global_translation_by_id(
    ctx: &AppContext,
    id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    let existing = i18n_translations::Entity::find_by_id(id)
        .one(&ctx.db)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;
    if existing.tenant_id.is_some() {
        return Err(err_bad_request(
            "i18n.global_delete_only",
            "该接口仅可删除全局翻译",
        ));
    }
    let before = serde_json::json!({
        "namespace": existing.namespace,
        "key": existing.key,
        "locale": existing.locale,
        "value": existing.value,
    });
    let (locale, namespace) = (existing.locale.clone(), existing.namespace.clone());

    let txn = ctx.db.begin().await.db_err()?;
    i18n_translations::Entity::delete_by_id(existing.id)
        .exec(&txn)
        .await
        .db_err()?;
    bump_global_revision(&txn, &locale, &namespace).await?;
    txn.commit().await.db_err()?;

    invalidate_bundle_cache(ctx, &locale, &namespace, None).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Delete,
        "i18n_translation_global",
        &format!("{namespace}.{}::{locale}", existing.key),
        Some(&before),
        None::<&serde_json::Value>,
    )
    .await
    .model_err()?;

    Ok(())
}

// ── Translation upsert (tenant) ─────────────────────────────────────────────

#[tracing::instrument(skip_all)]
pub async fn upsert_tenant_override(
    ctx: &AppContext,
    tenant_id: Uuid,
    user_id: Uuid,
    params: &UpsertTenantOverrideRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<TranslationResponse> {
    validate_namespace(&params.namespace)?;
    let (own_prefix, readable_namespaces) =
        load_tenant_namespace_policy(&ctx.db, tenant_id).await?;
    ensure_tenant_namespace_allowed(
        &params.namespace,
        &own_prefix,
        &readable_namespaces,
    )?;
    validate_key(&params.key)?;
    validate_locale(&params.locale)?;
    if params.value.is_empty() {
        return Err(err_bad_request("i18n.value_empty", "value 不能为空"));
    }

    let txn = ctx.db.begin().await.db_err()?;
    let existing = trans_model::Model::find_tenant(
        &txn,
        &params.namespace,
        &params.key,
        &params.locale,
        tenant_id,
    )
    .await
    .db_err()?;

    let before = existing
        .as_ref()
        .map(|m| serde_json::json!({ "value": m.value }));

    let model = if let Some(row) = existing {
        let am = i18n_translations::ActiveModel {
            id: ActiveValue::Unchanged(row.id),
            value: ActiveValue::Set(params.value.clone()),
            updated_by: ActiveValue::Set(Some(user_id)),
            ..Default::default()
        };
        am.update(&txn).await.db_err()?
    } else {
        let am = i18n_translations::ActiveModel {
            namespace: ActiveValue::Set(params.namespace.clone()),
            key: ActiveValue::Set(params.key.clone()),
            locale: ActiveValue::Set(params.locale.clone()),
            value: ActiveValue::Set(params.value.clone()),
            scope: ActiveValue::Set(trans_model::SCOPE_TENANT.to_string()),
            tenant_id: ActiveValue::Set(Some(tenant_id)),
            updated_by: ActiveValue::Set(Some(user_id)),
            ..Default::default()
        };
        am.insert(&txn).await.db_err()?
    };

    bump_tenant_revision(&txn, &params.locale, &params.namespace, tenant_id).await?;
    txn.commit().await.db_err()?;

    invalidate_bundle_cache(ctx, &params.locale, &params.namespace, Some(tenant_id))
        .await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        if before.is_some() {
            AuditAction::Update
        } else {
            AuditAction::Create
        },
        "i18n_translation_tenant",
        &format!(
            "{}.{}::{}::{}",
            params.namespace, params.key, params.locale, tenant_id
        ),
        before.as_ref(),
        Some(&serde_json::json!({
            "namespace": model.namespace,
            "key": model.key,
            "locale": model.locale,
            "value": model.value,
            "tenantId": tenant_id.to_string(),
        })),
    )
    .await
    .model_err()?;

    Ok(TranslationResponse::from(&model))
}

#[tracing::instrument(skip_all)]
pub async fn delete_tenant_override_by_id(
    ctx: &AppContext,
    id: Uuid,
    tenant_id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    let existing = i18n_translations::Entity::find_by_id(id)
        .one(&ctx.db)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    if existing.tenant_id != Some(tenant_id) {
        // Hide existence across tenants.
        return Err(crate::views::errors::err_not_found(
            "i18n.not_found",
            "翻译条目不存在",
        ));
    }

    let before = serde_json::json!({
        "namespace": existing.namespace,
        "key": existing.key,
        "locale": existing.locale,
        "value": existing.value,
    });
    let (locale, namespace) = (existing.locale.clone(), existing.namespace.clone());

    let txn = ctx.db.begin().await.db_err()?;
    i18n_translations::Entity::delete_by_id(existing.id)
        .exec(&txn)
        .await
        .db_err()?;
    bump_tenant_revision(&txn, &locale, &namespace, tenant_id).await?;
    txn.commit().await.db_err()?;

    invalidate_bundle_cache(ctx, &locale, &namespace, Some(tenant_id)).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Delete,
        "i18n_translation_tenant",
        &format!("{namespace}.{}::{locale}::{tenant_id}", existing.key),
        Some(&before),
        None::<&serde_json::Value>,
    )
    .await
    .model_err()?;

    Ok(())
}

/// Delete a tenant override row identified by its
/// `(namespace, key, locale)` natural key.
///
/// The tenant-admin UI lets users "reset to global" a single cell. Cells in
/// the editor are addressed by their composite key (the override row id is
/// not surfaced to the client), so this function looks the row up by that
/// triple within the caller's tenant scope.
///
/// Behavior:
/// - If no matching override row exists, returns `Ok(false)` (idempotent —
///   the cell is already inherited, which is the desired post-condition).
/// - On delete, bumps the per-(locale, namespace, tenant) bundle revision
///   and invalidates the in-process bundle cache, identical to the by-id
///   delete path, so cached bundles flip back to the inherited value.
/// - Audit log records the deleted row's `before` snapshot.
#[tracing::instrument(skip_all, fields(namespace, key, locale, tenant_id = %tenant_id))]
pub async fn delete_tenant_override_by_triple(
    ctx: &AppContext,
    tenant_id: Uuid,
    namespace: &str,
    key: &str,
    locale: &str,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<bool> {
    let existing = i18n_translations::Entity::find()
        .filter(i18n_translations::Column::TenantId.eq(tenant_id))
        .filter(i18n_translations::Column::Namespace.eq(namespace))
        .filter(i18n_translations::Column::Key.eq(key))
        .filter(i18n_translations::Column::Locale.eq(locale))
        .one(&ctx.db)
        .await
        .db_err()?;

    let Some(existing) = existing else {
        // Idempotent: cell is already inherited.
        return Ok(false);
    };

    let before = serde_json::json!({
        "namespace": existing.namespace,
        "key": existing.key,
        "locale": existing.locale,
        "value": existing.value,
    });
    let row_id = existing.id;
    let (locale_owned, namespace_owned, key_owned) = (
        existing.locale.clone(),
        existing.namespace.clone(),
        existing.key.clone(),
    );

    let txn = ctx.db.begin().await.db_err()?;
    i18n_translations::Entity::delete_by_id(row_id)
        .exec(&txn)
        .await
        .db_err()?;
    bump_tenant_revision(&txn, &locale_owned, &namespace_owned, tenant_id).await?;
    txn.commit().await.db_err()?;

    invalidate_bundle_cache(ctx, &locale_owned, &namespace_owned, Some(tenant_id)).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Delete,
        "i18n_translation_tenant",
        &format!("{namespace_owned}.{key_owned}::{locale_owned}::{tenant_id}"),
        Some(&before),
        None::<&serde_json::Value>,
    )
    .await
    .model_err()?;

    Ok(true)
}

// ── Internal: helpers exposed for import/export service ─────────────────────

/// Result of [`force_sync_global_in_txn`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForceSyncOutcome {
    /// No row existed; one was inserted.
    Inserted,
    /// A row existed and its `value` differed from `value`; it was updated.
    Updated,
    /// A row existed and already matched `value`; nothing was written.
    Unchanged,
}

/// Force-sync a global translation from the manifest's source default
/// (`t('Ns.key', '中文')`). Used by the manifest endpoint to keep the zh-CN
/// baseline aligned with the latest source code.
///
/// Semantics — frontend source IS the source of truth:
/// - No row exists → INSERT (`Inserted`).
/// - Row exists with the same value → no-op (`Unchanged`).
/// - Row exists with a different value → UPDATE (`Updated`). The previous
///   value is **overwritten unconditionally**; tenant overrides and other
///   locales are unaffected.
///
/// `updated_by` is always NULL because the manifest endpoint runs under a CI
/// token, not a human user.
///
/// The caller drives the transaction and is responsible for bumping the
/// `(locale, namespace)` revision once per affected namespace after all
/// updates are applied.
pub(crate) async fn force_sync_global_in_txn<C>(
    txn: &C,
    namespace: &str,
    key: &str,
    locale: &str,
    value: &str,
) -> loco_rs::Result<ForceSyncOutcome>
where
    C: ConnectionTrait,
{
    let existing = trans_model::Model::find_global(txn, namespace, key, locale)
        .await
        .db_err()?;
    match existing {
        Some(row) if row.value == value => Ok(ForceSyncOutcome::Unchanged),
        Some(row) => {
            let am = i18n_translations::ActiveModel {
                id: ActiveValue::Unchanged(row.id),
                value: ActiveValue::Set(value.to_string()),
                updated_by: ActiveValue::Set(None),
                ..Default::default()
            };
            am.update(txn).await.db_err()?;
            Ok(ForceSyncOutcome::Updated)
        }
        None => {
            let am = i18n_translations::ActiveModel {
                namespace: ActiveValue::Set(namespace.to_string()),
                key: ActiveValue::Set(key.to_string()),
                locale: ActiveValue::Set(locale.to_string()),
                value: ActiveValue::Set(value.to_string()),
                scope: ActiveValue::Set(trans_model::SCOPE_GLOBAL.to_string()),
                tenant_id: ActiveValue::Set(None),
                updated_by: ActiveValue::Set(None),
                ..Default::default()
            };
            am.insert(txn).await.db_err()?;
            Ok(ForceSyncOutcome::Inserted)
        }
    }
}

/// Insert-or-update a single global translation **without** transactional
/// revision bump or audit. Caller must drive the transaction and call
/// [`bump_global_revision`] once per `(locale, namespace)` afterwards.
pub(crate) async fn upsert_global_one_in_txn<C>(
    txn: &C,
    user_id: Uuid,
    namespace: &str,
    key: &str,
    locale: &str,
    value: &str,
) -> loco_rs::Result<bool>
where
    C: ConnectionTrait,
{
    let existing = trans_model::Model::find_global(txn, namespace, key, locale)
        .await
        .db_err()?;
    let was_update = existing.is_some();
    if let Some(row) = existing {
        let am = i18n_translations::ActiveModel {
            id: ActiveValue::Unchanged(row.id),
            value: ActiveValue::Set(value.to_string()),
            updated_by: ActiveValue::Set(Some(user_id)),
            ..Default::default()
        };
        am.update(txn).await.db_err()?;
    } else {
        let am = i18n_translations::ActiveModel {
            namespace: ActiveValue::Set(namespace.to_string()),
            key: ActiveValue::Set(key.to_string()),
            locale: ActiveValue::Set(locale.to_string()),
            value: ActiveValue::Set(value.to_string()),
            scope: ActiveValue::Set(trans_model::SCOPE_GLOBAL.to_string()),
            tenant_id: ActiveValue::Set(None),
            updated_by: ActiveValue::Set(Some(user_id)),
            ..Default::default()
        };
        am.insert(txn).await.db_err()?;
    }
    Ok(was_update)
}

pub(crate) async fn upsert_tenant_one_in_txn<C>(
    txn: &C,
    tenant_id: Uuid,
    user_id: Uuid,
    namespace: &str,
    key: &str,
    locale: &str,
    value: &str,
) -> loco_rs::Result<bool>
where
    C: ConnectionTrait,
{
    let existing =
        trans_model::Model::find_tenant(txn, namespace, key, locale, tenant_id)
            .await
            .db_err()?;
    let was_update = existing.is_some();
    if let Some(row) = existing {
        let am = i18n_translations::ActiveModel {
            id: ActiveValue::Unchanged(row.id),
            value: ActiveValue::Set(value.to_string()),
            updated_by: ActiveValue::Set(Some(user_id)),
            ..Default::default()
        };
        am.update(txn).await.db_err()?;
    } else {
        let am = i18n_translations::ActiveModel {
            namespace: ActiveValue::Set(namespace.to_string()),
            key: ActiveValue::Set(key.to_string()),
            locale: ActiveValue::Set(locale.to_string()),
            value: ActiveValue::Set(value.to_string()),
            scope: ActiveValue::Set(trans_model::SCOPE_TENANT.to_string()),
            tenant_id: ActiveValue::Set(Some(tenant_id)),
            updated_by: ActiveValue::Set(Some(user_id)),
            ..Default::default()
        };
        am.insert(txn).await.db_err()?;
    }
    Ok(was_update)
}

// ── Batch update (update-only, no insert) ─────────────────────────────────

/// Update a single existing global translation row. Returns `Ok(false)` if
/// the `(namespace, key, locale)` triple does not exist (skipped).
async fn update_global_one_in_txn<C>(
    txn: &C,
    user_id: Uuid,
    namespace: &str,
    key: &str,
    locale: &str,
    value: &str,
) -> loco_rs::Result<bool>
where
    C: ConnectionTrait,
{
    let existing = trans_model::Model::find_global(txn, namespace, key, locale)
        .await
        .db_err()?;
    let Some(row) = existing else {
        return Ok(false);
    };
    let am = i18n_translations::ActiveModel {
        id: ActiveValue::Unchanged(row.id),
        value: ActiveValue::Set(value.to_string()),
        updated_by: ActiveValue::Set(Some(user_id)),
        ..Default::default()
    };
    am.update(txn).await.db_err()?;
    Ok(true)
}

/// Update a single existing tenant translation row. Returns `Ok(false)` if
/// the `(namespace, key, locale)` triple does not exist for the tenant.
async fn update_tenant_one_in_txn<C>(
    txn: &C,
    tenant_id: Uuid,
    user_id: Uuid,
    namespace: &str,
    key: &str,
    locale: &str,
    value: &str,
) -> loco_rs::Result<bool>
where
    C: ConnectionTrait,
{
    let existing =
        trans_model::Model::find_tenant(txn, namespace, key, locale, tenant_id)
            .await
            .db_err()?;
    let Some(row) = existing else {
        return Ok(false);
    };
    let am = i18n_translations::ActiveModel {
        id: ActiveValue::Unchanged(row.id),
        value: ActiveValue::Set(value.to_string()),
        updated_by: ActiveValue::Set(Some(user_id)),
        ..Default::default()
    };
    am.update(txn).await.db_err()?;
    Ok(true)
}

/// Validate entries for batch update (format only, not existence).
fn validate_batch_entries(entries: &[ImportEntry]) -> loco_rs::Result<()> {
    if entries.is_empty() {
        return Err(err_bad_request("i18n.entries_empty", "entries 不能为空"));
    }
    if entries.len() > IMPORT_MAX_ENTRIES {
        return Err(err_bad_request(
            "i18n.entries_too_many",
            format!(
                "entries 数量 {} 超过上限 {IMPORT_MAX_ENTRIES}",
                entries.len()
            ),
        ));
    }
    for (i, e) in entries.iter().enumerate() {
        validate_namespace(&e.namespace).map_err(|err| {
            err_bad_request(
                "i18n.entry_namespace_invalid",
                format!("entries[{i}].namespace: {err}"),
            )
        })?;
        validate_key(&e.key).map_err(|err| {
            err_bad_request("i18n.entry_key_invalid", format!("entries[{i}].key: {err}"))
        })?;
        validate_locale(&e.locale).map_err(|err| {
            err_bad_request(
                "i18n.entry_locale_invalid",
                format!("entries[{i}].locale: {err}"),
            )
        })?;
        if e.value.is_empty() {
            return Err(err_bad_request(
                "i18n.entry_value_empty",
                format!("entries[{i}].value 不能为空"),
            ));
        }
    }
    Ok(())
}

/// Batch-update existing global translations. Entries whose
/// `(namespace, key, locale)` triple does not exist in the DB are silently
/// skipped. No new rows are ever created.
#[tracing::instrument(skip_all)]
pub async fn batch_update_global(
    ctx: &AppContext,
    user_id: Uuid,
    entries: &[ImportEntry],
    audit_ctx: &AuditContext,
) -> loco_rs::Result<ImportResponse> {
    validate_batch_entries(entries)?;

    let txn = ctx.db.begin().await.db_err()?;

    let triples: Vec<(String, String, String)> = entries
        .iter()
        .map(|e| (e.namespace.clone(), e.key.clone(), e.locale.clone()))
        .collect();
    let existing_keys: HashSet<(String, String, String)> =
        trans_model::Model::find_existing_global_keys(&txn, &triples)
            .await
            .db_err()?;

    let mut updated: u64 = 0;
    let mut skipped: u64 = 0;
    let mut affected: std::collections::BTreeSet<(String, String)> =
        std::collections::BTreeSet::new();

    for e in entries {
        let triple = (e.namespace.clone(), e.key.clone(), e.locale.clone());
        if !existing_keys.contains(&triple) {
            skipped += 1;
            continue;
        }
        let was_updated = update_global_one_in_txn(
            &txn,
            user_id,
            &e.namespace,
            &e.key,
            &e.locale,
            &e.value,
        )
        .await?;
        if was_updated {
            updated += 1;
            affected.insert((e.locale.clone(), e.namespace.clone()));
        } else {
            skipped += 1;
        }
    }

    for (locale, namespace) in &affected {
        bump_global_revision(&txn, locale, namespace).await?;
    }

    txn.commit().await.db_err()?;

    for (locale, namespace) in &affected {
        invalidate_bundle_cache(ctx, locale, namespace, None).await;
    }

    let summary = serde_json::json!({
        "scope": "global",
        "updated": updated,
        "skipped": skipped,
    });
    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Update,
        "i18n_batch_update_global",
        &format!("entries={}", entries.len()),
        None::<&serde_json::Value>,
        Some(&summary),
    )
    .await
    .model_err()?;

    Ok(ImportResponse {
        inserted: 0,
        updated,
        skipped,
    })
}

/// Batch-update existing tenant translations. Same semantics as
/// [`batch_update_global`] but scoped to a single tenant.
#[tracing::instrument(skip_all)]
pub async fn batch_update_tenant(
    ctx: &AppContext,
    tenant_id: Uuid,
    user_id: Uuid,
    entries: &[ImportEntry],
    audit_ctx: &AuditContext,
) -> loco_rs::Result<ImportResponse> {
    validate_batch_entries(entries)?;

    let (own_prefix, readable_namespaces) =
        load_tenant_namespace_policy(&ctx.db, tenant_id).await?;
    for e in entries {
        ensure_tenant_namespace_allowed(&e.namespace, &own_prefix, &readable_namespaces)?;
    }

    let txn = ctx.db.begin().await.db_err()?;

    let triples: Vec<(String, String, String)> = entries
        .iter()
        .map(|e| (e.namespace.clone(), e.key.clone(), e.locale.clone()))
        .collect();
    let existing_keys: HashSet<(String, String, String)> =
        trans_model::Model::find_existing_tenant_keys(&txn, tenant_id, &triples)
            .await
            .db_err()?;

    let mut updated: u64 = 0;
    let mut skipped: u64 = 0;
    let mut affected: std::collections::BTreeSet<(String, String)> =
        std::collections::BTreeSet::new();

    for e in entries {
        let triple = (e.namespace.clone(), e.key.clone(), e.locale.clone());
        if !existing_keys.contains(&triple) {
            skipped += 1;
            continue;
        }
        let was_updated = update_tenant_one_in_txn(
            &txn,
            tenant_id,
            user_id,
            &e.namespace,
            &e.key,
            &e.locale,
            &e.value,
        )
        .await?;
        if was_updated {
            updated += 1;
            affected.insert((e.locale.clone(), e.namespace.clone()));
        } else {
            skipped += 1;
        }
    }

    for (locale, namespace) in &affected {
        bump_tenant_revision(&txn, locale, namespace, tenant_id).await?;
    }

    txn.commit().await.db_err()?;

    for (locale, namespace) in &affected {
        invalidate_bundle_cache(ctx, locale, namespace, Some(tenant_id)).await;
    }

    let summary = serde_json::json!({
        "scope": "tenant",
        "tenantId": tenant_id.to_string(),
        "updated": updated,
        "skipped": skipped,
    });
    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Update,
        "i18n_batch_update_tenant",
        &format!("tenant={tenant_id}, entries={}", entries.len()),
        None::<&serde_json::Value>,
        Some(&summary),
    )
    .await
    .model_err()?;

    Ok(ImportResponse {
        inserted: 0,
        updated,
        skipped,
    })
}
