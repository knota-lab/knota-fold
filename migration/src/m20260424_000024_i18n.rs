//! i18n module — schema for the in-DB translation system.
//!
//! Tables (see `system-design/国际化.md` §4):
//! - `i18n_supported_locales`     — platform-wide enabled locales.
//! - `i18n_entries`               — registry of (namespace + key) with status + description.
//! - `i18n_entry_locations`       — extractor-discovered call sites; FK → entries with cascade.
//! - `i18n_translations`          — actual translation rows; scope=global|tenant + `tenant_id` NULL=global.
//! - `i18n_bundle_revisions`      — monotonic revision per (locale, namespace, scope, `tenant_id`?), drives `ETag`.
//!
//! NOTE on UNIQUE-with-NULL: `PostgreSQL` and `SQLite` both treat NULL as distinct
//! in unique indexes, so `(…, tenant_id)` uniques will NOT prevent duplicate
//! NULL-tenant rows. Service layer must guard global duplicates (same pattern
//! as `sys_configs`).
//!
//! Also seeds:
//! - `i18n_supported_locales` with zh-CN + en-US
//! - `i18n_translations` with the minimal Common + `CommonError` bundles (§10)
//! - `i18n_bundle_revisions` rows aligned with the seeded bundles
//!
//! Casbin permissions are NOT seeded here. The codebase uses URL-based auth:
//! `permissions.code = "{METHOD}:{path}"` is auto-synced from registered routes
//! by `permission_service::sync_permissions`. The CI manifest endpoint is
//! guarded by `ci_token_layer`, not Casbin.

