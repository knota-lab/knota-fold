use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(Roles::Table)
                .if_not_exists()
                .col(ColumnDef::new(Roles::Id).uuid().not_null().primary_key())
                .col(ColumnDef::new(Roles::TenantId).uuid().not_null())
                .col(ColumnDef::new(Roles::Name).string_len(64).not_null())
                .col(ColumnDef::new(Roles::Code).string_len(64).not_null())
                .col(ColumnDef::new(Roles::ParentId).uuid().null())
                .col(
                    ColumnDef::new(Roles::IsSystem)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(ColumnDef::new(Roles::Description).text().null())
                .col(
                    ColumnDef::new(Roles::Version)
                        .integer()
                        .not_null()
                        .default(1),
                )
                .col(ColumnDef::new(Roles::UpdatedBy).uuid().null())
                .col(
                    ColumnDef::new(Roles::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(Roles::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(Roles::Status)
                        .string_len(16)
                        .not_null()
                        .default("active"),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("uk_roles_tenant_code")
                .table(Roles::Table)
                .col(Roles::TenantId)
                .col(Roles::Code)
                .unique()
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_roles_tenant_id")
                .table(Roles::Table)
                .col(Roles::TenantId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_roles_parent_id")
                .table(Roles::Table)
                .col(Roles::ParentId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_roles_status")
                .table(Roles::Table)
                .col(Roles::Status)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(Roles::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum Roles {
    Table,
    Id,
    TenantId,
    Name,
    Code,
    ParentId,
    IsSystem,
    Description,
    Version,
    UpdatedBy,
    CreatedAt,
    UpdatedAt,
    Status,
}
