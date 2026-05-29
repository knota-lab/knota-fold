//! Bulk import and export of global and tenant translations.

use std::collections::HashSet;

use loco_rs::prelude::*;
use sea_orm::{DatabaseConnection, TransactionTrait};
use uuid::Uuid;

use crate::models::i18n_queries;
use crate::models::i18n_translations as trans_model;
use crate::services::audit_service;
use crate::utils::error::{IntoAppError, IntoModelResult};
use crate::views::audit_logs::{AuditAction, AuditContext};
use crate::views::errors::err_bad_request;
use crate::views::i18n::{
    ExportResponse, ImportEntry, ImportRequest, ImportResponse, ImportScope,
    IMPORT_MAX_ENTRIES,
};

use super::i18n_bundle::{
    bump_global_revision, bump_tenant_revision, invalidate_bundle_cache,
};
use super::i18n_crud::{upsert_global_one_in_txn, upsert_tenant_one_in_txn};
use super::i18n_validation::{
    ensure_tenant_namespace_allowed, load_tenant_namespace_policy, validate_key,
    validate_locale, validate_namespace,
};

// ── Import / export ────────────────────────────────────────────────────────

/// Import strategy parsed from `ImportRequest::strategy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportStrategy {
    /// Overwrite existing values (default).
    Replace,
    /// Leave existing values untouched, only insert new rows.
    Skip,
}

fn parse_strategy(s: Option<&str>) -> loco_rs::Result<ImportStrategy> {
    match s.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("" | "replace") => Ok(ImportStrategy::Replace),
        Some("skip") => Ok(ImportStrategy::Skip),
        Some(other) => Err(err_bad_request(
            "i18n.import_format_invalid",
            format!("未知 strategy '{other}'，支持 replace / skip"),
        )),
    }
}

/// Validate batch shape; returns BadRequest on first violation.
/// All entries must pass before any DB write happens.
fn validate_import_entries(entries: &[ImportEntry]) -> loco_rs::Result<()> {
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

/// Bulk-import global translations in a single transaction.
///
/// All entries are validated before any write. Cache invalidation and audit
/// happen after commit. Revision bumps are coalesced per `(locale, namespace)`
/// so a 500-entry import touching 2 locales × 3 namespaces only does 6
/// `bump_global_revision` calls.
#[tracing::instrument(skip_all)]
pub async fn import_global(
    ctx: &AppContext,
    user_id: Uuid,
    req: &ImportRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<ImportResponse> {
    if req.scope != ImportScope::Global {
        return Err(err_bad_request(
            "i18n.import_scope_mismatch_global",
            "scope 与端点不匹配：期望 global",
        ));
    }
    let strategy = parse_strategy(req.strategy.as_deref())?;
    validate_import_entries(&req.entries)?;

    let txn = ctx.db.begin().await.db_err()?;
    let mut inserted: u64 = 0;
    let mut updated: u64 = 0;
    let mut skipped: u64 = 0;
    let mut affected: std::collections::BTreeSet<(String, String)> =
        std::collections::BTreeSet::new();
    let existing_keys: HashSet<(String, String, String)> =
        if strategy == ImportStrategy::Skip {
            let triples = req
                .entries
                .iter()
                .map(|e| (e.namespace.clone(), e.key.clone(), e.locale.clone()))
                .collect::<Vec<_>>();
            trans_model::Model::find_existing_global_keys(&txn, &triples)
                .await
                .db_err()?
        } else {
            HashSet::new()
        };

    for e in &req.entries {
        if strategy == ImportStrategy::Skip
            && existing_keys.contains(&(
                e.namespace.clone(),
                e.key.clone(),
                e.locale.clone(),
            ))
        {
            skipped += 1;
            continue;
        }
        let was_update = upsert_global_one_in_txn(
            &txn,
            user_id,
            &e.namespace,
            &e.key,
            &e.locale,
            &e.value,
        )
        .await?;
        if was_update {
            updated += 1;
        } else {
            inserted += 1;
        }
        affected.insert((e.locale.clone(), e.namespace.clone()));
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
        "strategy": match strategy {
            ImportStrategy::Replace => "replace",
            ImportStrategy::Skip => "skip",
        },
        "inserted": inserted,
        "updated": updated,
        "skipped": skipped,
        "affectedBundles": affected.len(),
    });
    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Update,
        "i18n_translation_import_global",
        &format!("entries={}", req.entries.len()),
        None::<&serde_json::Value>,
        Some(&summary),
    )
    .await
    .model_err()?;

    Ok(ImportResponse {
        inserted,
        updated,
        skipped,
    })
}

