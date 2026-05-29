use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(FileInstantIdempotency::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(FileInstantIdempotency::TenantId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileInstantIdempotency::UserId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileInstantIdempotency::ExpectedHash)
                        .text()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileInstantIdempotency::IdempotencyKey)
                        .text()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileInstantIdempotency::ResponseBody)
                        .blob()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileInstantIdempotency::StatusCode)
                        .integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileInstantIdempotency::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .primary_key(
                    Index::create()
                        .col(FileInstantIdempotency::TenantId)
                        .col(FileInstantIdempotency::UserId)
                        .col(FileInstantIdempotency::ExpectedHash)
                        .col(FileInstantIdempotency::IdempotencyKey),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_file_instant_idempotency_created_at")
                .table(FileInstantIdempotency::Table)
                .col(FileInstantIdempotency::CreatedAt)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(
            Table::drop()
                .table(FileInstantIdempotency::Table)
                .to_owned(),
        )
        .await
    }
}

#[derive(Iden)]
enum FileInstantIdempotency {
    Table,
    TenantId,
    UserId,
    ExpectedHash,
    IdempotencyKey,
    ResponseBody,
    StatusCode,
    CreatedAt,
}
