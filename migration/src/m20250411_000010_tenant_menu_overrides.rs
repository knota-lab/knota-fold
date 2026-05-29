use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(TenantMenuOverrides::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(TenantMenuOverrides::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(TenantMenuOverrides::TenantId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(TenantMenuOverrides::SysMenuId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(TenantMenuOverrides::CustomName)
                        .string_len(64)
                        .null(),
                )
                .col(
                    ColumnDef::new(TenantMenuOverrides::CustomIcon)
                        .string_len(128)
                        .null(),
                )
                .col(
                    ColumnDef::new(TenantMenuOverrides::CustomSort)
                        .integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(TenantMenuOverrides::IsHidden)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(TenantMenuOverrides::Version)
                        .integer()
                        .not_null()
                        .default(1),
                )
                .col(ColumnDef::new(TenantMenuOverrides::UpdatedBy).uuid().null())
                .col(
                    ColumnDef::new(TenantMenuOverrides::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(TenantMenuOverrides::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("uk_overrides_tenant_menu")
                .table(TenantMenuOverrides::Table)
                .col(TenantMenuOverrides::TenantId)
                .col(TenantMenuOverrides::SysMenuId)
                .unique()
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(TenantMenuOverrides::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum TenantMenuOverrides {
    Table,
    Id,
    TenantId,
    SysMenuId,
    CustomName,
    CustomIcon,
    CustomSort,
    IsHidden,
    Version,
    UpdatedBy,
    CreatedAt,
    UpdatedAt,
}