/// Bulk-import tenant override translations in a single transaction.
///
/// Same semantics as [`import_global`] but writes are scoped to a single
/// tenant. A tenant may import namespaces under its own `Tenant.{tenant_code}`
/// prefix or any namespace the tenant can already read.
#[tracing::instrument(skip_all)]
pub async fn import_tenant(
    ctx: &AppContext,
    tenant_id: Uuid,
    user_id: Uuid,
    req: &ImportRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<ImportResponse> {
    if req.scope != ImportScope::Tenant {
        return Err(err_bad_request(
            "i18n.import_scope_mismatch_tenant",
            "scope 与端点不匹配：期望 tenant",
        ));
    }
    let strategy = parse_strategy(req.strategy.as_deref())?;
    validate_import_entries(&req.entries)?;
    let (own_prefix, readable_namespaces) =
        load_tenant_namespace_policy(&ctx.db, tenant_id).await?;
    for e in &req.entries {
        ensure_tenant_namespace_allowed(&e.namespace, &own_prefix, &readable_namespaces)?;
    }

    let txn = ctx.db.begin().await.db_err()?;
    let mut inserted: u64 = 0;
    let mut updated: u64 = 0;
    let mut skipped: u64 = 0;
    let mut affected: std::collections::BTreeSet<(String, String)> =
        std::collections::BTreeSet::new();
    let existing_keys: HashSet<(String, String, String)> =
        if strategy == ImportStrategy::Skip {
            let triples = req
                .entries
                .iter()
                .map(|e| (e.namespace.clone(), e.key.clone(), e.locale.clone()))
                .collect::<Vec<_>>();
            trans_model::Model::find_existing_tenant_keys(&txn, tenant_id, &triples)
                .await
                .db_err()?
        } else {
            HashSet::new()
        };

    for e in &req.entries {
        if strategy == ImportStrategy::Skip
            && existing_keys.contains(&(
                e.namespace.clone(),
                e.key.clone(),
                e.locale.clone(),
            ))
        {
            skipped += 1;
            continue;
        }
        let was_update = upsert_tenant_one_in_txn(
            &txn,
            tenant_id,
            user_id,
            &e.namespace,
            &e.key,
            &e.locale,
            &e.value,
        )
        .await?;
        if was_update {
            updated += 1;
        } else {
            inserted += 1;
        }
        affected.insert((e.locale.clone(), e.namespace.clone()));
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
        "strategy": match strategy {
            ImportStrategy::Replace => "replace",
            ImportStrategy::Skip => "skip",
        },
        "inserted": inserted,
        "updated": updated,
        "skipped": skipped,
        "affectedBundles": affected.len(),
    });
    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Update,
        "i18n_translation_import_tenant",
        &format!("tenant={tenant_id}, entries={}", req.entries.len()),
        None::<&serde_json::Value>,
        Some(&summary),
    )
    .await
    .model_err()?;

    Ok(ImportResponse {
        inserted,
        updated,
        skipped,
    })
}

/// Export global translations as a flat list, optionally filtered.
#[tracing::instrument(skip_all)]
pub async fn export_global(
    db: &DatabaseConnection,
    namespace: Option<&str>,
    locale: Option<&str>,
) -> loco_rs::Result<ExportResponse> {
    let rows = i18n_queries::export_global(db, namespace, locale)
        .await
        .db_err()?;
    Ok(ExportResponse {
        scope: "global".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        entries: rows
            .into_iter()
            .map(|r| ImportEntry {
                namespace: r.namespace,
                key: r.key,
                locale: r.locale,
                value: r.value,
            })
            .collect(),
    })
}

/// Export tenant override translations as a flat list, optionally filtered.
#[tracing::instrument(skip_all)]
pub async fn export_tenant(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    namespace: Option<&str>,
    locale: Option<&str>,
) -> loco_rs::Result<ExportResponse> {
    if let Some(namespace) = namespace {
        let (own_prefix, readable_namespaces) =
            load_tenant_namespace_policy(db, tenant_id).await?;
        ensure_tenant_namespace_allowed(namespace, &own_prefix, &readable_namespaces)?;
    }

    let rows = i18n_queries::export_tenant(db, tenant_id, namespace, locale)
        .await
        .db_err()?;
    Ok(ExportResponse {
        scope: "tenant".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        entries: rows
            .into_iter()
            .map(|r| ImportEntry {
                namespace: r.namespace,
                key: r.key,
                locale: r.locale,
                value: r.value,
            })
            .collect(),
    })
}
