//! Translation key listing for the matrix admin UI — namespaces and key grids.

use std::collections::{BTreeMap, HashMap};

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::models::i18n_queries;
use crate::models::i18n_translations as trans_model;
use crate::utils::error::IntoAppError;
use crate::views::i18n::{
    KeyEntryResponse, KeyListResponse, KeyLocaleValue, NamespaceSummaryResponse,
};

/// List namespaces with per-namespace key/locale counts. Powers the matrix
/// admin UI which renders one parent row per namespace, then lazy-loads the
/// `(key × locale)` grid via [`list_global_keys`] /
/// [`list_tenant_keys`] when expanded.
///
/// Filtering rule:
/// - `tenant_id = None` → only GLOBAL rows (`tenant_id IS NULL`).
/// - `tenant_id = Some(id)` → only that tenant's override rows.
#[tracing::instrument(skip_all)]
pub async fn list_namespaces(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
) -> loco_rs::Result<Vec<NamespaceSummaryResponse>> {
    i18n_queries::list_namespaces(db, tenant_id)
        .await
        .db_err()
        .map(|rows| {
            rows.into_iter()
                .map(|row| NamespaceSummaryResponse {
                    namespace: row.namespace,
                    key_count: row.key_count,
                    locale_count: row.locale_count,
                })
                .collect()
        })
}

