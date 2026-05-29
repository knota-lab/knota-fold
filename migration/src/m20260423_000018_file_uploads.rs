use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(FileUploads::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(FileUploads::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(FileUploads::TenantId).uuid().not_null())
                .col(
                    ColumnDef::new(FileUploads::FileName)
                        .string_len(512)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploads::MimeTypeHint)
                        .string_len(128)
                        .null(),
                )
                .col(
                    ColumnDef::new(FileUploads::ExpectedSize)
                        .big_integer()
                        .not_null(),
                )
                // expected_hash is nullable: clients that already know the
                // full content hash (small uploads, or post-probe instant
                // confirms) supply it up-front for verification; large-file
                // streaming uploads omit it and let `complete_upload` adopt
                // the hash computed while streaming as authoritative.
                .col(
                    ColumnDef::new(FileUploads::ExpectedHash)
                        .string_len(128)
                        .null(),
                )
                .col(
                    ColumnDef::new(FileUploads::ExpectedHashAlgo)
                        .string_len(16)
                        .not_null()
                        .default("b3"),
                )
                .col(
                    ColumnDef::new(FileUploads::PartSize)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(FileUploads::PartsTotal).integer().not_null())
                .col(
                    ColumnDef::new(FileUploads::PartsReceived)
                        .integer()
                        .not_null()
                        .default(0i32),
                )
                .col(
                    ColumnDef::new(FileUploads::StorageBackend)
                        .string_len(32)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploads::Bucket)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploads::TempKey)
                        .string_len(512)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploads::S3UploadId)
                        .string_len(256)
                        .null(),
                )
                .col(
                    ColumnDef::new(FileUploads::Status)
                        .string_len(32)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploads::StatusReason)
                        .string_len(256)
                        .null(),
                )
                .col(
                    ColumnDef::new(FileUploads::ExpiresAt)
                        .timestamp_with_time_zone()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploads::ExpiredAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .col(ColumnDef::new(FileUploads::CompletedFileId).uuid().null())
                .col(
                    ColumnDef::new(FileUploads::ExpectedHashFast)
                        .string_len(80)
                        .null(),
                )
                .col(
                    ColumnDef::new(FileUploads::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(FileUploads::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(ColumnDef::new(FileUploads::CreatedBy).uuid().not_null())
                .col(ColumnDef::new(FileUploads::UpdatedBy).uuid().not_null())
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_file_uploads_tenant_id")
                .table(FileUploads::Table)
                .col(FileUploads::TenantId)
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_file_uploads_status")
                .table(FileUploads::Table)
                .col(FileUploads::Status)
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_file_uploads_expires_at")
                .table(FileUploads::Table)
                .col(FileUploads::ExpiresAt)
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_file_uploads_status_expired_at")
                .table(FileUploads::Table)
                .col(FileUploads::Status)
                .col(FileUploads::ExpiredAt)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(FileUploads::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum FileUploads {
    Table,
    Id,
    TenantId,
    FileName,
    MimeTypeHint,
    ExpectedSize,
    ExpectedHash,
    ExpectedHashAlgo,
    PartSize,
    PartsTotal,
    PartsReceived,
    StorageBackend,
    Bucket,
    TempKey,
    S3UploadId,
    Status,
    StatusReason,
    ExpiresAt,
    ExpiredAt,
    CompletedFileId,
    ExpectedHashFast,
    CreatedAt,
    UpdatedAt,
    CreatedBy,
    UpdatedBy,
}
