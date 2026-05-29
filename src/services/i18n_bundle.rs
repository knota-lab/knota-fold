//! Bundle resolution, ETag computation, cache management, and revision bookkeeping.
//!
//! ## Resolution model
//!
//! A bundle is identified by `(locale, namespace, scope, tenant_id)`. The
//! frontend always asks for `(locale, namespace)` for a specific tenant
//! context. The resolved bundle merges:
//!
//! 1. Global rows (`scope='global'`, `tenant_id=NULL`)
//! 2. Tenant overrides (`scope='tenant'`, `tenant_id=$1`) — win on conflict
//! 3. Base-locale fallback (`zh-CN`) — fills gaps when target locale is missing
//!
//! ## ETag formula
//!
//! `ETag = "{global_revision}-{tenant_revision}"`
//!
//! - `global_revision` is read from `i18n_bundle_revisions` row
//!   `(scope='global', tenant_id=NULL)`. If absent → `0`.
//! - `tenant_revision` is from `(scope='tenant', tenant_id=?)`. If absent → `0`.
//! - Public-only callers (no tenant context) use `tenant_revision = 0`.
//!
//! ## Revision cascade
//!
//! Any GLOBAL write to `(locale, namespace)` increments:
//! - that GLOBAL revision row, AND
//! - **every existing TENANT revision row for the same (locale, namespace)**
//!   so cached tenant ETags also invalidate.
//!
//! TENANT writes only bump the tenant revision row.

use std::collections::BTreeMap;
use std::time::Duration;

use loco_rs::prelude::*;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    QueryFilter, Statement,
};
use uuid::Uuid;

use crate::models::_entities::i18n_bundle_revisions;
use crate::models::i18n_bundle_revisions as rev_model;
use crate::models::i18n_queries;
use crate::models::i18n_translations as trans_model;
use crate::utils::error::IntoAppError;
use crate::views::i18n::{BundleResponse, BASE_LOCALE};

use super::i18n_validation::{validate_locale, validate_namespace};

const BUNDLE_CACHE_TTL: Duration = Duration::from_secs(300);

// ── Cache keys ──────────────────────────────────────────────────────────────

fn cache_key_bundle(locale: &str, namespace: &str, tenant: &str) -> String {
    format!("i18n:bundle:{tenant}:{namespace}:{locale}")
}

fn cache_key_etag(locale: &str, namespace: &str, tenant: &str) -> String {
    format!("i18n:etag:{tenant}:{namespace}:{locale}")
}

fn tenant_str(tenant_id: Option<Uuid>) -> String {
    tenant_id.map_or_else(|| "global".to_string(), |id| id.to_string())
}

// ── Revision helpers ────────────────────────────────────────────────────────

async fn bump_revision_inner<C>(
    txn: &C,
    locale: &str,
    namespace: &str,
    scope: &str,
    tenant_id: Option<Uuid>,
) -> loco_rs::Result<i64>
where
    C: ConnectionTrait,
{
    let mut query = i18n_bundle_revisions::Entity::find()
        .filter(i18n_bundle_revisions::Column::Locale.eq(locale))
        .filter(i18n_bundle_revisions::Column::Namespace.eq(namespace))
        .filter(i18n_bundle_revisions::Column::Scope.eq(scope));
    query = match tenant_id {
        Some(tid) => query.filter(i18n_bundle_revisions::Column::TenantId.eq(tid)),
        None => query.filter(i18n_bundle_revisions::Column::TenantId.is_null()),
    };

    let existing = query.one(txn).await.db_err()?;

    let new_rev = if let Some(row) = existing {
        let next = row.revision + 1;
        let am = i18n_bundle_revisions::ActiveModel {
            id: ActiveValue::Unchanged(row.id),
            revision: ActiveValue::Set(next),
            ..Default::default()
        };
        am.update(txn).await.db_err()?;
        next
    } else {
        let am = i18n_bundle_revisions::ActiveModel {
            locale: ActiveValue::Set(locale.to_string()),
            namespace: ActiveValue::Set(namespace.to_string()),
            scope: ActiveValue::Set(scope.to_string()),
            tenant_id: ActiveValue::Set(tenant_id),
            revision: ActiveValue::Set(1),
            ..Default::default()
        };
        am.insert(txn).await.db_err()?;
        1
    };

    Ok(new_rev)
}