/// Tenant-facing namespace listing: union of global namespaces and the
/// tenant's own override namespaces.
///
/// Mirrors the `list_tenant_keys` skeleton model — every global namespace
/// is editable by the tenant, plus any namespaces the tenant has already
/// overridden (typically a subset, but we union for safety in case global
/// rows were deleted after the tenant created an override).
///
/// Counts are computed across the union so the UI badge reflects the
/// total number of `(namespace, distinct key)` and distinct locales the
/// user can address — not just the rows the tenant has authored.
pub async fn list_tenant_namespaces(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> loco_rs::Result<Vec<NamespaceSummaryResponse>> {
    i18n_queries::list_tenant_namespaces(db, tenant_id)
        .await
        .db_err()
        .map(|rows| {
            rows.into_iter()
                .map(|row| NamespaceSummaryResponse {
                    namespace: row.namespace,
                    key_count: row.key_count,
                    locale_count: row.locale_count,
                })
                .collect()
        })
}

/// Internal helper: list translations grouped into one entry per
/// `(namespace, key)`, with all locale variants bundled.
///
/// Implementation notes:
/// - Pagination is on **distinct keys**, not on individual translation rows,
///   so a single page never splits a key's locale set across pages.
/// - We use two queries: first paginate `(namespace, key)` (DISTINCT) on the
///   server, then fetch all translation rows for the page's keys in one
///   `WHERE (namespace, key) IN (...)`-style batch and group in memory.
/// - `q` matches against `key` OR `value` (case-insensitive via `LIKE`).
async fn list_keys_inner(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    namespace: Option<&str>,
    q: Option<&str>,
    empty_locale: Option<&str>,
    page: u64,
    page_size: u64,
) -> loco_rs::Result<KeyListResponse> {
    let offset = page.saturating_sub(1) * page_size;

    let total = i18n_queries::count_distinct_keys(
        db,
        match tenant_id {
            Some(tid) => i18n_queries::KeyScope::TenantOnly(tid),
            None => i18n_queries::KeyScope::GlobalOnly,
        },
        namespace,
        q,
        empty_locale,
    )
    .await
    .db_err()?;

    let key_rows = i18n_queries::paginate_distinct_keys(
        db,
        match tenant_id {
            Some(tid) => i18n_queries::KeyScope::TenantOnly(tid),
            None => i18n_queries::KeyScope::GlobalOnly,
        },
        namespace,
        q,
        empty_locale,
        page_size,
        offset,
    )
    .await
    .db_err()?;

    let mut entries: Vec<KeyEntryResponse> = Vec::with_capacity(key_rows.len());
    let mut order: HashMap<(String, String), usize> =
        HashMap::with_capacity(key_rows.len());
    for (ns, k) in &key_rows {
        order.insert((ns.clone(), k.clone()), entries.len());
        entries.push(KeyEntryResponse {
            stable_id: format!("{ns}.{k}"),
            namespace: ns.clone(),
            key: k.clone(),
            by_locale: BTreeMap::new(),
            entry_id: None,
            entry_status: None,
            entry_description: None,
            entry_last_seen_at: None,
        });
    }

    if entries.is_empty() {
        let total_pages = total.div_ceil(page_size.max(1));
        return Ok(KeyListResponse {
            items: entries,
            total_items: total,
            total_pages,
            page,
            page_size,
        });
    }

    // ── Pass 2: fetch all translation rows for the page's (ns, key) pairs. ──
    //
    // SeaORM's IN-tuple support is awkward; we instead OR-chain a `(namespace
    // = $a AND key = $b)` clause per key. Page size is bounded (≤200), so the
    // resulting WHERE has at most 200 OR-groups — well within DB planner
    // limits.
    let detail_rows = i18n_queries::fetch_detail_rows(db, tenant_id, &key_rows)
        .await
        .db_err()?;

    for row in detail_rows {
        let Some(&idx) = order.get(&(row.namespace.clone(), row.key.clone())) else {
            continue; // shouldn't happen — pass 1 selected the same set
        };
        entries[idx].by_locale.insert(
            row.locale.clone(),
            KeyLocaleValue {
                id: row.id.to_string(),
                value: row.value.clone(),
                updated_at: row.updated_at.to_rfc3339(),
                // Global listing: every value is authoritative.
                is_override: false,
                inherited_value: None,
            },
        );
    }

    let entry_meta = i18n_queries::fetch_entries_by_pairs(db, &key_rows)
        .await
        .db_err()?;
    for entry in &mut entries {
        if let Some(meta) = entry_meta.get(&(entry.namespace.clone(), entry.key.clone()))
        {
            entry.entry_id = Some(meta.id.to_string());
            entry.entry_status = Some(meta.status.clone());
            entry.entry_description.clone_from(&meta.description);
            entry.entry_last_seen_at = Some(meta.last_seen_at.to_rfc3339());
        }
    }

    let total_pages = total.div_ceil(page_size.max(1));
    Ok(KeyListResponse {
        items: entries,
        total_items: total,
        total_pages,
        page,
        page_size,
    })
}

/// List global keys (one row per `(namespace, key)`, locale variants bundled).
#[tracing::instrument(skip_all)]
pub async fn list_global_keys(
    db: &DatabaseConnection,
    namespace: Option<&str>,
    q: Option<&str>,
    empty_locale: Option<&str>,
    page: u64,
    page_size: u64,
) -> loco_rs::Result<KeyListResponse> {
    list_keys_inner(db, None, namespace, q, empty_locale, page, page_size).await
}

/// List a tenant's keys with **inherited global values** as the baseline.
///
/// Unlike [`list_global_keys`], a row appears here as soon as **either** a
/// global translation OR a tenant override exists for `(namespace, key)`.
/// For each `(namespace, key, locale)` cell the tenant override wins; if
/// only the global row exists the cell is returned with that value but
/// `is_override = false` so the UI can mark it as inherited and offer a
/// "create override" affordance. When both exist, `inherited_value` carries
/// the global text being shadowed so the UI can show a "reset to global"
/// button without an extra round-trip.
///
/// `q` matches against `key` OR `value` across BOTH scopes. Filtering by a
/// tenant-only override value will surface the row even if no global row
/// matches, and vice versa.
#[tracing::instrument(skip_all)]
pub async fn list_tenant_keys(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    namespace: Option<&str>,
    q: Option<&str>,
    empty_locale: Option<&str>,
    page: u64,
    page_size: u64,
) -> loco_rs::Result<KeyListResponse> {
    let offset = page.saturating_sub(1) * page_size;

    let total = i18n_queries::count_distinct_keys(
        db,
        i18n_queries::KeyScope::TenantWithGlobal(tenant_id),
        namespace,
        q,
        empty_locale,
    )
    .await
    .db_err()?;

    let key_rows = i18n_queries::paginate_distinct_keys(
        db,
        i18n_queries::KeyScope::TenantWithGlobal(tenant_id),
        namespace,
        q,
        empty_locale,
        page_size,
        offset,
    )
    .await
    .db_err()?;

    let mut entries: Vec<KeyEntryResponse> = Vec::with_capacity(key_rows.len());
    let mut order: HashMap<(String, String), usize> =
        HashMap::with_capacity(key_rows.len());
    for (ns, k) in &key_rows {
        order.insert((ns.clone(), k.clone()), entries.len());
        entries.push(KeyEntryResponse {
            stable_id: format!("{ns}.{k}"),
            namespace: ns.clone(),
            key: k.clone(),
            by_locale: BTreeMap::new(),
            entry_id: None,
            entry_status: None,
            entry_description: None,
            entry_last_seen_at: None,
        });
    }

    if entries.is_empty() {
        let total_pages = total.div_ceil(page_size.max(1));
        return Ok(KeyListResponse {
            items: entries,
            total_items: total,
            total_pages,
            page,
            page_size,
        });
    }

    // ── Pass 2: fetch BOTH global and tenant detail rows for the page. ──────
    //
    // Two queries with the same OR-chain pair filter, scoped by tenant_id.
    // Page size is bounded (≤200) so the OR-chain stays well within planner
    // limits.
    let global_rows = i18n_queries::fetch_global_rows(db, &key_rows)
        .await
        .db_err()?;

    let tenant_rows = i18n_queries::fetch_tenant_rows(db, tenant_id, &key_rows)
        .await
        .db_err()?;

    // Index globals by (ns, key, locale) so the tenant pass can look up the
    // shadowed value in O(1) and stamp it onto `inherited_value`.
    let mut global_index: HashMap<(String, String, String), &trans_model::Model> =
        HashMap::with_capacity(global_rows.len());
    for row in &global_rows {
        global_index.insert(
            (row.namespace.clone(), row.key.clone(), row.locale.clone()),
            row,
        );
    }

    // Seed cells from globals first (inherited baseline).
    for row in &global_rows {
        let Some(&idx) = order.get(&(row.namespace.clone(), row.key.clone())) else {
            continue;
        };
        entries[idx].by_locale.insert(
            row.locale.clone(),
            KeyLocaleValue {
                id: String::new(),
                value: row.value.clone(),
                updated_at: row.updated_at.to_rfc3339(),
                is_override: false,
                inherited_value: None,
            },
        );
    }

    // Tenant rows shadow globals; record the shadowed text as
    // `inherited_value` so the UI can offer "reset to global" inline.
    for row in &tenant_rows {
        let Some(&idx) = order.get(&(row.namespace.clone(), row.key.clone())) else {
            continue;
        };
        let inherited = global_index
            .get(&(row.namespace.clone(), row.key.clone(), row.locale.clone()))
            .map(|g| g.value.clone());
        entries[idx].by_locale.insert(
            row.locale.clone(),
            KeyLocaleValue {
                id: row.id.to_string(),
                value: row.value.clone(),
                updated_at: row.updated_at.to_rfc3339(),
                is_override: true,
                inherited_value: inherited,
            },
        );
    }

    let entry_meta = i18n_queries::fetch_entries_by_pairs(db, &key_rows)
        .await
        .db_err()?;
    for entry in &mut entries {
        if let Some(meta) = entry_meta.get(&(entry.namespace.clone(), entry.key.clone()))
        {
            entry.entry_id = Some(meta.id.to_string());
            entry.entry_status = Some(meta.status.clone());
            entry.entry_description.clone_from(&meta.description);
            entry.entry_last_seen_at = Some(meta.last_seen_at.to_rfc3339());
        }
    }

    let total_pages = total.div_ceil(page_size.max(1));
    Ok(KeyListResponse {
        items: entries,
        total_items: total,
        total_pages,
        page,
        page_size,
    })
}
