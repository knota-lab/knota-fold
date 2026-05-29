use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(Files::Table)
                .if_not_exists()
                .col(ColumnDef::new(Files::Id).uuid().not_null().primary_key())
                .col(ColumnDef::new(Files::TenantId).uuid().not_null())
                .col(ColumnDef::new(Files::Name).string_len(512).not_null())
                .col(ColumnDef::new(Files::MimeType).string_len(128).not_null())
                .col(ColumnDef::new(Files::Size).big_integer().not_null())
                .col(
                    ColumnDef::new(Files::ContentHash)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Files::ContentHashAlgo)
                        .string_len(16)
                        .not_null()
                        .default("b3"),
                )
                .col(ColumnDef::new(Files::ContentHashFast).string_len(80).null())
                .col(
                    ColumnDef::new(Files::StorageBackend)
                        .string_len(32)
                        .not_null(),
                )
                .col(ColumnDef::new(Files::Bucket).string_len(128).not_null())
                .col(ColumnDef::new(Files::StorageKey).string_len(512).not_null())
                .col(
                    ColumnDef::new(Files::MultipartUploadId)
                        .string_len(256)
                        .null(),
                )
                .col(ColumnDef::new(Files::Status).string_len(16).not_null())
                .col(ColumnDef::new(Files::StatusReason).string_len(256).null())
                .col(
                    ColumnDef::new(Files::DeletedAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .col(
                    ColumnDef::new(Files::PurgeAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .col(ColumnDef::new(Files::DeletedBy).uuid().null())
                .col(ColumnDef::new(Files::UploadedBy).uuid().not_null())
                .col(
                    ColumnDef::new(Files::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(Files::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(ColumnDef::new(Files::CreatedBy).uuid().not_null())
                .col(ColumnDef::new(Files::UpdatedBy).uuid().not_null())
                .to_owned(),
        )
        .await?;

        // Unique: (tenant_id, content_hash, size) — content-addressed dedup per tenant
        m.create_index(
            Index::create()
                .name("uq_files_tenant_hash_size")
                .table(Files::Table)
                .col(Files::TenantId)
                .col(Files::ContentHash)
                .col(Files::Size)
                .unique()
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_files_tenant_id")
                .table(Files::Table)
                .col(Files::TenantId)
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_files_status")
                .table(Files::Table)
                .col(Files::Status)
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_files_uploaded_by")
                .table(Files::Table)
                .col(Files::UploadedBy)
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_files_created_at")
                .table(Files::Table)
                .col(Files::CreatedAt)
                .to_owned(),
        )
        .await?;

        // Partial index: only rows with content_hash_fast set AND not soft-deleted.
        // SeaORM's Index builder doesn't support WHERE clauses across backends,
        // so we use raw SQL. Both SQLite and PostgreSQL accept this syntax.
        m.get_connection()
            .execute_unprepared(
                r#"CREATE INDEX IF NOT EXISTS idx_files_fast_dedup ON files(tenant_id, content_hash_fast, size) WHERE content_hash_fast IS NOT NULL AND deleted_at IS NULL"#,
            )
            .await?;

        // Partial index for purge sweeper: only soft-deleted rows with purge_at scheduled.
        m.get_connection()
            .execute_unprepared(
                r#"CREATE INDEX IF NOT EXISTS idx_files_purge_at ON files(purge_at) WHERE status = 'DELETED' AND purge_at IS NOT NULL"#,
            )
            .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(Files::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum Files {
    Table,
    Id,
    TenantId,
    Name,
    MimeType,
    Size,
    ContentHash,
    ContentHashAlgo,
    ContentHashFast,
    StorageBackend,
    Bucket,
    StorageKey,
    MultipartUploadId,
    Status,
    StatusReason,
    DeletedAt,
    PurgeAt,
    DeletedBy,
    UploadedBy,
    CreatedAt,
    UpdatedAt,
    CreatedBy,
    UpdatedBy,
}
