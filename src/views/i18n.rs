//! DTOs for the i18n module — request bodies, query params, and responses.
//!
//! Naming follows the project convention (`#[serde(rename_all = "camelCase")]`).
//! `stable_id` in payloads is the computed `{namespace}.{key}` string and is
//! never persisted as a column.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::models::_entities::{
    i18n_bundle_revisions, i18n_entries, i18n_supported_locales, i18n_translations,
};

// ── Constants (also re-exported at module level) ────────────────────────────

/// Tenant-managed namespace prefix. Anything else is global-only.
pub const TENANT_NAMESPACE_PREFIX: &str = "Tenant";
/// Hard-coded base locale. The contract says `zh-CN`.
pub const BASE_LOCALE: &str = "zh-CN";
/// Public namespaces — readable without auth (login screen etc.).
pub const PUBLIC_NAMESPACES: &[&str] = &["Common", "CommonError"];
/// Namespaces preloaded by the frontend on first paint.
pub const PRELOAD_NAMESPACES: &[&str] = &["Common", "Layout"];
/// Hard cap for import payload size (bytes).
pub const IMPORT_MAX_BYTES: usize = 2 * 1024 * 1024;
/// Hard cap for entries per import.
pub const IMPORT_MAX_ENTRIES: usize = 500;

// ── Public bundle endpoint ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleQuery {
    pub locale: String,
    pub namespace: String,
}

/// Body of `GET /api/i18n/bundle`. Map key is the **fully-qualified**
/// `{namespace}.{key}` string (matches the frontend `t()` lookup).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BundleResponse {
    pub locale: String,
    pub namespace: String,
    pub revision: String,
    /// `BTreeMap` to keep deterministic ordering for ETag stability.
    pub entries: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LocaleResponse {
    pub locale: String,
    pub label: String,
    pub is_enabled: bool,
    pub sort_order: i32,
}

impl From<&i18n_supported_locales::Model> for LocaleResponse {
    fn from(m: &i18n_supported_locales::Model) -> Self {
        Self {
            locale: m.locale.clone(),
            label: m.label.clone(),
            is_enabled: m.is_enabled,
            sort_order: m.sort_order,
        }
    }
}

// ── Admin: supported locales ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateLocaleRequest {
    pub locale: String,
    pub label: String,
    pub is_enabled: Option<bool>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLocaleRequest {
    pub label: Option<String>,
    pub is_enabled: Option<bool>,
    pub sort_order: Option<i32>,
}

