use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(SysConfigs::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(SysConfigs::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(SysConfigs::Key).string_len(128).not_null())
                .col(ColumnDef::new(SysConfigs::Value).text().not_null())
                .col(
                    ColumnDef::new(SysConfigs::ValueType)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(SysConfigs::Category)
                        .string_len(64)
                        .not_null(),
                )
                .col(ColumnDef::new(SysConfigs::Scope).string_len(16).not_null())
                .col(ColumnDef::new(SysConfigs::TenantId).uuid().null())
                .col(ColumnDef::new(SysConfigs::Label).string_len(128).not_null())
                .col(ColumnDef::new(SysConfigs::Description).text().null())
                .col(ColumnDef::new(SysConfigs::UpdatedBy).uuid().null())
                .col(
                    ColumnDef::new(SysConfigs::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(SysConfigs::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // Unique constraint: (key, tenant_id) — NULL != NULL in both PG and SQLite
        m.create_index(
            Index::create()
                .name("uq_sys_configs_key_tenant")
                .table(SysConfigs::Table)
                .col(SysConfigs::Key)
                .col(SysConfigs::TenantId)
                .unique()
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_sys_configs_key")
                .table(SysConfigs::Table)
                .col(SysConfigs::Key)
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_sys_configs_tenant_id")
                .table(SysConfigs::Table)
                .col(SysConfigs::TenantId)
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_sys_configs_category")
                .table(SysConfigs::Table)
                .col(SysConfigs::Category)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(SysConfigs::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum SysConfigs {
    Table,
    Id,
    Key,
    Value,
    ValueType,
    Category,
    Scope,
    TenantId,
    Label,
    Description,
    UpdatedBy,
    CreatedAt,
    UpdatedAt,
}