/// Increment (or create) the GLOBAL revision row, then cascade all TENANT rows
/// for the same (locale, namespace). Run inside the caller's transaction.
pub(crate) async fn bump_global_revision<C>(
    txn: &C,
    locale: &str,
    namespace: &str,
) -> loco_rs::Result<i64>
where
    C: ConnectionTrait,
{
    let new_rev =
        bump_revision_inner(txn, locale, namespace, rev_model::SCOPE_GLOBAL, None)
            .await?;

    // Cascade — bump every existing tenant revision row.
    let backend = txn.get_database_backend();
    txn.execute(Statement::from_sql_and_values(
        backend,
        "UPDATE i18n_bundle_revisions \
         SET revision = revision + 1, updated_at = CURRENT_TIMESTAMP \
         WHERE locale = $1 AND namespace = $2 AND scope = 'tenant'",
        [locale.to_string().into(), namespace.to_string().into()],
    ))
    .await
    .db_err()?;

    Ok(new_rev)
}

/// Increment (or create) a single TENANT revision row.
pub(crate) async fn bump_tenant_revision<C>(
    txn: &C,
    locale: &str,
    namespace: &str,
    tenant_id: Uuid,
) -> loco_rs::Result<i64>
where
    C: ConnectionTrait,
{
    bump_revision_inner(
        txn,
        locale,
        namespace,
        rev_model::SCOPE_TENANT,
        Some(tenant_id),
    )
    .await
}

async fn read_revisions(
    db: &DatabaseConnection,
    locale: &str,
    namespace: &str,
    tenant_id: Option<Uuid>,
) -> loco_rs::Result<(i64, i64)> {
    i18n_queries::read_revisions(db, locale, namespace, tenant_id)
        .await
        .db_err()
}

/// Bumped whenever the on-the-wire bundle shape changes in a way that would
/// make a cached client payload incorrect even if the (global, tenant)
/// revision pair is unchanged. v2: `entries` keys switched from
/// `"{namespace}.{key}"` to bare `"{key}"` to match the frontend resolver
/// contract (see `build_bundle_from_db`).
const BUNDLE_SHAPE_VERSION: u32 = 2;

#[must_use]
pub fn etag_for(global_rev: i64, tenant_rev: i64) -> String {
    format!("\"v{BUNDLE_SHAPE_VERSION}-{global_rev}-{tenant_rev}\"")
}

fn etag_for_bundle(b: &BundleResponse) -> String {
    // bundle.revision is already `"{global}-{tenant}"`. Prefixing the shape
    // version forces clients holding a stale-shape payload to refetch.
    format!("\"v{BUNDLE_SHAPE_VERSION}-{}\"", b.revision)
}

/// Cheap pre-flight ETag (no bundle materialization). Used for `If-None-Match`.
#[tracing::instrument(skip_all)]
pub async fn compute_etag(
    ctx: &AppContext,
    locale: &str,
    namespace: &str,
    tenant_id: Option<Uuid>,
) -> loco_rs::Result<String> {
    let ts = tenant_str(tenant_id);
    let cache_key = cache_key_etag(locale, namespace, &ts);

    let etag = ctx
        .cache
        .get_or_insert_with_expiry::<String, _>(&cache_key, BUNDLE_CACHE_TTL, async {
            let (g, t) = read_revisions(&ctx.db, locale, namespace, tenant_id).await?;
            Ok(etag_for(g, t))
        })
        .await?;
    Ok(etag)
}

// ── Bundle resolution ───────────────────────────────────────────────────────

/// Resolve a bundle for `(locale, namespace, tenant_id)`. Returns the bundle
/// AND the ETag string (so callers can set the response header).
#[tracing::instrument(skip_all)]
pub async fn resolve_bundle(
    ctx: &AppContext,
    locale: &str,
    namespace: &str,
    tenant_id: Option<Uuid>,
) -> loco_rs::Result<(BundleResponse, String)> {
    validate_locale(locale)?;
    validate_namespace(namespace)?;

    let ts = tenant_str(tenant_id);
    let cache_key = cache_key_bundle(locale, namespace, &ts);

    let bundle = ctx
        .cache
        .get_or_insert_with_expiry::<BundleResponse, _>(
            &cache_key,
            BUNDLE_CACHE_TTL,
            async { build_bundle_from_db(&ctx.db, locale, namespace, tenant_id).await },
        )
        .await?;

    let etag = etag_for_bundle(&bundle);
    Ok((bundle, etag))
}