use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // ── i18n_supported_locales ───────────────────────────────────────────
        m.create_table(
            Table::create()
                .table(I18nSupportedLocales::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(I18nSupportedLocales::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(I18nSupportedLocales::Locale)
                        .string_len(35)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(I18nSupportedLocales::Label)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(I18nSupportedLocales::IsEnabled)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(I18nSupportedLocales::SortOrder)
                        .integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(I18nSupportedLocales::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(I18nSupportedLocales::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("uq_i18n_supported_locales_locale")
                .table(I18nSupportedLocales::Table)
                .col(I18nSupportedLocales::Locale)
                .unique()
                .to_owned(),
        )
        .await?;

        // ── i18n_entries ─────────────────────────────────────────────────────
        m.create_table(
            Table::create()
                .table(I18nEntries::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(I18nEntries::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(I18nEntries::Namespace)
                        .string_len(64)
                        .not_null(),
                )
                .col(ColumnDef::new(I18nEntries::Key).string_len(256).not_null())
                .col(ColumnDef::new(I18nEntries::Description).text().null())
                .col(
                    ColumnDef::new(I18nEntries::Status)
                        .string_len(16)
                        .not_null()
                        .default("active"),
                )
                .col(
                    ColumnDef::new(I18nEntries::LastSeenAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(I18nEntries::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(I18nEntries::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("uq_i18n_entries_namespace_key")
                .table(I18nEntries::Table)
                .col(I18nEntries::Namespace)
                .col(I18nEntries::Key)
                .unique()
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_i18n_entries_namespace")
                .table(I18nEntries::Table)
                .col(I18nEntries::Namespace)
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_i18n_entries_status")
                .table(I18nEntries::Table)
                .col(I18nEntries::Status)
                .to_owned(),
        )
        .await?;

        // ── i18n_entry_locations ─────────────────────────────────────────────
        m.create_table(
            Table::create()
                .table(I18nEntryLocations::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(I18nEntryLocations::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(I18nEntryLocations::EntryId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(I18nEntryLocations::FilePath)
                        .string_len(512)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(I18nEntryLocations::Line)
                        .integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(I18nEntryLocations::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_i18n_entry_locations_entry")
                        .from(I18nEntryLocations::Table, I18nEntryLocations::EntryId)
                        .to(I18nEntries::Table, I18nEntries::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("uq_i18n_locations_entry_file_line")
                .table(I18nEntryLocations::Table)
                .col(I18nEntryLocations::EntryId)
                .col(I18nEntryLocations::FilePath)
                .col(I18nEntryLocations::Line)
                .unique()
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_i18n_locations_entry_id")
                .table(I18nEntryLocations::Table)
                .col(I18nEntryLocations::EntryId)
                .to_owned(),
        )
        .await?;

        // ── i18n_translations ────────────────────────────────────────────────
        m.create_table(
            Table::create()
                .table(I18nTranslations::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(I18nTranslations::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(I18nTranslations::Namespace)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(I18nTranslations::Key)
                        .string_len(256)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(I18nTranslations::Locale)
                        .string_len(35)
                        .not_null(),
                )
                .col(ColumnDef::new(I18nTranslations::Value).text().not_null())
                .col(
                    ColumnDef::new(I18nTranslations::Scope)
                        .string_len(16)
                        .not_null(),
                )
                .col(ColumnDef::new(I18nTranslations::TenantId).uuid().null())
                .col(ColumnDef::new(I18nTranslations::UpdatedBy).uuid().null())
                .col(
                    ColumnDef::new(I18nTranslations::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(I18nTranslations::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // UNIQUE (namespace, key, locale, tenant_id). NULL != NULL caveat applies;
        // service layer guards global (NULL tenant) duplicates.
        m.create_index(
            Index::create()
                .name("uq_i18n_translations_ns_key_locale_tenant")
                .table(I18nTranslations::Table)
                .col(I18nTranslations::Namespace)
                .col(I18nTranslations::Key)
                .col(I18nTranslations::Locale)
                .col(I18nTranslations::TenantId)
                .unique()
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_i18n_translations_ns_locale_scope")
                .table(I18nTranslations::Table)
                .col(I18nTranslations::Namespace)
                .col(I18nTranslations::Locale)
                .col(I18nTranslations::Scope)
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_i18n_translations_tenant_id")
                .table(I18nTranslations::Table)
                .col(I18nTranslations::TenantId)
                .to_owned(),
        )
        .await?;

        // ── i18n_bundle_revisions ────────────────────────────────────────────
        m.create_table(
            Table::create()
                .table(I18nBundleRevisions::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(I18nBundleRevisions::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(I18nBundleRevisions::Locale)
                        .string_len(35)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(I18nBundleRevisions::Namespace)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(I18nBundleRevisions::Scope)
                        .string_len(16)
                        .not_null(),
                )
                .col(ColumnDef::new(I18nBundleRevisions::TenantId).uuid().null())
                .col(
                    ColumnDef::new(I18nBundleRevisions::Revision)
                        .big_integer()
                        .not_null()
                        .default(1),
                )
                .col(
                    ColumnDef::new(I18nBundleRevisions::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // UNIQUE (locale, namespace, scope, tenant_id). NULL caveat applies.
        m.create_index(
            Index::create()
                .name("uq_i18n_bundle_revisions_locale_ns_scope_tenant")
                .table(I18nBundleRevisions::Table)
                .col(I18nBundleRevisions::Locale)
                .col(I18nBundleRevisions::Namespace)
                .col(I18nBundleRevisions::Scope)
                .col(I18nBundleRevisions::TenantId)
                .unique()
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_i18n_bundle_revisions_lookup")
                .table(I18nBundleRevisions::Table)
                .col(I18nBundleRevisions::Locale)
                .col(I18nBundleRevisions::Namespace)
                .col(I18nBundleRevisions::Scope)
                .col(I18nBundleRevisions::TenantId)
                .to_owned(),
        )
        .await?;

        // ── seeds ────────────────────────────────────────────────────────────
        seed_supported_locales(m).await?;
        seed_translations_and_revisions(m).await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(I18nBundleRevisions::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(I18nTranslations::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(I18nEntryLocations::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(I18nEntries::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(I18nSupportedLocales::Table).to_owned())
            .await?;
        Ok(())
    }
}

// ── seed helpers ─────────────────────────────────────────────────────────────

/// Application-layer `UUIDv7` generation, matching `crate::utils::id::generate_id`
/// in the main crate. Migration crate cannot depend on the app crate, so we
/// inline `Uuid::now_v7()` here.
fn new_id() -> Uuid {
    Uuid::now_v7()
}

async fn seed_supported_locales(m: &SchemaManager<'_>) -> Result<(), DbErr> {
    let conn = m.get_connection();
    let backend = conn.get_database_backend();

    let rows: &[(&str, &str, bool, i32)] = &[
        ("zh-CN", "简体中文", true, 0),
        ("en-US", "English", true, 1),
    ];

    for (locale, label, enabled, sort) in rows {
        conn.execute(Statement::from_sql_and_values(
            backend,
            "INSERT INTO i18n_supported_locales (id, locale, label, is_enabled, sort_order) \
             VALUES ($1, $2, $3, $4, $5)",
            [
                new_id().into(),
                (*locale).into(),
                (*label).into(),
                (*enabled).into(),
                (*sort).into(),
            ],
        ))
        .await?;
    }

    Ok(())
}

async fn seed_translations_and_revisions(m: &SchemaManager<'_>) -> Result<(), DbErr> {
    let conn = m.get_connection();
    let backend = conn.get_database_backend();

    // (namespace, key, locale, value)
    let translations: &[(&str, &str, &str, &str)] = &[
        // Common
        ("Common", "save", "zh-CN", "保存"),
        ("Common", "save", "en-US", "Save"),
        ("Common", "cancel", "zh-CN", "取消"),
        ("Common", "cancel", "en-US", "Cancel"),
        ("Common", "delete", "zh-CN", "删除"),
        ("Common", "delete", "en-US", "Delete"),
        ("Common", "confirm", "zh-CN", "确定"),
        ("Common", "confirm", "en-US", "Confirm"),
        // CommonError (aligned with backend CommonError enum)
        (
            "CommonError",
            "auth.invalid_credentials",
            "zh-CN",
            "用户名或密码错误",
        ),
        (
            "CommonError",
            "auth.invalid_credentials",
            "en-US",
            "Invalid credentials",
        ),
        (
            "CommonError",
            "auth.account_locked",
            "zh-CN",
            "账户已被锁定",
        ),
        (
            "CommonError",
            "auth.account_locked",
            "en-US",
            "Account locked",
        ),
        ("CommonError", "unknown", "zh-CN", "未知错误"),
        ("CommonError", "unknown", "en-US", "Unknown error"),
    ];

    // Insert i18n_entries (one per (namespace, key))
    use std::collections::BTreeSet;
    let mut entry_keys: BTreeSet<(&str, &str)> = BTreeSet::new();
    for (ns, key, _, _) in translations {
        entry_keys.insert((*ns, *key));
    }
    for (ns, key) in &entry_keys {
        conn.execute(Statement::from_sql_and_values(
            backend,
            "INSERT INTO i18n_entries (id, namespace, key, status) VALUES ($1, $2, $3, 'active')",
            [new_id().into(), (*ns).to_string().into(), (*key).to_string().into()],
        ))
        .await?;
    }

    // Insert i18n_translations (all global, scope='global', tenant_id=NULL).
    for (ns, key, locale, value) in translations {
        conn.execute(Statement::from_sql_and_values(
            backend,
            "INSERT INTO i18n_translations \
             (id, namespace, key, locale, value, scope, tenant_id, updated_by) \
             VALUES ($1, $2, $3, $4, $5, 'global', NULL, NULL)",
            [
                new_id().into(),
                (*ns).to_string().into(),
                (*key).to_string().into(),
                (*locale).to_string().into(),
                (*value).to_string().into(),
            ],
        ))
        .await?;
    }

    // Initial bundle revisions: one row per (locale, namespace, scope='global'),
    // revision=1. Tenant rows are created lazily on first tenant override.
    let bundle_rows: &[(&str, &str)] = &[
        ("zh-CN", "Common"),
        ("en-US", "Common"),
        ("zh-CN", "CommonError"),
        ("en-US", "CommonError"),
    ];
    for (locale, ns) in bundle_rows {
        conn.execute(Statement::from_sql_and_values(
            backend,
            "INSERT INTO i18n_bundle_revisions \
             (id, locale, namespace, scope, tenant_id, revision) \
             VALUES ($1, $2, $3, 'global', NULL, 1)",
            [
                new_id().into(),
                (*locale).to_string().into(),
                (*ns).to_string().into(),
            ],
        ))
        .await?;
    }

    Ok(())
}

// ── Iden enums ───────────────────────────────────────────────────────────────

#[derive(Iden)]
enum I18nSupportedLocales {
    Table,
    Id,
    Locale,
    Label,
    IsEnabled,
    SortOrder,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum I18nEntries {
    Table,
    Id,
    Namespace,
    Key,
    Description,
    Status,
    LastSeenAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum I18nEntryLocations {
    Table,
    Id,
    EntryId,
    FilePath,
    Line,
    CreatedAt,
}

#[derive(Iden)]
enum I18nTranslations {
    Table,
    Id,
    Namespace,
    Key,
    Locale,
    Value,
    Scope,
    TenantId,
    UpdatedBy,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum I18nBundleRevisions {
    Table,
    Id,
    Locale,
    Namespace,
    Scope,
    TenantId,
    Revision,
    UpdatedAt,
}
