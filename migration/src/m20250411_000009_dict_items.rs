use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(DictItems::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(DictItems::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(DictItems::TenantId).uuid().null())
                .col(ColumnDef::new(DictItems::DictTypeId).uuid().not_null())
                .col(ColumnDef::new(DictItems::SourceItemId).uuid().null())
                .col(ColumnDef::new(DictItems::Code).string_len(64).not_null())
                .col(ColumnDef::new(DictItems::Name).string_len(128).not_null())
                .col(ColumnDef::new(DictItems::Value).string_len(128).not_null())
                .col(ColumnDef::new(DictItems::ParentId).uuid().null())
                .col(
                    ColumnDef::new(DictItems::SortOrder)
                        .integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(DictItems::Status)
                        .string_len(16)
                        .not_null()
                        .default("active"),
                )
                .col(ColumnDef::new(DictItems::Description).text().null())
                .col(
                    ColumnDef::new(DictItems::Version)
                        .integer()
                        .not_null()
                        .default(1),
                )
                .col(ColumnDef::new(DictItems::UpdatedBy).uuid().null())
                .col(
                    ColumnDef::new(DictItems::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(DictItems::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(DictItems::DeletedAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .to_owned(),
        )
        .await?;

        // 系统字典项在类型内 code 唯一
        m.create_index(
            Index::create()
                .name("uk_dict_items_system_code")
                .table(DictItems::Table)
                .col(DictItems::DictTypeId)
                .col(DictItems::Code)
                .unique()
                .and_where(Expr::col(DictItems::TenantId).is_null())
                .and_where(Expr::col(DictItems::DeletedAt).is_null())
                .to_owned(),
        )
        .await?;

        // 租户自建字典项 (tenant_id, dict_type_id, code) 唯一
        m.create_index(
            Index::create()
                .name("uk_dict_items_tenant_code")
                .table(DictItems::Table)
                .col(DictItems::TenantId)
                .col(DictItems::DictTypeId)
                .col(DictItems::Code)
                .unique()
                .and_where(Expr::col(DictItems::TenantId).is_not_null())
                .and_where(Expr::col(DictItems::SourceItemId).is_null())
                .and_where(Expr::col(DictItems::DeletedAt).is_null())
                .to_owned(),
        )
        .await?;

        // 同一租户对同一系统字典项最多一个覆盖行
        m.create_index(
            Index::create()
                .name("uk_dict_items_tenant_override")
                .table(DictItems::Table)
                .col(DictItems::TenantId)
                .col(DictItems::SourceItemId)
                .unique()
                .and_where(Expr::col(DictItems::SourceItemId).is_not_null())
                .and_where(Expr::col(DictItems::DeletedAt).is_null())
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_dict_items_tenant_id")
                .table(DictItems::Table)
                .col(DictItems::TenantId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_dict_items_dict_type_id")
                .table(DictItems::Table)
                .col(DictItems::DictTypeId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_dict_items_source_item_id")
                .table(DictItems::Table)
                .col(DictItems::SourceItemId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_dict_items_parent_id")
                .table(DictItems::Table)
                .col(DictItems::ParentId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_dict_items_deleted_at")
                .table(DictItems::Table)
                .col(DictItems::DeletedAt)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(DictItems::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum DictItems {
    Table,
    Id,
    TenantId,
    DictTypeId,
    SourceItemId,
    Code,
    Name,
    Value,
    ParentId,
    SortOrder,
    Status,
    Description,
    Version,
    UpdatedBy,
    CreatedAt,
    UpdatedAt,
    DeletedAt,
}