async fn build_bundle_from_db(
    db: &DatabaseConnection,
    locale: &str,
    namespace: &str,
    tenant_id: Option<Uuid>,
) -> loco_rs::Result<BundleResponse> {
    let (global_rev, tenant_rev) =
        read_revisions(db, locale, namespace, tenant_id).await?;

    // 1. Global rows for the requested locale.
    //
    // Bundle is already scoped to one (namespace, locale); the entries map's
    // key is the *sub-key only* (e.g. `title`), not `Welcome.title`. The
    // frontend resolver in `src/i18n/translate.ts` strips the namespace
    // prefix from the full lookup key and queries this map by sub-key, so
    // including the prefix here would cause every lookup to miss and fall
    // back to the source-text fallback baked into the call site.
    let global_fut = trans_model::Model::list_global_bundle(db, namespace, locale);
    let base_fut = async {
        if locale != BASE_LOCALE {
            trans_model::Model::list_global_bundle(db, namespace, BASE_LOCALE).await
        } else {
            Ok(Vec::new())
        }
    };

    let (global_rows, base_rows) = tokio::join!(global_fut, base_fut);
    let global_rows = global_rows.db_err()?;
    let base_rows = base_rows.db_err()?;

    let mut entries: BTreeMap<String, String> = BTreeMap::new();
    for row in &global_rows {
        entries.insert(row.key.clone(), row.value.clone());
    }

    // 2. Base-locale fallback (only if requested locale != base).
    for row in &base_rows {
        entries
            .entry(row.key.clone())
            .or_insert_with(|| row.value.clone());
    }

    // 3. Tenant overrides — win on conflict.
    if let Some(tid) = tenant_id {
        let tenant_rows =
            trans_model::Model::list_tenant_bundle(db, namespace, locale, tid)
                .await
                .db_err()?;
        for row in &tenant_rows {
            entries.insert(row.key.clone(), row.value.clone());
        }
    }

    Ok(BundleResponse {
        locale: locale.to_string(),
        namespace: namespace.to_string(),
        revision: format!("{global_rev}-{tenant_rev}"),
        entries,
    })
}

// ── Cache invalidation ──────────────────────────────────────────────────────

pub(crate) async fn invalidate_bundle_cache(
    ctx: &AppContext,
    locale: &str,
    namespace: &str,
    tenant_id: Option<Uuid>,
) {
    let ts = tenant_str(tenant_id);
    let _ = ctx
        .cache
        .remove(&cache_key_bundle(locale, namespace, &ts))
        .await;
    let _ = ctx
        .cache
        .remove(&cache_key_etag(locale, namespace, &ts))
        .await;

    // Global write also invalidates ALL tenant caches we know about — we don't
    // track them, so fall back to TTL expiry. The revision cascade already
    // makes tenant ETags change, so stale tenant cache entries will be
    // detected by clients via If-None-Match → 200.
    if tenant_id.is_none() {
        let _ = ctx
            .cache
            .remove(&cache_key_bundle(locale, namespace, "global"))
            .await;
        let _ = ctx
            .cache
            .remove(&cache_key_etag(locale, namespace, "global"))
            .await;
    }
}

/// Public façade so the manifest service can invalidate a (locale, namespace)
/// global bundle cache after seeding. Mirrors the in-module helper exactly.
pub async fn invalidate_global_bundle_cache(
    ctx: &AppContext,
    locale: &str,
    namespace: &str,
) {
    invalidate_bundle_cache(ctx, locale, namespace, None).await;
}

/// Public wrapper around `bump_global_revision` for cross-service callers.
pub async fn bump_global_revision_pub<C>(
    txn: &C,
    locale: &str,
    namespace: &str,
) -> loco_rs::Result<i64>
where
    C: ConnectionTrait,
{
    bump_global_revision(txn, locale, namespace).await
}

pub async fn bump_tenant_revision_pub<C>(
    txn: &C,
    locale: &str,
    namespace: &str,
    tenant_id: Uuid,
) -> loco_rs::Result<i64>
where
    C: ConnectionTrait,
{
    bump_tenant_revision(txn, locale, namespace, tenant_id).await
}
