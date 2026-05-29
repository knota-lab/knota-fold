use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(FileUploadIdempotency::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(FileUploadIdempotency::UploadId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploadIdempotency::Endpoint)
                        .text()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploadIdempotency::IdempotencyKey)
                        .text()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploadIdempotency::ResponseBody)
                        .blob()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploadIdempotency::StatusCode)
                        .integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploadIdempotency::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .primary_key(
                    Index::create()
                        .col(FileUploadIdempotency::UploadId)
                        .col(FileUploadIdempotency::Endpoint)
                        .col(FileUploadIdempotency::IdempotencyKey),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_file_upload_idempotency_upload_id")
                .table(FileUploadIdempotency::Table)
                .col(FileUploadIdempotency::UploadId)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(FileUploadIdempotency::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum FileUploadIdempotency {
    Table,
    UploadId,
    Endpoint,
    IdempotencyKey,
    ResponseBody,
    StatusCode,
    CreatedAt,
}