// ── Admin: entries ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryListParams {
    pub namespace: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
    pub page: Option<u64>,
    pub page_size: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryResponse {
    pub id: String,
    pub stable_id: String,
    pub namespace: String,
    pub key: String,
    pub description: Option<String>,
    pub status: String,
    pub last_seen_at: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&i18n_entries::Model> for EntryResponse {
    fn from(m: &i18n_entries::Model) -> Self {
        Self {
            id: m.id.to_string(),
            stable_id: format!("{}.{}", m.namespace, m.key),
            namespace: m.namespace.clone(),
            key: m.key.clone(),
            description: m.description.clone(),
            status: m.status.clone(),
            last_seen_at: m.last_seen_at.to_rfc3339(),
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

// ── Admin/Tenant: translations ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationListParams {
    pub namespace: Option<String>,
    pub locale: Option<String>,
    pub q: Option<String>,
    pub page: Option<u64>,
    pub page_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertGlobalTranslationRequest {
    pub namespace: String,
    pub key: String,
    pub locale: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTranslationRequest {
    pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertTenantOverrideRequest {
    pub namespace: String,
    pub key: String,
    pub locale: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationResponse {
    pub id: String,
    pub stable_id: String,
    pub namespace: String,
    pub key: String,
    pub locale: String,
    pub value: String,
    pub scope: String,
    pub tenant_id: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&i18n_translations::Model> for TranslationResponse {
    fn from(m: &i18n_translations::Model) -> Self {
        Self {
            id: m.id.to_string(),
            stable_id: format!("{}.{}", m.namespace, m.key),
            namespace: m.namespace.clone(),
            key: m.key.clone(),
            locale: m.locale.clone(),
            value: m.value.clone(),
            scope: m.scope.clone(),
            tenant_id: m.tenant_id.map(|id| id.to_string()),
            updated_by: m.updated_by.map(|id| id.to_string()),
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationListResponse {
    pub items: Vec<TranslationResponse>,
    pub total_items: u64,
    pub total_pages: u64,
    pub page: u64,
    pub page_size: u64,
}

// ── Namespaces summary (matrix UI) ──────────────────────────────────────────

/// One row of `GET /api/admin/i18n/namespaces` — used by the matrix table to
/// render the parent rows (one per namespace) before lazy-loading the
/// `(key × locale)` grid for an expanded namespace.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceSummaryResponse {
    pub namespace: String,
    /// Distinct number of keys present in this namespace (any locale).
    pub key_count: u64,
    /// Distinct number of locales that have at least one translation here.
    pub locale_count: u64,
}

// ── Keys list (one row per `(namespace, key)`) ──────────────────────────────

/// One locale entry inside [`KeyEntryResponse::by_locale`].
///
/// In tenant-scoped listings (`/tenant/i18n/keys` and the super-admin
/// per-tenant variant) a row may exist as a global translation only — i.e.
/// the tenant has not yet authored an override. In that case `value` carries
/// the inherited global text and `is_override = false`; `id` is empty
/// because no `i18n_translations` row exists for `(tenant_id, ns, key,
/// locale)` yet. When the tenant *has* overridden the value, `is_override
/// = true`, `id` is the override row's id, and `inherited_value` (if
/// present) holds the global text the override is shadowing — useful for
/// the admin UI to expose a "reset to global" affordance without a second
/// round-trip.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KeyLocaleValue {
    pub id: String,
    pub value: String,
    pub updated_at: String,
    /// `true` iff the row originates from this tenant's override scope.
    /// Always `true` in the global keys endpoint.
    pub is_override: bool,
    /// Global value being shadowed by an override. `None` when not
    /// applicable (global listing, or tenant row with no global counterpart
    /// for this locale).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inherited_value: Option<String>,
}

/// One row of `GET /api/admin/i18n/keys` — represents a single
/// `(namespace, key)` pair plus all of its locale translations bundled
/// together so the admin UI can render a "key with N expandable locale
/// children" tree without N+1 round-trips.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyEntryResponse {
    pub namespace: String,
    pub key: String,
    /// Computed `{namespace}.{key}` — convenient as the React row key.
    pub stable_id: String,
    /// Map of `locale → translation`. Locales without a translation are
    /// simply absent from the map (the frontend renders an empty editor).
    pub by_locale: BTreeMap<String, KeyLocaleValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_last_seen_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyListResponse {
    pub items: Vec<KeyEntryResponse>,
    pub total_items: u64,
    pub total_pages: u64,
    pub page: u64,
    pub page_size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyListParams {
    /// Filter by exact namespace (top-level dropdown in the admin UI).
    pub namespace: Option<String>,
    /// Substring search against `key` and any `value`.
    pub q: Option<String>,
    /// Only return keys where this locale has no (or empty) translation value.
    pub empty_locale: Option<String>,
    pub page: Option<u64>,
    pub page_size: Option<u64>,
}

// ── Locales admin response ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocaleAdminResponse {
    pub id: String,
    pub locale: String,
    pub label: String,
    pub is_enabled: bool,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&i18n_supported_locales::Model> for LocaleAdminResponse {
    fn from(m: &i18n_supported_locales::Model) -> Self {
        Self {
            id: m.id.to_string(),
            locale: m.locale.clone(),
            label: m.label.clone(),
            is_enabled: m.is_enabled,
            sort_order: m.sort_order,
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

// ── Bundle revision (debug / admin) ─────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleRevisionResponse {
    pub locale: String,
    pub namespace: String,
    pub scope: String,
    pub tenant_id: Option<String>,
    pub revision: i64,
    pub updated_at: String,
}

impl From<&i18n_bundle_revisions::Model> for BundleRevisionResponse {
    fn from(m: &i18n_bundle_revisions::Model) -> Self {
        Self {
            locale: m.locale.clone(),
            namespace: m.namespace.clone(),
            scope: m.scope.clone(),
            tenant_id: m.tenant_id.map(|id| id.to_string()),
            revision: m.revision,
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

// ── Manifest upload (CI) ────────────────────────────────────────────────────

/// Body of `POST /api/ci/i18n/manifest`. Sent by `i18n-extractor upload`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestUploadRequest {
    /// Iso8601 timestamp of when the extractor ran. Optional – informational.
    pub generated_at: Option<String>,
    /// Optional commit SHA — recorded for audit only.
    pub commit_sha: Option<String>,
    pub entries: Vec<ManifestEntryInput>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntryInput {
    pub namespace: String,
    pub key: String,
    pub description: Option<String>,
    pub locations: Vec<ManifestLocation>,
    /// 提取器从 `t('Ns.key', '中文', ...)` 第二个字符串字面量参数采集到的源码默认值。
    /// 仅作为 zh-CN 翻译缺失时的初始值（seed）：若该 (namespace, key, zh-CN) 已存在
    /// 翻译记录，则 **不会** 被覆盖。带插值的源码（如 `'你好 {{name}}'`）原样入库。
    pub source_text: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ManifestLocation {
    pub file_path: String,
    pub line: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestUploadResponse {
    pub created_entries: u64,
    pub updated_entries: u64,
    pub stale_entries: u64,
    pub total_locations: u64,
    /// zh-CN 基线翻译被首次写入的条数（之前 DB 中无对应行）。
    pub synced_inserted: u64,
    /// zh-CN 基线翻译被覆盖更新的条数（DB 中已有但 value 不同）。
    /// 注意：前端源码是真理来源，admin 在管理后台对 zh-CN 的修改会在
    /// 下次 manifest 上传时被前端硬编码覆盖。
    pub synced_overwritten: u64,
}

// ── Import / export ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub scope: ImportScope,
    pub entries: Vec<ImportEntry>,
    /// `replace` (default) overwrites existing values; `skip` leaves them.
    pub strategy: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImportScope {
    Global,
    Tenant,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImportEntry {
    pub namespace: String,
    pub key: String,
    pub locale: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResponse {
    pub inserted: u64,
    pub updated: u64,
    pub skipped: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportQuery {
    pub scope: Option<String>,
    pub namespace: Option<String>,
    pub locale: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResponse {
    pub scope: String,
    pub generated_at: String,
    pub entries: Vec<ImportEntry>,
}

// ── Batch update (update-only, no insert) ────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchUpdateRequest {
    pub entries: Vec<ImportEntry>,
}

// ── User preference (GET/PUT /api/i18n/me) ──────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct I18nMeResponse {
    /// User's saved locale preference. `null` when not set.
    pub preferred_locale: Option<String>,
    /// Default fallback chain the frontend should use.
    pub default_locale: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateI18nMeRequest {
    pub preferred_locale: Option<String>,
}

// ── Entries DTOs (list/locations responses) ──────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryListResponse {
    pub items: Vec<EntryResponse>,
    pub total_items: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryLocationResponse {
    pub id: String,
    pub file_path: String,
    pub line: i32,
}
