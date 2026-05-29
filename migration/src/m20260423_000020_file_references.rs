use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(FileReferences::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(FileReferences::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(FileReferences::TenantId).uuid().not_null())
                .col(ColumnDef::new(FileReferences::FileId).uuid().not_null())
                // resource_type uses domain-qualified naming (e.g. "crm:contract",
                // "iam:user_avatar"). Decoupled from physical table names so that
                // schema/table refactors do not invalidate stored references.
                // Application layer maintains a registry of valid values; DB enforces
                // length only.
                .col(
                    ColumnDef::new(FileReferences::ResourceType)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileReferences::ResourceId)
                        .string_len(128)
                        .not_null(),
                )
                // field_name MUST default to '' (empty string), NOT NULL.
                // PostgreSQL treats NULL != NULL in unique indexes, which would silently
                // break the unique constraint when field_name is omitted. Empty string
                // means "the resource has a single attachment slot".
                .col(
                    ColumnDef::new(FileReferences::FieldName)
                        .string_len(64)
                        .not_null()
                        .default(""),
                )
                .col(ColumnDef::new(FileReferences::CreatedBy).uuid().not_null())
                // display_name is the per-reference business display name.
                // NULL means "fall back to files.original_name". This lets
                // multiple business rows reference the same canonical
                // files row (e.g. after instant-upload dedup) while each
                // keeping its own user-facing label.
                .col(
                    ColumnDef::new(FileReferences::DisplayName)
                        .string_len(512)
                        .null(),
                )
                .col(
                    ColumnDef::new(FileReferences::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                // deleted_at = NULL means the reference is active. When a business
                // resource is soft-deleted (or a user explicitly detaches a file),
                // we stamp deleted_at instead of removing the row, preserving the
                // audit trail. The active-only partial unique index below ensures
                // a (tenant, file, resource, field) tuple can be re-attached after
                // a previous detach without primary-key collision.
                .col(
                    ColumnDef::new(FileReferences::DeletedAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .to_owned(),
        )
        .await?;

        // Partial unique index: at most ONE active reference per
        // (tenant, file, resource, field) tuple. Soft-deleted rows are
        // excluded so a re-attach after detach succeeds without conflict.
        // Implemented via raw SQL because sea-query's IndexCreateStatement
        // does not yet expose Postgres partial-index `WHERE` clauses.
        let backend = m.get_database_backend();
        m.get_connection()
            .execute_unprepared(match backend {
                sea_orm::DatabaseBackend::Postgres | sea_orm::DatabaseBackend::Sqlite => {
                    "CREATE UNIQUE INDEX IF NOT EXISTS uq_file_refs_active \
                     ON file_references (tenant_id, file_id, resource_type, resource_id, field_name) \
                     WHERE deleted_at IS NULL"
                }
                sea_orm::DatabaseBackend::MySql => {
                    // MySQL has no partial indexes; fall back to a non-unique
                    // index and rely on application-layer enforcement. We do
                    // not target MySQL but keep the branch compilable.
                    "CREATE INDEX IF NOT EXISTS uq_file_refs_active \
                     ON file_references (tenant_id, file_id, resource_type, resource_id, field_name)"
                }
            })
            .await?;

        // Hot-path index: "is this file still referenced by anyone?"
        // Used by purge_files (NOT EXISTS guard) and the "X 处使用" UI badge.
        // Partial on deleted_at IS NULL keeps it tight and skip-scan friendly.
        m.get_connection()
            .execute_unprepared(match backend {
                sea_orm::DatabaseBackend::Postgres | sea_orm::DatabaseBackend::Sqlite => {
                    "CREATE INDEX IF NOT EXISTS idx_file_refs_active_file \
                     ON file_references (file_id) WHERE deleted_at IS NULL"
                }
                sea_orm::DatabaseBackend::MySql => {
                    "CREATE INDEX IF NOT EXISTS idx_file_refs_active_file \
                     ON file_references (file_id)"
                }
            })
            .await?;

        // Resource-side lookup: "what files are attached to this business row?"
        // Active-only because detached references are audit data, not display data.
        m.get_connection()
            .execute_unprepared(match backend {
                sea_orm::DatabaseBackend::Postgres | sea_orm::DatabaseBackend::Sqlite => {
                    "CREATE INDEX IF NOT EXISTS idx_file_refs_active_resource \
                     ON file_references (tenant_id, resource_type, resource_id) WHERE deleted_at IS NULL"
                }
                sea_orm::DatabaseBackend::MySql => {
                    "CREATE INDEX IF NOT EXISTS idx_file_refs_active_resource \
                     ON file_references (tenant_id, resource_type, resource_id)"
                }
            })
            .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(FileReferences::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum FileReferences {
    Table,
    Id,
    TenantId,
    FileId,
    ResourceType,
    ResourceId,
    FieldName,
    DisplayName,
    CreatedBy,
    CreatedAt,
    DeletedAt,
}
