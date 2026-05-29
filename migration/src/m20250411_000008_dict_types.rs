use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(DictTypes::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(DictTypes::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(DictTypes::TenantId).uuid().null())
                .col(ColumnDef::new(DictTypes::SourceTypeId).uuid().null())
                .col(ColumnDef::new(DictTypes::Code).string_len(64).not_null())
                .col(ColumnDef::new(DictTypes::Name).string_len(128).not_null())
                .col(
                    ColumnDef::new(DictTypes::Status)
                        .string_len(16)
                        .not_null()
                        .default("active"),
                )
                .col(ColumnDef::new(DictTypes::Description).text().null())
                .col(
                    ColumnDef::new(DictTypes::Version)
                        .integer()
                        .not_null()
                        .default(1),
                )
                .col(ColumnDef::new(DictTypes::UpdatedBy).uuid().null())
                .col(
                    ColumnDef::new(DictTypes::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(DictTypes::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(DictTypes::DeletedAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .to_owned(),
        )
        .await?;

        // 系统字典 code 唯一 (tenant_id IS NULL)
        m.create_index(
            Index::create()
                .name("uk_dict_types_system_code")
                .table(DictTypes::Table)
                .col(DictTypes::Code)
                .unique()
                .and_where(Expr::col(DictTypes::TenantId).is_null())
                .and_where(Expr::col(DictTypes::DeletedAt).is_null())
                .to_owned(),
        )
        .await?;

        // 租户自建字典 (tenant_id, code) 唯一
        m.create_index(
            Index::create()
                .name("uk_dict_types_tenant_code")
                .table(DictTypes::Table)
                .col(DictTypes::TenantId)
                .col(DictTypes::Code)
                .unique()
                .and_where(Expr::col(DictTypes::TenantId).is_not_null())
                .and_where(Expr::col(DictTypes::SourceTypeId).is_null())
                .and_where(Expr::col(DictTypes::DeletedAt).is_null())
                .to_owned(),
        )
        .await?;

        // 同一租户对同一系统字典类型最多一个覆盖行
        m.create_index(
            Index::create()
                .name("uk_dict_types_tenant_override")
                .table(DictTypes::Table)
                .col(DictTypes::TenantId)
                .col(DictTypes::SourceTypeId)
                .unique()
                .and_where(Expr::col(DictTypes::SourceTypeId).is_not_null())
                .and_where(Expr::col(DictTypes::DeletedAt).is_null())
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_dict_types_tenant_id")
                .table(DictTypes::Table)
                .col(DictTypes::TenantId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_dict_types_source_type_id")
                .table(DictTypes::Table)
                .col(DictTypes::SourceTypeId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_dict_types_deleted_at")
                .table(DictTypes::Table)
                .col(DictTypes::DeletedAt)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(DictTypes::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum DictTypes {
    Table,
    Id,
    TenantId,
    SourceTypeId,
    Code,
    Name,
    Status,
    Description,
    Version,
    UpdatedBy,
    CreatedAt,
    UpdatedAt,
    DeletedAt,
}
