use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(FileUploadParts::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(FileUploadParts::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(FileUploadParts::UploadId).uuid().not_null())
                .col(
                    ColumnDef::new(FileUploadParts::PartNumber)
                        .integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploadParts::Etag)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploadParts::Size)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FileUploadParts::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // Unique: one row per (upload, part_number)
        m.create_index(
            Index::create()
                .name("uq_file_upload_parts_upload_part")
                .table(FileUploadParts::Table)
                .col(FileUploadParts::UploadId)
                .col(FileUploadParts::PartNumber)
                .unique()
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_file_upload_parts_upload_id")
                .table(FileUploadParts::Table)
                .col(FileUploadParts::UploadId)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(FileUploadParts::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum FileUploadParts {
    Table,
    Id,
    UploadId,
    PartNumber,
    Etag,
    Size,
    CreatedAt,
}
